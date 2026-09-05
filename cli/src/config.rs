use crate::game::{Deck, MAX_NAME_BYTES};
use anyhow::{Result, ensure};
use clap::{Parser, ValueEnum};
use rand::Rng;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Role {
    Auto,
    Central,
    Peripheral,
}

#[derive(Debug, Parser)]
#[command(version, about = "近くの端末とBLEで5W1Hの文節を交換します")]
pub struct Config {
    #[arg(
        long,
        default_value = "anonymous",
        help = "交換相手に表示するユーザー名"
    )]
    pub name: String,
    #[arg(long, default_value = "ある日", help = "配布する「いつ」の文節")]
    pub when: String,
    #[arg(long, default_value = "パリに", help = "配布する「どこで」の文節")]
    pub r#where: String,
    #[arg(long, default_value = "犬が", help = "配布する「だれが」の文節")]
    pub who: String,
    #[arg(long, default_value = "行く", help = "配布する「なにをする」の文節")]
    pub what: String,
    #[arg(long, default_value = "散歩のため", help = "配布する「なぜ」の文節")]
    pub why: String,
    #[arg(long, default_value = "従順な", help = "配布する「どのように」の文節")]
    pub how: String,
    #[arg(long, value_enum, default_value = "auto")]
    pub role: Role,
    #[arg(long, default_value_t = -65, allow_hyphen_values = true)]
    pub rssi_threshold: i16,
    #[arg(long, default_value_t = 8)]
    pub role_min_secs: u64,
    #[arg(long, default_value_t = 12)]
    pub role_max_secs: u64,
    #[arg(long, default_value_t = 5)]
    pub exchange_timeout_secs: u64,
    #[arg(long, default_value_t = 5)]
    pub drain_secs: u64,
    #[arg(long, default_value_t = 30)]
    pub cooldown_secs: u64,
    #[arg(long, help = "localhostで設定・閲覧用Web UIを起動")]
    pub web: bool,
    #[arg(long, default_value_t = 8787, help = "Web UIの待受ポート")]
    pub web_port: u16,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.name.is_empty(), "--name は空にできません");
        ensure!(
            self.name.len() <= MAX_NAME_BYTES,
            "--name はUTF-8で{}バイト以内にしてください",
            MAX_NAME_BYTES
        );
        self.deck()?;
        ensure!(
            (-127..=20).contains(&self.rssi_threshold),
            "RSSI閾値は -127〜20 dBm にしてください"
        );
        ensure!(
            self.role_min_secs > 0
                && self.role_min_secs <= self.role_max_secs
                && self.role_max_secs <= 3600,
            "役割の時間は 1 <= min <= max <= 3600 秒にしてください"
        );
        ensure!(
            (1..=3600).contains(&self.exchange_timeout_secs),
            "交換タイムアウトは1〜3600秒にしてください"
        );
        ensure!(
            self.role_min_secs >= self.exchange_timeout_secs,
            "役割の最小時間は交換タイムアウト以上にしてください"
        );
        ensure!(self.drain_secs <= 60, "ドレイン時間は0〜60秒にしてください");
        ensure!(
            self.cooldown_secs <= 86400,
            "再交換間隔は0〜86400秒にしてください"
        );
        ensure!(self.web_port > 0, "Web UIのポートは1〜65535にしてください");
        Ok(())
    }

    pub fn deck(&self) -> Result<Deck> {
        Deck::new([
            self.when.clone(),
            self.r#where.clone(),
            self.who.clone(),
            self.what.clone(),
            self.why.clone(),
            self.how.clone(),
        ])
    }

    pub fn slot_duration(&self) -> Duration {
        Duration::from_secs(rand::thread_rng().gen_range(self.role_min_secs..=self.role_max_secs))
    }

    pub fn role_run_duration(&self, role: Role) -> (Duration, u32) {
        let mut duration = self.slot_duration();
        let mut extensions = 0;
        while random_role() == role {
            duration = duration.saturating_add(self.slot_duration());
            extensions += 1;
        }
        (duration, extensions)
    }
}

pub fn random_role() -> Role {
    if rand::random() {
        Role::Central
    } else {
        Role::Peripheral
    }
}

pub fn rssi_allowed(rssi: Option<i16>, threshold: i16) -> bool {
    // 127 is the Bluetooth sentinel for an unavailable RSSI.
    rssi.is_some_and(|value| (-127..=20).contains(&value) && value >= threshold)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_leave_time_for_one_exchange_and_transition_drain() {
        let config = Config::try_parse_from(["test"]).unwrap();
        assert_eq!(config.role_min_secs, 8);
        assert_eq!(config.role_max_secs, 12);
        assert_eq!(config.exchange_timeout_secs, 5);
        assert_eq!(config.drain_secs, 5);
        assert!(!config.web);
        assert_eq!(config.web_port, 8787);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn rssi_boundary_and_unavailable() {
        assert!(rssi_allowed(Some(-65), -65));
        assert!(rssi_allowed(Some(-40), -65));
        assert!(!rssi_allowed(Some(-66), -65));
        assert!(!rssi_allowed(None, -65));
        assert!(!rssi_allowed(Some(127), -65));
    }

    #[test]
    fn validates_bytes_and_time_ranges() {
        let mut c = Config::try_parse_from([
            "test",
            "--name",
            "alice",
            "--who",
            "猫が",
            "--rssi-threshold=-70",
        ])
        .unwrap();
        assert!(c.validate().is_ok());
        c.who = "あ".repeat(22);
        assert!(c.validate().is_err());
        c.who = "猫が".into();
        c.name = "あ".repeat(11);
        assert!(c.validate().is_err());
        c.name = "alice".into();
        c.role_min_secs = 0;
        assert!(c.validate().is_err());
        c.role_min_secs = 8;
        c.exchange_timeout_secs = 9;
        assert!(c.validate().is_err());
        c.exchange_timeout_secs = 5;
        c.drain_secs = 61;
        assert!(c.validate().is_err());
    }
}
