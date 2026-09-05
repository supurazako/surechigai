use anyhow::{Result, ensure};
use rand::seq::SliceRandom;
use uuid::Uuid;

pub const MAX_PHRASE_BYTES: usize = 64;
pub const MAX_NAME_BYTES: usize = 32;
pub const SLOT_COUNT: usize = 6;
pub const ALL_MISSING: u8 = (1 << SLOT_COUNT) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Slot {
    When = 0,
    Where = 1,
    Who = 2,
    What = 3,
    Why = 4,
    How = 5,
}

impl Slot {
    pub const ALL: [Self; SLOT_COUNT] = [
        Self::When,
        Self::Where,
        Self::Who,
        Self::What,
        Self::Why,
        Self::How,
    ];

    pub fn from_u8(value: u8) -> Result<Self> {
        Ok(match value {
            0 => Self::When,
            1 => Self::Where,
            2 => Self::Who,
            3 => Self::What,
            4 => Self::Why,
            5 => Self::How,
            _ => anyhow::bail!("unknown 5W1H slot"),
        })
    }

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn bit(self) -> u8 {
        1 << self.index()
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::When => "いつ",
            Self::Where => "どこで",
            Self::Who => "だれが",
            Self::What => "なにをする",
            Self::Why => "なぜ",
            Self::How => "どのように",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phrase {
    pub slot: Slot,
    pub text: String,
}

impl Phrase {
    pub fn new(slot: Slot, text: String) -> Result<Self> {
        ensure!(!text.is_empty(), "{} の文節が空です", slot.label());
        ensure!(
            text.len() <= MAX_PHRASE_BYTES,
            "{} はUTF-8で{}バイト以内にしてください",
            slot.label(),
            MAX_PHRASE_BYTES
        );
        Ok(Self { slot, text })
    }
}

#[derive(Clone, Debug)]
pub struct Deck {
    phrases: [String; SLOT_COUNT],
}

impl Deck {
    pub fn new(phrases: [String; SLOT_COUNT]) -> Result<Self> {
        for (slot, text) in Slot::ALL.into_iter().zip(&phrases) {
            Phrase::new(slot, text.clone())?;
        }
        Ok(Self { phrases })
    }

    pub fn phrase(&self, slot: Slot) -> Phrase {
        Phrase {
            slot,
            text: self.phrases[slot.index()].clone(),
        }
    }

    pub fn choose_for(&self, missing: u8) -> Option<Phrase> {
        let candidates = Slot::ALL
            .into_iter()
            .filter(|slot| missing & slot.bit() != 0)
            .collect::<Vec<_>>();
        candidates
            .choose(&mut rand::thread_rng())
            .copied()
            .map(|slot| self.phrase(slot))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentenceEntry {
    pub source: Uuid,
    pub source_name: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Sentence {
    pub round: Uuid,
    entries: [Option<SentenceEntry>; SLOT_COUNT],
}

impl Sentence {
    pub fn new() -> Self {
        Self {
            round: Uuid::new_v4(),
            entries: std::array::from_fn(|_| None),
        }
    }

    pub fn missing_mask(&self) -> u8 {
        Slot::ALL.into_iter().fold(0, |mask, slot| {
            mask | if self.entries[slot.index()].is_none() {
                slot.bit()
            } else {
                0
            }
        })
    }

    pub fn accept(&mut self, source: Uuid, source_name: String, phrase: Phrase) -> bool {
        let target = &mut self.entries[phrase.slot.index()];
        if target.is_some() {
            return false;
        }
        *target = Some(SentenceEntry {
            source,
            source_name,
            text: phrase.text,
        });
        true
    }

    pub fn entry(&self, slot: Slot) -> Option<&SentenceEntry> {
        self.entries[slot.index()].as_ref()
    }

    pub fn is_complete(&self) -> bool {
        self.missing_mask() == 0
    }

    pub fn render(&self) -> String {
        [
            Slot::When,
            Slot::How,
            Slot::Who,
            Slot::Where,
            Slot::Why,
            Slot::What,
        ]
        .into_iter()
        .filter_map(|slot| self.entry(slot).map(|entry| entry.text.as_str()))
        .collect::<Vec<_>>()
        .join(" ")
    }
}

impl Default for Sentence {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck() -> Deck {
        Deck::new([
            "ある日".into(),
            "パリに".into(),
            "犬が".into(),
            "行く".into(),
            "散歩のため".into(),
            "従順な".into(),
        ])
        .unwrap()
    }

    #[test]
    fn chooses_only_a_missing_slot() {
        let phrase = deck().choose_for(Slot::Who.bit()).unwrap();
        assert_eq!(phrase, Phrase::new(Slot::Who, "犬が".into()).unwrap());
        assert!(deck().choose_for(0).is_none());
    }

    #[test]
    fn sentence_does_not_overwrite_and_renders_in_sentence_order() {
        let source = Uuid::new_v4();
        let mut sentence = Sentence::new();
        assert!(sentence.accept(source, "alice".into(), deck().phrase(Slot::How)));
        assert!(sentence.accept(source, "alice".into(), deck().phrase(Slot::Who)));
        assert!(sentence.accept(source, "alice".into(), deck().phrase(Slot::Where)));
        assert!(sentence.accept(source, "alice".into(), deck().phrase(Slot::What)));
        assert!(!sentence.accept(source, "alice".into(), deck().phrase(Slot::What)));
        assert_eq!(sentence.render(), "従順な 犬が パリに 行く");
        assert_eq!(sentence.missing_mask(), Slot::When.bit() | Slot::Why.bit());
        assert_eq!(sentence.entry(Slot::Who).unwrap().source_name, "alice");
    }
}
