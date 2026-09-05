use crate::protocol::{self, ACK, Assembler, DATA, INFO, Message, RX, SELECT, TX};
use anyhow::{Result, bail, ensure};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use uuid::Uuid;

struct Session {
    client: String,
    exchange: u32,
    deadline: Instant,
    incoming: Assembler,
    peer: Option<Message>,
    reply: Vec<Vec<u8>>,
    selected: Option<usize>,
    read: Vec<bool>,
    committed: bool,
}

pub struct State {
    pub own: Message,
    enabled: bool,
    session: Option<Session>,
    recent: HashMap<Uuid, Instant>,
    timeout: Duration,
    cooldown: Duration,
}

impl State {
    pub fn new(own: Message, timeout: Duration, cooldown: Duration) -> Self {
        Self {
            own,
            enabled: false,
            session: None,
            recent: HashMap::new(),
            timeout,
            cooldown,
        }
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn shutdown(&mut self) {
        self.enabled = false;
        self.session = None;
    }

    pub fn expire(&mut self, now: Instant) {
        self.recent.retain(|_, until| *until > now);
        if self.session.as_ref().is_some_and(|s| now >= s.deadline) {
            if !self.session.as_ref().unwrap().committed {
                eprintln!("交換失敗: 待受側の交換がタイムアウトしました");
            }
            self.session = None;
        }
    }

    /// The role timer and request handler use the same mutex: a request either
    /// acquires a lease first, or is rejected after this transition.
    pub fn disable_if_idle(&mut self, now: Instant) -> bool {
        self.expire(now);
        if self.session.is_some() {
            return false;
        }
        self.enabled = false;
        true
    }

    pub fn cooling_down(&self, node: Uuid, now: Instant) -> bool {
        self.recent.get(&node).is_some_and(|until| now < *until)
    }

    pub fn record(&mut self, peer: &Message, now: Instant) {
        self.recent.insert(peer.node, now + self.cooldown);
        println!(
            "交換成功 peer={} 送信={:?} 受信={:?}",
            peer.node, self.own.text, peer.text
        );
    }

    pub fn read(&mut self, client: &str, characteristic: Uuid, now: Instant) -> Result<Vec<u8>> {
        self.expire(now);
        ensure!(self.enabled, "not in peripheral role");
        if characteristic == INFO {
            return Ok(protocol::identity(self.own.node));
        }
        ensure!(characteristic == TX, "unknown characteristic");
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no exchange"))?;
        ensure!(session.client == client, "busy with another client");
        let selected = session
            .selected
            .ok_or_else(|| anyhow::anyhow!("select a fragment first"))?;
        session.read[selected] = true;
        Ok(session.reply[selected].clone())
    }

    pub fn write(
        &mut self,
        client: &str,
        characteristic: Uuid,
        value: &[u8],
        now: Instant,
    ) -> Result<()> {
        self.expire(now);
        ensure!(self.enabled && characteristic == RX, "write not available");
        let (kind, exchange) = protocol::header(value)?;
        if self.session.is_none() {
            ensure!(
                kind == DATA && value.len() >= 9 && value[6] == 0,
                "start with data fragment zero"
            );
            // Validate before reserving the lease, so malformed starts don't block it.
            let mut incoming = Assembler::default();
            incoming.push(value)?;
            self.session = Some(Session {
                client: client.into(),
                exchange,
                deadline: now + self.timeout,
                incoming,
                peer: None,
                reply: vec![],
                selected: None,
                read: vec![],
                committed: false,
            });
            println!("交換開始 role=peripheral exchange={exchange:08x}");
            return Ok(());
        }
        let session = self.session.as_mut().unwrap();
        ensure!(
            session.client == client && session.exchange == exchange,
            "busy with another exchange"
        );
        match kind {
            DATA => {
                ensure!(session.peer.is_none(), "message already received");
                if let Some(peer) = session.incoming.push(value)? {
                    ensure!(peer.node != self.own.node, "self exchange");
                    ensure!(
                        !self
                            .recent
                            .get(&peer.node)
                            .is_some_and(|until| now < *until),
                        "peer is cooling down"
                    );
                    session.reply = self.own.frames(exchange)?;
                    session.read = vec![false; session.reply.len()];
                    session.peer = Some(peer);
                }
            }
            SELECT => {
                ensure!(value.len() == 7 && !session.committed, "invalid select");
                let index = value[6] as usize;
                ensure!(
                    index < session.reply.len(),
                    "reply unavailable/index out of range"
                );
                session.selected = Some(index);
            }
            ACK => {
                ensure!(
                    value.len() == 6
                        && session.peer.is_some()
                        && session.read.iter().all(|read| *read),
                    "premature acknowledgement"
                );
                if !session.committed {
                    session.committed = true;
                    // Keep the service available while the ATT write response is delivered.
                    session.deadline = now + Duration::from_secs(1);
                    let peer = session.peer.clone().unwrap();
                    self.record(&peer, now);
                }
            }
            _ => bail!("unknown command"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (State, Message, Instant) {
        let mut state = State::new(
            Message {
                node: Uuid::new_v4(),
                text: "待受です".into(),
            },
            Duration::from_secs(10),
            Duration::from_secs(30),
        );
        state.enable();
        (
            state,
            Message {
                node: Uuid::new_v4(),
                text: "接続です".into(),
            },
            Instant::now(),
        )
    }

    #[test]
    fn complete_exchange_and_cooldown() {
        let (mut state, peer, now) = fixture();
        for frame in peer.frames(9).unwrap() {
            state.write("client", RX, &frame, now).unwrap();
        }
        assert!(
            state
                .write("client", RX, &protocol::command(ACK, 9), now)
                .is_err()
        );
        let mut incoming = Assembler::default();
        let mut result = None;
        for index in 0..state.own.frames(9).unwrap().len() {
            let mut select = protocol::command(SELECT, 9);
            select.push(index as u8);
            state.write("client", RX, &select, now).unwrap();
            let frame = state.read("client", TX, now).unwrap();
            assert_eq!(state.read("client", TX, now).unwrap(), frame);
            result = incoming.push(&frame).unwrap();
        }
        assert_eq!(result, Some(state.own.clone()));
        state
            .write("client", RX, &protocol::command(ACK, 9), now)
            .unwrap();
        // Retrying the final ACK must not extend the cooldown.
        state
            .write(
                "client",
                RX,
                &protocol::command(ACK, 9),
                now + Duration::from_millis(500),
            )
            .unwrap();
        assert!(state.cooling_down(peer.node, now + Duration::from_secs(29)));
        assert!(!state.cooling_down(peer.node, now + Duration::from_secs(30)));
        assert!(state.disable_if_idle(now + Duration::from_secs(1)));
    }

    #[test]
    fn lease_blocks_role_change_and_other_clients_until_timeout() {
        let (mut state, peer, now) = fixture();
        let frames = peer.frames(9).unwrap();
        state.write("a", RX, &frames[0], now).unwrap();
        assert!(!state.disable_if_idle(now + Duration::from_secs(8)));
        assert!(state.write("b", RX, &frames[1], now).is_err());
        assert!(state.disable_if_idle(now + Duration::from_secs(10)));
        assert!(
            state
                .write("a", RX, &frames[1], now + Duration::from_secs(10))
                .is_err()
        );
        state.enable();
        assert!(
            state
                .write("b", RX, &frames[0], now + Duration::from_secs(11))
                .is_ok()
        );
    }

    #[test]
    fn waiter_rejects_recent_peer_and_accepts_after_cooldown() {
        let (mut state, peer, now) = fixture();
        state.record(&peer, now);
        let early = now + Duration::from_secs(2);
        let frames = peer.frames(10).unwrap();
        assert!(
            frames
                .iter()
                .try_for_each(|frame| state.write("a", RX, frame, early))
                .is_err()
        );
        assert!(state.read("a", TX, early).is_err());
        assert!(
            state
                .write("a", RX, &protocol::command(ACK, 10), early)
                .is_err()
        );
        let later = now + Duration::from_secs(30);
        for frame in peer.frames(11).unwrap() {
            state.write("a", RX, &frame, later).unwrap();
        }
        let mut select = protocol::command(SELECT, 11);
        select.push(0);
        state.write("a", RX, &select, later).unwrap();
        assert!(state.read("a", TX, later).is_ok());
    }

    #[test]
    fn shutdown_rejects_stale_requests() {
        let (mut state, peer, now) = fixture();
        let frames = peer.frames(9).unwrap();
        state.write("a", RX, &frames[0], now).unwrap();
        state.shutdown();
        assert!(state.read("a", INFO, now).is_err());
        assert!(state.write("a", RX, &frames[1], now).is_err());
        state.enable();
        assert!(state.write("a", RX, &frames[1], now).is_err());
        assert!(state.write("b", RX, &frames[0], now).is_ok());
    }

    #[test]
    fn role_change_wins_before_first_request() {
        let (mut state, peer, now) = fixture();
        assert!(state.disable_if_idle(now));
        assert!(
            state
                .write("a", RX, &peer.frames(9).unwrap()[0], now)
                .is_err()
        );
    }
}
