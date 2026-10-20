use std::process::ExitCode;
use std::sync::Arc;

use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::RwLock;
use tracing::{info, warn};

use sni::{ITEM_PATH, Item, MENU_PATH, SharedState, StatusNotifierWatcherProxy};
use state::TrayState;

mod config;
mod ipc;
mod menu;
mod notify;
mod sni;
mod state;
mod watch;

const PROG: &str = "podctl-tray";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    install_default_sigpipe();
    podctl::logging::init("info");

    match run().await {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            eprintln!("{PROG}: {e}");
            ExitCode::from(70)
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let cfg = config::load();
    info!(
        left_click = cfg.left_click.as_str(),
        low_battery = cfg.low_battery_threshold,
        notify = cfg.notify_threshold,
        "tray config loaded"
    );
    let state: SharedState = Arc::new(RwLock::new(TrayState::new(cfg.low_battery_threshold)));
    let quit = Arc::new(tokio::sync::Notify::new());

    let pid = std::process::id();
    let well_known = format!("org.kde.StatusNotifierItem-{pid}-1");

    let conn = zbus::connection::Builder::session()
        .map_err(|e| anyhow::anyhow!("session bus: {e}"))?
        .name(well_known.as_str())
        .map_err(|e| anyhow::anyhow!("request bus name: {e}"))?
        .serve_at(ITEM_PATH, Item::new(Arc::clone(&state), cfg.left_click))
        .map_err(|e| anyhow::anyhow!("export {ITEM_PATH}: {e}"))?
        .serve_at(
            MENU_PATH,
            menu::Menu::new(Arc::clone(&state), Arc::clone(&quit)),
        )
        .map_err(|e| anyhow::anyhow!("export {MENU_PATH}: {e}"))?
        .build()
        .await
        .map_err(|e| anyhow::anyhow!("connect to session bus: {e}"))?;

    // A missing watcher is not a reason to die. Exiting here put the
    // unit into a restart loop every two seconds — on GNOME without
    // the extension, for as long as the session lasted — and the icon
    // never appeared once a panel came back later either.
    match register(&conn, &well_known).await {
        Ok(()) => info!(name = %well_known, "podctl-tray registered"),
        Err(e) => info!(
            error = %e,
            "no StatusNotifierWatcher yet — waiting for one (GNOME needs the AppIndicator extension, see 'podctl tray status')"
        ),
    }
    watch_for_watcher(conn.clone(), well_known.clone(), Arc::clone(&state));

    let notifier = match notify::Notifier::new(&conn, cfg.notify_threshold).await {
        Ok(n) => Some(Arc::new(n)),
        Err(e) => {
            warn!(error = %e, "no org.freedesktop.Notifications on the bus, low-battery alerts disabled");
            None
        }
    };

    // podctld and podctl-tray share the socket's shapes, so a tray
    // left running across an upgrade can fail at everything without a
    // word. Say it out loud, once.
    if let Ok(Some(note)) = tokio::task::spawn_blocking(podctl::client::version_mismatch).await {
        warn!("{note}");
        if let Some(n) = &notifier
            && let Err(e) = n.version_mismatch(&note).await
        {
            warn!(error = %e, "could not show the version mismatch");
        }
    }

    watch::spawn(Arc::clone(&state), conn.clone(), notifier);

    wait_for_shutdown(&quit).await;
    info!("shutting down");
    Ok(())
}

async fn register(conn: &zbus::Connection, well_known: &str) -> zbus::Result<()> {
    let watcher = StatusNotifierWatcherProxy::new(conn).await?;
    watcher.register_status_notifier_item(well_known).await
}

/// Panels come and go — plasmashell restarts, waybar is reloaded — and
/// each new one starts an empty watcher. Registering once at startup
/// meant the icon was gone until the tray itself was restarted.
fn watch_for_watcher(conn: zbus::Connection, well_known: String, state: SharedState) {
    tokio::spawn(async move {
        let dbus = match zbus::fdo::DBusProxy::new(&conn).await {
            Ok(d) => d,
            Err(e) => {
                warn!(error = %e, "cannot watch for a tray host");
                return;
            }
        };
        let mut owners = match dbus
            .receive_name_owner_changed_with_args(&[(0, sni::WATCHER_NAME)])
            .await
        {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "cannot watch for a tray host");
                return;
            }
        };
        use futures_util::StreamExt;
        while let Some(signal) = owners.next().await {
            let Ok(args) = signal.args() else { continue };
            if args.new_owner().is_none() {
                continue;
            }
            match register(&conn, &well_known).await {
                Ok(()) => {
                    info!("tray host is back — registered again");
                    // The new panel knows nothing about us yet.
                    let snapshot = state.read().await.clone();
                    if let Err(e) = watch::emit_sni(&conn, &snapshot).await {
                        warn!(error = %e, "emit sni signals after re-register");
                    }
                    if let Err(e) = watch::emit_menu(&conn, &snapshot).await {
                        warn!(error = %e, "emit menu signals after re-register");
                    }
                }
                Err(e) => warn!(error = %e, "re-register with the tray host failed"),
            }
        }
    });
}

async fn wait_for_shutdown(quit: &tokio::sync::Notify) {
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "cannot install SIGTERM handler, falling back to SIGINT only");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = quit.notified() => {}
            }
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
        _ = quit.notified() => {}
    }
}

fn install_default_sigpipe() {
    unsafe {
        let _ = libc_signal(SIGPIPE, SIG_DFL);
    }
}

const SIGPIPE: i32 = 13;
const SIG_DFL: usize = 0;
unsafe extern "C" {
    #[link_name = "signal"]
    fn libc_signal(sig: i32, handler: usize) -> usize;
}
