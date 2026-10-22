use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::time::sleep;
use tracing::{debug, warn};
use zbus::Connection;

use podctl::model::{DeviceState, Event};
use podctl::{Request, Response, WatchRole, client};

use crate::ipc;
use crate::menu::Menu;
use crate::notify::Notifier;
use crate::sni::{ITEM_PATH, Item, MENU_PATH, SharedState};
use crate::state::TrayState;

const BACKOFF_INITIAL_MS: u64 = 500;
const BACKOFF_MAX_MS: u64 = 30_000;
/// A session that ran at least this long counts as healthy: the next
/// reconnect starts from the short delay again.
const HEALTHY_SESSION: Duration = Duration::from_secs(5);

pub fn spawn(state: SharedState, conn: Connection, notifier: Option<Arc<Notifier>>) {
    tokio::spawn(async move { run(state, conn, notifier).await });
}

async fn run(state: SharedState, conn: Connection, notifier: Option<Arc<Notifier>>) {
    let mut backoff = BACKOFF_INITIAL_MS;
    loop {
        let started = Instant::now();
        match serve(&state, &conn, notifier.as_deref()).await {
            Ok(()) => debug!("watch stream closed, reconnecting"),
            Err(e) => warn!(error = %e, "watch stream errored, reconnecting"),
        }
        mark_disconnected(&state, &conn, notifier.as_deref()).await;
        // The backoff used to ratchet up permanently: after a handful of
        // daemon restarts the tray sat out 30 s before every reconnect,
        // showing "Not connected" long after the daemon was back. Only a
        // session that failed fast extends the delay.
        backoff = if started.elapsed() >= HEALTHY_SESSION {
            BACKOFF_INITIAL_MS
        } else {
            (backoff.saturating_mul(2)).min(BACKOFF_MAX_MS)
        };
        sleep(Duration::from_millis(backoff)).await;
    }
}

async fn serve(
    state: &SharedState,
    conn: &Connection,
    notifier: Option<&Notifier>,
) -> anyhow::Result<()> {
    refresh_status(state, conn, notifier).await;

    let mut reader = client::async_io::watch(WatchRole::Observer).await?;
    while let Some(l) = reader.next_line().await? {
        if l.is_empty() {
            continue;
        }
        match serde_json::from_str::<Response>(&l) {
            Ok(Response::Event(e)) => {
                // Capabilities only arrive in a full Status. On connect
                // (and any settings change) the daemon now knows the
                // model, so re-pull Status or the menu stays disabled.
                let needs_status = matches!(e, Event::Connected { .. } | Event::SettingsChanged);
                apply_event(state, e).await;
                if needs_status {
                    refresh_status(state, conn, notifier).await;
                } else {
                    notify_changed(state, conn, notifier).await;
                }
            }
            // The daemon sends the current state right after the
            // subscription, and again when it had to drop events.
            Ok(Response::State(ds)) => {
                apply_status(state, ds).await;
                notify_changed(state, conn, notifier).await;
            }
            Ok(Response::Done) => {}
            Ok(_) => {}
            Err(e) => warn!(error = %e, raw = %l, "malformed event line"),
        }
    }
    Ok(())
}

/// Pull a full Status from the daemon into the shared tray state.
/// Capabilities and most settings only come through here (events carry
/// deltas, not the model). `wait` bounds the answer for callers that a
/// person is waiting on.
pub(crate) async fn pull_status(state: &SharedState, wait: Option<Duration>) {
    if crate::demo::active() {
        return;
    }
    let answer = match wait {
        Some(w) => match tokio::time::timeout(w, ipc::send(&Request::Status)).await {
            Ok(a) => a,
            Err(_) => {
                debug!("status took longer than {w:?}, showing what we have");
                return;
            }
        },
        None => ipc::send(&Request::Status).await,
    };
    match answer {
        Ok(Response::State(s)) => apply_status(state, s).await,
        Ok(_) => {}
        Err(_) => {
            let mut s = state.write().await;
            s.daemon_up = false;
            s.forget_link();
        }
    }
}

async fn refresh_status(state: &SharedState, conn: &Connection, notifier: Option<&Notifier>) {
    pull_status(state, None).await;
    notify_changed(state, conn, notifier).await;
}

