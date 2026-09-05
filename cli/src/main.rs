mod ble;

use anyhow::Result;
use clap::Parser;
use surechigai::{
    config::Config,
    web::{self, ViewerHandle},
};

#[tokio::main]
async fn main() -> Result<()> {
    let mut config = Config::parse();
    config.validate()?;
    if !config.web {
        return run_radio(config, None).await;
    }

    let (server, mut setup_receiver) = web::start(&config).await?;
    println!("Web UI: {}", server.address());
    println!("ブラウザで設定し、「交換を開始」を押してください");
    let setup = tokio::select! {
        setup = web::wait_for_setup(&mut setup_receiver) => Some(setup?),
        signal = shutdown_signal() => {
            signal?;
            println!("終了します…");
            None
        }
    };
    let Some(setup) = setup else {
        return server.shutdown().await;
    };
    setup.apply_to(&mut config);
    config.validate()?;
    let viewer = server.viewer();
    let result = run_radio(config, Some(viewer.clone())).await;
    viewer.set_stopped();
    result.and(server.shutdown().await)
}

async fn run_radio(config: Config, viewer: Option<ViewerHandle>) -> Result<()> {
    let mut radio = ble::Radio::with_viewer(config, viewer)?;
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
