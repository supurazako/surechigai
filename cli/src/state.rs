use crate::{
    game::{Deck, Sentence},
    post,
    protocol::{self, ACK, Assembler, DATA, GiftPacket, INFO, Packet, Profile, RX, SELECT, TX},
};
use anyhow::{Result, bail, ensure};
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};
use uuid::Uuid;

struct Session {
    client: String,
    exchange: u32,
    deadline: Instant,
    incoming: Assembler,
    peer: Option<Profile>,
    outgoing_gift: Option<GiftPacket>,
    incoming_gift: Option<GiftPacket>,
    reply: Vec<Vec<u8>>,
    selected: Option<usize>,
    read: Vec<bool>,
    committed: bool,
}

#[derive(Clone, Debug)]
pub struct ExchangeRecord {
    pub sequence: u64,
    pub peer_node: Uuid,
    pub peer_name: String,
    pub sent: Option<crate::game::Phrase>,
    pub received: Option<crate::game::Phrase>,
}

pub struct State {
    node: Uuid,
    name: String,
    deck: Deck,
    sentence: Sentence,
    enabled: bool,
    session: Option<Session>,
    recent: HashMap<Uuid, Instant>,
    exchanges: VecDeque<ExchangeRecord>,
    exchange_count: u64,
    timeout: Duration,
    cooldown: Duration,
    post_url: Option<String>,
    image_status: post::ImageStatusHandle,
    posted_round: Option<Uuid>,
}

impl State {
    pub fn new(
        node: Uuid,
        name: String,
        deck: Deck,
        timeout: Duration,
        cooldown: Duration,
    ) -> Self {
        Self {
            node,
            name,
            deck,
            sentence: Sentence::new(),
            enabled: false,
            session: None,
            recent: HashMap::new(),
            exchanges: VecDeque::new(),
            exchange_count: 0,
            timeout,
            cooldown,
            post_url: None,
            image_status: post::new_image_status_handle(),
            posted_round: None,
        }
    }

    /// 文章完成時にPOSTする広場サーバのURL（`server/`の`POST /submit`）を設定する。
    /// 未設定なら送信しない。
    pub fn set_post_url(&mut self, post_url: Option<String>) {
        self.post_url = post_url;
    }

