mod daemon;

use std::sync::Arc;

use tokio::signal::unix::{SignalKind, signal};
use tracing::info;

use daemon::Daemon;

/// Printed for --help and for an argument the daemon does not know.
/// Without this it took anything and started regardless, so a mistyped
/// `podctld --help` quietly ran a second daemon.
const USAGE: &str = "\
podctld — the podctl daemon

It takes no arguments of its own; the settings live in daemon.toml.

    -h, --help       this text
    -V, --version    version and protocol

Start it through systemd:

    systemctl --user enable --now podctld

Settings:  $XDG_CONFIG_HOME/podctl/daemon.toml
Socket:    $XDG_RUNTIME_DIR/podctl.sock
Log level: PODCTL_LOG (for instance PODCTL_LOG=debug)
";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!(
                    "podctld {} (protocol {})",
                    env!("CARGO_PKG_VERSION"),
                    podctl::ipc::PROTO
                );
                return Ok(());
            }
            other => {
                eprintln!("podctld: unknown argument: {other}");
                eprint!("{USAGE}");
                std::process::exit(64);
            }
        }
    }

    podctl::logging::init("info");

    info!("podctld starting");

    let daemon = Arc::new(Daemon::new());

    {
        let d = Arc::clone(&daemon);
        tokio::spawn(async move {
            d.link_loop().await;
        });
    }

    {
        let d = Arc::clone(&daemon);
        tokio::spawn(async move { d.battery_loop().await });
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
