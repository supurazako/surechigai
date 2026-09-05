use anyhow::{Result, bail, ensure};
use uuid::Uuid;

pub const SERVICE: Uuid = Uuid::from_u128(0x478f5400_73ad_47a6_a131_562697033a90);
pub const INFO: Uuid = Uuid::from_u128(0x478f5401_73ad_47a6_a131_562697033a90);
pub const RX: Uuid = Uuid::from_u128(0x478f5402_73ad_47a6_a131_562697033a90);
pub const TX: Uuid = Uuid::from_u128(0x478f5403_73ad_47a6_a131_562697033a90);
pub const VERSION: u8 = 1;
pub const DATA: u8 = 1;
pub const SELECT: u8 = 2;
pub const ACK: u8 = 3;
const CHUNK: usize = 12;
const MAX_ENVELOPE: usize = 16 + 2 + 128;
const MAX_FRAMES: usize = MAX_ENVELOPE.div_ceil(CHUNK);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub node: Uuid,
    pub text: String,
}

impl Message {
    pub fn frames(&self, exchange: u32) -> Result<Vec<Vec<u8>>> {
        ensure!(self.text.len() <= 128, "message too large");
        let mut bytes = self.node.as_bytes().to_vec();
        bytes.extend_from_slice(&(self.text.len() as u16).to_le_bytes());
        bytes.extend_from_slice(self.text.as_bytes());
        let count = bytes.len().div_ceil(CHUNK) as u8;
        Ok(bytes
            .chunks(CHUNK)
            .enumerate()
            .map(|(i, chunk)| {
                let mut frame = command(DATA, exchange);
                frame.extend_from_slice(&[i as u8, count]);
                frame.extend_from_slice(chunk);
                frame
            })
            .collect())
    }
}

pub fn command(kind: u8, exchange: u32) -> Vec<u8> {
    let mut bytes = vec![VERSION, kind];
    bytes.extend_from_slice(&exchange.to_le_bytes());
    bytes
}

pub fn header(bytes: &[u8]) -> Result<(u8, u32)> {
    ensure!((6..=20).contains(&bytes.len()), "invalid frame length");
    ensure!(bytes[0] == VERSION, "unsupported protocol version");
    Ok((bytes[1], u32::from_le_bytes(bytes[2..6].try_into()?)))
}

pub fn identity(node: Uuid) -> Vec<u8> {
    let mut value = vec![VERSION];
    value.extend_from_slice(node.as_bytes());
    value
}

pub fn parse_identity(bytes: &[u8]) -> Result<Uuid> {
    ensure!(
        bytes.len() == 17 && bytes[0] == VERSION,
        "invalid peer identity/version"
    );
    Ok(Uuid::from_slice(&bytes[1..])?)
}

#[derive(Debug, Default)]
pub struct Assembler {
    exchange: Option<u32>,
    count: u8,
    next: u8,
    bytes: Vec<u8>,
}

impl Assembler {
    pub fn push(&mut self, frame: &[u8]) -> Result<Option<Message>> {
        let (kind, exchange) = header(frame)?;
        ensure!(kind == DATA && frame.len() >= 9, "expected data frame");
        let (seq, count) = (frame[6], frame[7]);
        ensure!(
            (2..=MAX_FRAMES as u8).contains(&count) && seq < count,
            "invalid fragment count/index"
        );
        ensure!(seq == self.next, "out-of-order or duplicate fragment");
        ensure!(
            seq + 1 == count || frame.len() == 20,
            "short non-final fragment"
        );
        if let Some(expected) = self.exchange {
            ensure!(
                expected == exchange && self.count == count,
                "fragment belongs to another exchange"
            );
        }
        ensure!(
            self.bytes.len() + frame.len() - 8 <= MAX_ENVELOPE,
            "message too large"
        );
        self.exchange = Some(exchange);
        self.count = count;
        self.bytes.extend_from_slice(&frame[8..]);
        self.next += 1;
        if self.next != count {
            return Ok(None);
        }
        ensure!(self.bytes.len() >= 18, "truncated message");
        let len = u16::from_le_bytes(self.bytes[16..18].try_into()?) as usize;
        ensure!(
            len <= 128 && self.bytes.len() == 18 + len,
            "message length mismatch"
        );
        let node = Uuid::from_slice(&self.bytes[..16])?;
        let text = String::from_utf8(self.bytes[18..].to_vec())?;
        Ok(Some(Message { node, text }))
    }
}

pub fn require_exchange(frame: &[u8], expected: u32) -> Result<()> {
    if header(frame)?.1 != expected {
        bail!("reply belongs to another exchange");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_roundtrip_across_fragments() {
        for text in [
            String::new(),
            "こんにちは🍺".to_string(),
            "あ".repeat(42),
            "a".repeat(128),
        ] {
            let message = Message {
                node: Uuid::new_v4(),
                text,
            };
            let frames = message.frames(42).unwrap();
            let mut assembler = Assembler::default();
            let mut result = None;
            for frame in frames {
                assert!(frame.len() <= 20);
                result = assembler.push(&frame).unwrap();
            }
            assert_eq!(result, Some(message));
        }
    }

    #[test]
    fn rejects_invalid_utf8_and_oversized_envelopes() {
        let message = Message {
            node: Uuid::nil(),
            text: "a".into(),
        };
        let mut frames = message.frames(1).unwrap();
        *frames.last_mut().unwrap().last_mut().unwrap() = 0xff;
        let mut a = Assembler::default();
        assert!(
            frames
                .iter()
                .try_for_each(|f| a.push(f).map(|_| ()))
                .is_err()
        );
        let too_large = Message {
            node: Uuid::nil(),
            text: "a".repeat(129),
        };
        assert!(too_large.frames(1).is_err());
        let mut frame = message.frames(1).unwrap()[0].clone();
        frame[7] = 255;
        assert!(Assembler::default().push(&frame).is_err());
    }

    #[test]
    fn rejects_bad_order_version_length_and_exchange() {
        let message = Message {
            node: Uuid::nil(),
            text: "hello world".into(),
        };
        let frames = message.frames(7).unwrap();
        assert!(Assembler::default().push(&frames[1]).is_err());
        let mut a = Assembler::default();
        a.push(&frames[0]).unwrap();
        assert!(a.push(&frames[0]).is_err());
        let mut bad = frames[1].clone();
        bad[2] = 8;
        assert!(a.push(&bad).is_err());
        bad = frames[0].clone();
        bad[0] = 2;
        assert!(Assembler::default().push(&bad).is_err());
        assert!(Assembler::default().push(&[1, 1]).is_err());
        let mut frames = frames;
        frames.last_mut().unwrap().pop();
        let mut a = Assembler::default();
        assert!(
            frames
                .iter()
                .try_for_each(|f| a.push(f).map(|_| ()))
                .is_err()
        );
    }
}
