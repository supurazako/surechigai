use crate::game::{ALL_MISSING, MAX_NAME_BYTES, MAX_PHRASE_BYTES, Phrase, Slot};
use anyhow::{Result, bail, ensure};
use uuid::Uuid;

pub const SERVICE: Uuid = Uuid::from_u128(0x478f5400_73ad_47a6_a131_562697033a90);
pub const INFO: Uuid = Uuid::from_u128(0x478f5401_73ad_47a6_a131_562697033a90);
pub const RX: Uuid = Uuid::from_u128(0x478f5402_73ad_47a6_a131_562697033a90);
pub const TX: Uuid = Uuid::from_u128(0x478f5403_73ad_47a6_a131_562697033a90);
pub const VERSION: u8 = 2;
pub const DATA: u8 = 1;
pub const SELECT: u8 = 2;
pub const ACK: u8 = 3;
const PROFILE: u8 = 1;
const GIFT: u8 = 2;
const NO_GIFT: u8 = u8::MAX;
const CHUNK: usize = 12;
const MAX_ENVELOPE: usize = 1 + 16 + 1 + 1 + MAX_PHRASE_BYTES;
pub const MAX_FRAMES: usize = MAX_ENVELOPE.div_ceil(CHUNK);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub node: Uuid,
    pub name: String,
    pub round: Uuid,
    pub missing: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GiftPacket {
    pub receiver_round: Uuid,
    pub gift: Option<Phrase>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    Profile(Profile),
    Gift(GiftPacket),
}

impl Packet {
    pub fn frames(&self, exchange: u32) -> Result<Vec<Vec<u8>>> {
        let bytes = self.encode()?;
        ensure!(bytes.len() <= MAX_ENVELOPE, "packet too large");
        let count = bytes.len().div_ceil(CHUNK) as u8;
        Ok(bytes
            .chunks(CHUNK)
            .enumerate()
            .map(|(index, chunk)| {
                let mut frame = command(DATA, exchange);
                frame.extend_from_slice(&[index as u8, count]);
                frame.extend_from_slice(chunk);
                frame
            })
            .collect())
    }

