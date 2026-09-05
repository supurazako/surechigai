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
#[command(version, about = "近くの端末とBLEで短いメッセージを交換します")]
pub struct Config {
    #[arg(long, help = "交換するテキスト（UTF-8で最大128バイト）")]
    pub message: String,
    #[arg(long, value_enum, default_value = "auto")]
    pub role: Role,
    #[arg(long, default_value_t = -65, allow_hyphen_values = true)]
    pub rssi_threshold: i16,
    #[arg(long, default_value_t = 1)]
    pub role_min_secs: u64,
    #[arg(long, default_value_t = 5)]
    pub role_max_secs: u64,
    #[arg(long, default_value_t = 10)]
    pub exchange_timeout_secs: u64,
    #[arg(long, default_value_t = 30)]
    pub cooldown_secs: u64,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.message.len() <= 128,
            "--message はUTF-8で128バイト以内にしてください"
        );
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
            self.cooldown_secs <= 86400,
            "再交換間隔は0〜86400秒にしてください"
        );
        Ok(())
    }

    pub fn slot_duration(&self) -> Duration {
        Duration::from_secs(rand::thread_rng().gen_range(self.role_min_secs..=self.role_max_secs))
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
    fn rssi_boundary_and_unavailable() {
        assert!(rssi_allowed(Some(-65), -65));
        assert!(rssi_allowed(Some(-40), -65));
        assert!(!rssi_allowed(Some(-66), -65));
        assert!(!rssi_allowed(None, -65));
        assert!(!rssi_allowed(Some(127), -65));
    }

    #[test]
    fn validates_bytes_and_time_ranges() {
        let mut c =
            Config::try_parse_from(["test", "--message", "こんにちは", "--rssi-threshold=-70"])
                .unwrap();
        assert!(c.validate().is_ok());
        c.message = "あ".repeat(43);
        assert!(c.validate().is_err());
        c.message.clear();
        c.role_min_secs = 0;
        assert!(c.validate().is_err());
    }
}