    /// 直近に完成した文章の画像生成状況（Web Viewer表示用）。
    pub fn image_status(&self) -> Option<post::ImageStatus> {
        self.image_status
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().and_then(|tracked| tracked.status.clone()))
    }

    pub fn node(&self) -> Uuid {
        self.node
    }

    pub fn sentence(&self) -> &Sentence {
        &self.sentence
    }

    pub fn exchanges(&self) -> &VecDeque<ExchangeRecord> {
        &self.exchanges
    }

    pub fn profile(&self) -> Profile {
        Profile {
            node: self.node,
            name: self.name.clone(),
            round: self.sentence.round,
            missing: self.sentence.missing_mask(),
        }
    }

    pub fn choose_gift(&self, peer: &Profile) -> GiftPacket {
        GiftPacket {
            receiver_round: peer.round,
            gift: self.deck.choose_for(peer.missing),
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

    pub fn record_exchange(
        &mut self,
        peer: &Profile,
        sent: &GiftPacket,
        received: &GiftPacket,
        now: Instant,
    ) -> Result<()> {
        ensure!(peer.node != self.node, "self exchange");
        ensure!(
            sent.receiver_round == peer.round,
            "gift targets another round"
        );
        ensure!(
            received.receiver_round == self.sentence.round,
            "received gift targets another round"
        );
        if let Some(phrase) = &sent.gift {
            ensure!(
                peer.missing & phrase.slot.bit() != 0,
                "gift is not requested by peer"
            );
        }
        if let Some(phrase) = received.gift.clone() {
            ensure!(
                self.sentence.accept(peer.node, peer.name.clone(), phrase),
                "received slot is already filled"
            );
        }
        self.exchange_count += 1;
        self.exchanges.push_front(ExchangeRecord {
            sequence: self.exchange_count,
            peer_node: peer.node,
            peer_name: peer.name.clone(),
            sent: sent.gift.clone(),
            received: received.gift.clone(),
        });
        self.exchanges.truncate(50);
        self.recent.insert(peer.node, now + self.cooldown);
        println!(
            "交換成功 peer={}({}) 配布={} 受取={} 作成中={:?} 残り={}",
            peer.name,
            peer.node,
            gift_label(sent),
            gift_label(received),
            self.sentence.render(),
            self.sentence.missing_mask().count_ones()
        );
        if self.sentence.is_complete() {
            let rendered = self.sentence.render();
            println!("文章完成 round={} 文={:?}", self.sentence.round, rendered);
            // 完成後も交換自体は継続するため、同じroundでの二重送信（＝二重課金）を防ぐ。
            if self.posted_round != Some(self.sentence.round) {
                self.posted_round = Some(self.sentence.round);
                if let Some(url) = &self.post_url {
                    post::spawn_post(
                        url.clone(),
                        self.name.clone(),
                        rendered,
                        self.sentence.round,
                        self.image_status.clone(),
                    );
                }
            }
        }
        Ok(())
    }

    pub fn read(&mut self, client: &str, characteristic: Uuid, now: Instant) -> Result<Vec<u8>> {
        self.expire(now);
        ensure!(self.enabled, "not in peripheral role");
        if characteristic == INFO {
            return Ok(protocol::identity(self.node));
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
            let mut incoming = Assembler::default();
            let packet = incoming.push(value)?;
            self.session = Some(Session {
                client: client.into(),
                exchange,
                deadline: now + self.timeout,
                incoming,
                peer: None,
                outgoing_gift: None,
                incoming_gift: None,
                reply: vec![],
                selected: None,
                read: vec![],
                committed: false,
            });
            println!("交換開始 role=peripheral exchange={exchange:08x}");
            if let Some(packet) = packet {
                self.handle_packet(packet, now)?;
            }
            return Ok(());
        }

        let session = self.session.as_ref().unwrap();
        ensure!(
            session.client == client && session.exchange == exchange,
            "busy with another exchange"
        );
        match kind {
            DATA => {
                let receiving_gift = self.session.as_ref().unwrap().peer.is_some();
                if receiving_gift {
                    ensure!(
                        self.session.as_ref().unwrap().read.iter().all(|read| *read),
                        "profile reply has not been read"
                    );
                    ensure!(
                        self.session.as_ref().unwrap().incoming_gift.is_none(),
                        "gift already received"
                    );
                }
                let packet = self.session.as_mut().unwrap().incoming.push(value)?;
                if let Some(packet) = packet {
                    self.handle_packet(packet, now)?;
                }
            }
            SELECT => {
                ensure!(value.len() == 7 && !session.committed, "invalid select");
                let index = value[6] as usize;
                ensure!(
                    index < session.reply.len(),
                    "reply unavailable/index out of range"
                );
                self.session.as_mut().unwrap().selected = Some(index);
            }
            ACK => {
                ensure!(
                    value.len() == 6
                        && session.incoming_gift.is_some()
                        && session.read.iter().all(|read| *read),
                    "premature acknowledgement"
                );
                let commit = if session.committed {
                    None
                } else {
                    Some((
                        session.peer.clone().unwrap(),
                        session.outgoing_gift.clone().unwrap(),
                        session.incoming_gift.clone().unwrap(),
                    ))
                };
                if let Some((peer, sent, received)) = commit {
                    self.record_exchange(&peer, &sent, &received, now)?;
                    let session = self.session.as_mut().unwrap();
                    session.committed = true;
                    // Keep the service available while the ATT write response is delivered.
                    session.deadline = now + Duration::from_secs(1);
                }
            }
            _ => bail!("unknown command"),
        }
        Ok(())
    }

    fn handle_packet(&mut self, packet: Packet, now: Instant) -> Result<()> {
        if self.session.as_ref().unwrap().peer.is_none() {
            let Packet::Profile(peer) = packet else {
                bail!("profile must be sent first")
            };
            ensure!(peer.node != self.node, "self exchange");
            ensure!(!self.cooling_down(peer.node, now), "peer is cooling down");
            let outgoing_gift = self.choose_gift(&peer);
            let reply =
                Packet::Profile(self.profile()).frames(self.session.as_ref().unwrap().exchange)?;
            let session = self.session.as_mut().unwrap();
            session.peer = Some(peer);
            session.outgoing_gift = Some(outgoing_gift);
            session.reply = reply;
            session.read = vec![false; session.reply.len()];
            session.selected = None;
            session.incoming = Assembler::default();
            return Ok(());
        }

        let Packet::Gift(gift) = packet else {
            bail!("expected gift packet")
        };
        ensure!(
            gift.receiver_round == self.sentence.round,
            "gift targets another round"
        );
        if let Some(phrase) = &gift.gift {
            ensure!(
                self.sentence.entry(phrase.slot).is_none(),
                "gift slot is already filled"
            );
        }
        let exchange = self.session.as_ref().unwrap().exchange;
        let outgoing = self
            .session
            .as_ref()
            .unwrap()
            .outgoing_gift
            .clone()
            .unwrap();
        let reply = Packet::Gift(outgoing).frames(exchange)?;
        let session = self.session.as_mut().unwrap();
        session.incoming_gift = Some(gift);
        session.reply = reply;
        session.read = vec![false; session.reply.len()];
        session.selected = None;
        session.incoming = Assembler::default();
        Ok(())
    }
}

fn gift_label(packet: &GiftPacket) -> String {
    packet.gift.as_ref().map_or_else(
        || "なし".into(),
        |phrase| format!("{}:{:?}", phrase.slot.label(), phrase.text),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{ALL_MISSING, Phrase, Slot};

    fn deck(prefix: &str) -> Deck {
        Deck::new([
            format!("{prefix}-when"),
            format!("{prefix}-where"),
            format!("{prefix}-who"),
            format!("{prefix}-what"),
            format!("{prefix}-why"),
            format!("{prefix}-how"),
        ])
        .unwrap()
    }

    fn fixture() -> (State, Profile, Instant) {
        let mut state = State::new(
            Uuid::new_v4(),
            "local-user".into(),
            deck("local"),
            Duration::from_secs(10),
            Duration::from_secs(30),
        );
        state.enable();
        (
            state,
            Profile {
                node: Uuid::new_v4(),
                name: "peer-user".into(),
                round: Uuid::new_v4(),
                missing: ALL_MISSING,
            },
            Instant::now(),
        )
    }

    fn write_packet(state: &mut State, client: &str, exchange: u32, packet: Packet, now: Instant) {
        for frame in packet.frames(exchange).unwrap() {
            state.write(client, RX, &frame, now).unwrap();
        }
    }

    fn read_reply(state: &mut State, client: &str, exchange: u32, now: Instant) -> Packet {
        let count = state.session.as_ref().unwrap().reply.len();
        let mut assembler = Assembler::default();
        let mut result = None;
        for index in 0..count {
            let mut select = protocol::command(SELECT, exchange);
            select.push(index as u8);
            state.write(client, RX, &select, now).unwrap();
            let frame = state.read(client, TX, now).unwrap();
            assert_eq!(state.read(client, TX, now).unwrap(), frame);
            result = assembler.push(&frame).unwrap();
        }
        result.unwrap()
    }

    fn complete_exchange(state: &mut State, peer: &Profile, exchange: u32, now: Instant) {
        write_packet(
            state,
            "client",
            exchange,
            Packet::Profile(peer.clone()),
            now,
        );
        assert!(
            state
                .write("client", RX, &protocol::command(ACK, exchange), now)
                .is_err()
        );
        assert!(matches!(
            read_reply(state, "client", exchange, now),
            Packet::Profile(_)
        ));
        let gift = GiftPacket {
            receiver_round: state.sentence.round,
            gift: Some(Phrase::new(Slot::Who, "peer-who".into()).unwrap()),
        };
        write_packet(state, "client", exchange, Packet::Gift(gift), now);
        let Packet::Gift(reply) = read_reply(state, "client", exchange, now) else {
            panic!("expected gift")
        };
        assert_eq!(reply.receiver_round, peer.round);
        assert!(reply.gift.is_some());
        state
            .write("client", RX, &protocol::command(ACK, exchange), now)
            .unwrap();
    }

    #[test]
    fn complete_symmetric_exchange_and_cooldown() {
        let (mut state, peer, now) = fixture();
        complete_exchange(&mut state, &peer, 9, now);
        assert_eq!(state.sentence.entry(Slot::Who).unwrap().text, "peer-who");
        assert_eq!(state.sentence.entry(Slot::Who).unwrap().source, peer.node);
        assert_eq!(
            state.sentence.entry(Slot::Who).unwrap().source_name,
            "peer-user"
        );
        assert_eq!(state.exchanges.len(), 1);
        assert_eq!(state.exchanges.front().unwrap().peer_name, "peer-user");
        assert_eq!(state.exchanges.front().unwrap().sequence, 1);
        // Retrying the final ACK must not extend the cooldown.
        state
            .write(
                "client",
                RX,
                &protocol::command(ACK, 9),
                now + Duration::from_millis(500),
            )
            .unwrap();
        assert_eq!(state.exchanges.len(), 1);
        assert!(state.cooling_down(peer.node, now + Duration::from_secs(29)));
        assert!(!state.cooling_down(peer.node, now + Duration::from_secs(30)));
        assert!(state.disable_if_idle(now + Duration::from_secs(1)));
    }

    #[test]
    fn both_devices_give_and_build_their_own_sentence() {
        let now = Instant::now();
        let mut a = State::new(
            Uuid::new_v4(),
            "alice".into(),
            deck("a"),
            Duration::from_secs(10),
            Duration::from_secs(30),
        );
        let mut b = State::new(
            Uuid::new_v4(),
            "bob".into(),
            deck("b"),
            Duration::from_secs(10),
            Duration::from_secs(30),
        );
        let a_profile = a.profile();
        let b_profile = b.profile();
        let a_to_b = a.choose_gift(&b_profile);
        let b_to_a = b.choose_gift(&a_profile);

        a.record_exchange(&b_profile, &a_to_b, &b_to_a, now)
            .unwrap();
        b.record_exchange(&a_profile, &b_to_a, &a_to_b, now)
            .unwrap();

        assert_eq!(a.sentence.missing_mask().count_ones(), 5);
        assert_eq!(b.sentence.missing_mask().count_ones(), 5);
        let b_phrase = b_to_a.gift.unwrap();
        let a_phrase = a_to_b.gift.unwrap();
        assert!(
            a.sentence
                .entry(b_phrase.slot)
                .unwrap()
                .text
                .starts_with("b-")
        );
        assert!(
            b.sentence
                .entry(a_phrase.slot)
                .unwrap()
                .text
                .starts_with("a-")
        );
    }

    #[test]
    fn completed_device_keeps_giving_without_receiving() {
        let now = Instant::now();
        let mut completed = State::new(
            Uuid::new_v4(),
            "completed".into(),
            deck("completed"),
            Duration::from_secs(10),
            Duration::from_secs(30),
        );
        for slot in Slot::ALL {
            assert!(completed.sentence.accept(
                Uuid::new_v4(),
                "source".into(),
                Phrase::new(slot, format!("filled-{}", slot.label())).unwrap(),
            ));
        }
        let mut collecting = State::new(
            Uuid::new_v4(),
            "collecting".into(),
            deck("collecting"),
            Duration::from_secs(10),
            Duration::from_secs(30),
        );

        let completed_profile = completed.profile();
        let collecting_profile = collecting.profile();
        assert_eq!(completed_profile.missing, 0);
        let completed_to_collecting = completed.choose_gift(&collecting_profile);
        let collecting_to_completed = collecting.choose_gift(&completed_profile);
        let distributed = completed_to_collecting.gift.clone().unwrap();
        assert!(collecting_to_completed.gift.is_none());

        completed
            .record_exchange(
                &collecting_profile,
                &completed_to_collecting,
                &collecting_to_completed,
                now,
            )
            .unwrap();
        collecting
            .record_exchange(
                &completed_profile,
                &collecting_to_completed,
                &completed_to_collecting,
                now,
            )
            .unwrap();

        assert!(completed.sentence.is_complete());
        assert_eq!(collecting.sentence.missing_mask().count_ones(), 5);
        assert_eq!(
            collecting.sentence.entry(distributed.slot).unwrap().source,
            completed.node
        );
    }

    #[test]
    fn lease_blocks_role_change_and_other_clients_until_timeout() {
        let (mut state, peer, now) = fixture();
        let frames = Packet::Profile(peer).frames(9).unwrap();
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
        state
            .recent
            .insert(peer.node, now + Duration::from_secs(30));
        let frames = Packet::Profile(peer.clone()).frames(10).unwrap();
        let early = now + Duration::from_secs(2);
        assert!(
            frames
                .iter()
                .try_for_each(|frame| state.write("a", RX, frame, early))
                .is_err()
        );
        assert!(state.read("a", TX, early).is_err());
        state.expire(now + Duration::from_secs(12));
        state.enable();
        let later = now + Duration::from_secs(30);
        for frame in Packet::Profile(peer).frames(11).unwrap() {
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
        let frames = Packet::Profile(peer).frames(9).unwrap();
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
                .write("a", RX, &Packet::Profile(peer).frames(9).unwrap()[0], now)
                .is_err()
        );
    }

    #[tokio::test]
    async fn completed_round_posts_once_even_when_exchanges_continue() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
            },
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/submit", listener.local_addr().unwrap());
        let posts = Arc::new(AtomicUsize::new(0));
        let counted = posts.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(4);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut buf = [0; 4096];
                        let n = stream.read(&mut buf).unwrap();
                        let body = if String::from_utf8_lossy(&buf[..n]).starts_with("POST") {
                            counted.fetch_add(1, Ordering::SeqCst);
                            r#"{"id":1,"status":"queued"}"#
                        } else {
                            r#"{"id":1,"status":"done","image":"/image/1.jpg"}"#
                        };
                        write!(
                            stream,
                            "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => panic!("{error}"),
                }
            }
        });
        let (mut state, peer, now) = fixture();
        state.set_post_url(Some(url));
        for slot in Slot::ALL {
            let sent = state.choose_gift(&peer);
            let received = GiftPacket {
                receiver_round: state.sentence.round,
                gift: Some(Phrase::new(slot, "word".into()).unwrap()),
            };
            state.record_exchange(&peer, &sent, &received, now).unwrap();
        }
        for _ in 0..3 {
            let sent = state.choose_gift(&peer);
            let received = GiftPacket {
                receiver_round: state.sentence.round,
                gift: None,
            };
            state
                .record_exchange(&peer, &sent, &received, now + Duration::from_secs(61))
                .unwrap();
        }
        server.join().unwrap();
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        assert_eq!(state.image_status().unwrap().status, "done");
    }
}
