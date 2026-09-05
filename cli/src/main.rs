mod ble;

use anyhow::Result;
use clap::Parser;
use surechigai::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::parse();
    config.validate()?;
    let mut radio = ble::Radio::new(config)?;
    let result = tokio::select! {
        result = radio.run() => result,
        signal = shutdown_signal() => {
            signal?;
            println!("終了します…");
            Ok(())
        }
    };
    let cleanup = radio.cleanup().await;
    result.and(cleanup)
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = terminate.recv() => (),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}
