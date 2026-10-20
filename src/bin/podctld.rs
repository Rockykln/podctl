mod daemon;

use std::sync::Arc;

use tokio::signal::unix::{SignalKind, signal};
use tracing::info;
use tracing_subscriber::EnvFilter;

use daemon::Daemon;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    info!("podctld starting");

    let daemon = Arc::new(Daemon::new());

    {
        let d = Arc::clone(&daemon);
        tokio::spawn(async move {
            d.link_loop().await;
        });
    }

    let serve = {
        let d = Arc::clone(&daemon);
        tokio::spawn(async move { d.serve().await })
    };

    // A volume left ducked by a session that was killed, before
    // anything else happens.
    {
        let d = Arc::clone(&daemon);
        tokio::spawn(async move { d.recover_duck().await });
    }

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = sigterm.recv() => info!("SIGTERM — shutting down"),
        _ = sigint.recv()  => info!("SIGINT — shutting down"),
        r = serve => return r.unwrap_or_else(|e| Err(anyhow::anyhow!(e))),
    }

    // Asked for politely, so the duck does not outlive us.
    daemon.undo_duck().await;
    let _ = std::fs::remove_file(podctl::socket_path());
    info!("podctld stopped");
    Ok(())
}