    fn encode(&self) -> Result<Vec<u8>> {
        match self {
            Self::Profile(profile) => {
                ensure!(profile.missing & !ALL_MISSING == 0, "invalid missing mask");
                ensure!(
                    !profile.name.is_empty() && profile.name.len() <= MAX_NAME_BYTES,
                    "invalid user name"
                );
                let mut bytes = vec![PROFILE];
                bytes.extend_from_slice(profile.node.as_bytes());
                bytes.extend_from_slice(profile.round.as_bytes());
                bytes.push(profile.missing);
                bytes.push(profile.name.len() as u8);
                bytes.extend_from_slice(profile.name.as_bytes());
                Ok(bytes)
            }
            Self::Gift(packet) => {
                let mut bytes = vec![GIFT];
                bytes.extend_from_slice(packet.receiver_round.as_bytes());
                match &packet.gift {
                    Some(phrase) => {
                        Phrase::new(phrase.slot, phrase.text.clone())?;
                        bytes.push(phrase.slot as u8);
                        bytes.push(phrase.text.len() as u8);
                        bytes.extend_from_slice(phrase.text.as_bytes());
                    }
                    None => bytes.extend_from_slice(&[NO_GIFT, 0]),
                }
                Ok(bytes)
            }
        }
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(!bytes.is_empty(), "empty packet");
        match bytes[0] {
            PROFILE => {
                ensure!(bytes.len() >= 35, "invalid profile length");
                let missing = bytes[33];
                let name_len = bytes[34] as usize;
                ensure!(missing & !ALL_MISSING == 0, "invalid missing mask");
                ensure!(
                    (1..=MAX_NAME_BYTES).contains(&name_len) && bytes.len() == 35 + name_len,
                    "invalid user name length"
                );
                Ok(Self::Profile(Profile {
                    node: Uuid::from_slice(&bytes[1..17])?,
                    name: String::from_utf8(bytes[35..].to_vec())?,
                    round: Uuid::from_slice(&bytes[17..33])?,
                    missing,
                }))
            }
            GIFT => {
                ensure!(bytes.len() >= 19, "truncated gift");
                let receiver_round = Uuid::from_slice(&bytes[1..17])?;
                let slot = bytes[17];
                let len = bytes[18] as usize;
                ensure!(
                    len <= MAX_PHRASE_BYTES && bytes.len() == 19 + len,
                    "gift length mismatch"
                );
                let gift = if slot == NO_GIFT {
                    ensure!(len == 0, "invalid empty gift");
                    None
                } else {
                    Some(Phrase::new(
                        Slot::from_u8(slot)?,
                        String::from_utf8(bytes[19..].to_vec())?,
                    )?)
                };
                Ok(Self::Gift(GiftPacket {
                    receiver_round,
                    gift,
                }))
            }
            _ => bail!("unknown packet type"),
        }
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
    pub fn push(&mut self, frame: &[u8]) -> Result<Option<Packet>> {
        let (kind, exchange) = header(frame)?;
        ensure!(kind == DATA && frame.len() >= 9, "expected data frame");
        let (seq, count) = (frame[6], frame[7]);
        ensure!(
            (1..=MAX_FRAMES as u8).contains(&count) && seq < count,
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
            "packet too large"
        );
        self.exchange = Some(exchange);
        self.count = count;
        self.bytes.extend_from_slice(&frame[8..]);
        self.next += 1;
        if self.next != count {
            return Ok(None);
        }
        Ok(Some(Packet::decode(&self.bytes)?))
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

    fn roundtrip(packet: Packet) {
        let frames = packet.frames(42).unwrap();
        let mut assembler = Assembler::default();
        let mut result = None;
        for frame in frames {
            assert!(frame.len() <= 20);
            result = assembler.push(&frame).unwrap();
        }
        assert_eq!(result, Some(packet));
    }

    #[test]
    fn packets_roundtrip_across_fragments() {
        roundtrip(Packet::Profile(Profile {
            node: Uuid::new_v4(),
            name: "alice".into(),
            round: Uuid::new_v4(),
            missing: ALL_MISSING,
        }));
        roundtrip(Packet::Gift(GiftPacket {
            receiver_round: Uuid::new_v4(),
            gift: Some(Phrase::new(Slot::How, "従順な🍺".into()).unwrap()),
        }));
        roundtrip(Packet::Gift(GiftPacket {
            receiver_round: Uuid::new_v4(),
            gift: None,
        }));
        roundtrip(Packet::Gift(GiftPacket {
            receiver_round: Uuid::new_v4(),
            gift: Some(Phrase::new(Slot::What, "a".repeat(64)).unwrap()),
        }));
    }

    #[test]
    fn rejects_invalid_payloads_and_fragments() {
        let packet = Packet::Gift(GiftPacket {
            receiver_round: Uuid::nil(),
            gift: Some(Phrase::new(Slot::What, "hello".into()).unwrap()),
        });
        let frames = packet.frames(7).unwrap();
        assert!(Assembler::default().push(&frames[1]).is_err());
        let mut assembler = Assembler::default();
        assembler.push(&frames[0]).unwrap();
        assert!(assembler.push(&frames[0]).is_err());

        let mut bad_version = frames[0].clone();
        bad_version[0] = VERSION + 1;
        assert!(Assembler::default().push(&bad_version).is_err());

        let too_large = Packet::Gift(GiftPacket {
            receiver_round: Uuid::nil(),
            gift: Some(Phrase {
                slot: Slot::What,
                text: "a".repeat(MAX_PHRASE_BYTES + 1),
            }),
        });
        assert!(too_large.frames(1).is_err());
    }
}