async fn apply_status(state: &SharedState, ds: DeviceState) {
    let mut s = state.write().await;
    s.daemon_up = true;
    s.pulled = Some(Instant::now());
    s.connected = ds.connected;
    s.name = ds.name;
    // Model/caps are sticky: a Status taken in the daemon's startup
    // window (or while BlueZ is mid-refresh) can carry Unknown — never
    // clobber a model we already resolved, or the menu would disable
    // Mode/Conv and the tooltip would show "unknown model".
    if ds.capabilities.model != podctl::caps::Model::Unknown {
        s.model = Some(ds.capabilities.model.label().to_string());
        s.capabilities = ds.capabilities;
    }
    s.battery = ds.battery;
    s.settings = ds.settings;
    s.audio = ds.audio;
}

async fn apply_event(state: &SharedState, event: Event) {
    let mut s = state.write().await;
    match event {
        Event::Connected { name, .. } => {
            s.connected = true;
            s.name = Some(name);
        }
        Event::Disconnected => s.forget_link(),
        Event::Battery(b) => s.battery = b,
        Event::Mode { mode } => s.settings.mode = Some(mode),
        Event::ConvAwareness { conv } => s.settings.conv_awareness = Some(conv),
        Event::InEar(_)
        | Event::CaseLid { .. }
        | Event::Press { .. }
        | Event::SettingsChanged
        | Event::ShowPopup => {}
    }
}

async fn mark_disconnected(state: &SharedState, conn: &Connection, notifier: Option<&Notifier>) {
    {
        let mut s = state.write().await;
        s.daemon_up = false;
        s.forget_link();
    }
    notify_changed(state, conn, notifier).await;
}

pub(crate) async fn notify_changed(
    state: &SharedState,
    conn: &Connection,
    notifier: Option<&Notifier>,
) {
    let snapshot = state.read().await.clone();
    if let Err(e) = emit_sni(conn, &snapshot).await {
        warn!(error = %e, "emit sni signals");
    }
    if let Err(e) = emit_menu(conn, &snapshot).await {
        warn!(error = %e, "emit menu signals");
    }
    if let Some(n) = notifier {
        n.observe(&snapshot).await;
    }
}

/// What the host was last told, so an event that changes nothing it
/// shows does not make it reload the icon.
#[derive(Clone, Default, PartialEq)]
struct Shown {
    icon: String,
    tooltip: (String, String),
    title: String,
    status: &'static str,
}

static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);

pub(crate) async fn emit_sni(conn: &Connection, snapshot: &TrayState) -> zbus::Result<()> {
    let now = Shown {
        icon: snapshot.icon_name(),
        tooltip: snapshot.tooltip(),
        title: snapshot.title(),
        status: snapshot.status(),
    };
    let before = SHOWN
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .replace(now.clone());
    let fresh = before.is_none();
    let before = before.unwrap_or_default();
    let iface_ref = conn.object_server().interface::<_, Item>(ITEM_PATH).await?;
    let emitter = iface_ref.signal_emitter();
    if fresh || before.icon != now.icon {
        Item::new_icon(emitter).await?;
        Item::new_attention_icon(emitter).await?;
    }
    if fresh || before.tooltip != now.tooltip {
        Item::new_tool_tip(emitter).await?;
    }
    if fresh || before.title != now.title {
        Item::new_title(emitter).await?;
    }
    if fresh || before.status != now.status {
        Item::new_status(emitter, now.status).await?;
    }
    Ok(())
}

pub(crate) async fn emit_menu(conn: &Connection, snapshot: &TrayState) -> zbus::Result<()> {
    let iface_ref = conn.object_server().interface::<_, Menu>(MENU_PATH).await?;
    let emitter = iface_ref.signal_emitter();
    let (updated, revision) = iface_ref.get().await.changes(snapshot);
    if let Some(rev) = revision {
        Menu::layout_updated(emitter, rev, 0).await?;
    }
    if !updated.is_empty() {
        Menu::items_properties_updated(emitter, updated, Vec::new()).await?;
    }
    Ok(())
}

/// A host that just appeared has seen nothing yet.
pub(crate) async fn forget_shown(conn: &Connection) {
    *SHOWN.lock().unwrap_or_else(|p| p.into_inner()) = None;
    if let Ok(menu) = conn.object_server().interface::<_, Menu>(MENU_PATH).await {
        menu.get().await.forget_shown();
    }
}
