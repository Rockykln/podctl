use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::sleep;
use tracing::{debug, warn};
use zbus::Connection;

use podctl::model::{Battery, DeviceState, Event};
use podctl::{Request, Response, socket_path};

use crate::ipc;
use crate::menu::{self, Menu};
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

    let path = socket_path();
    let stream = UnixStream::connect(&path).await?;
    let (rx, mut tx) = stream.into_split();
    let mut line = serde_json::to_vec(&Request::Watch)?;
    line.push(b'\n');
    tx.write_all(&line).await?;
    tx.flush().await?;
    drop(tx);

    let mut reader = BufReader::new(rx).lines();
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
            Ok(Response::Done) => {}
            Ok(_) => {}
            Err(e) => warn!(error = %e, raw = %l, "malformed event line"),
        }
    }
    Ok(())
}

/// Pull a full Status from the daemon into the shared tray state.
/// Capabilities, mode and conv only come through here (events carry
/// deltas, not the model), so the menu calls this before it opens.
pub(crate) async fn pull_status(state: &SharedState) {
    if let Ok(Response::State(s)) = ipc::send(&Request::Status).await {
        apply_status(state, s).await;
    }
}

async fn refresh_status(state: &SharedState, conn: &Connection, notifier: Option<&Notifier>) {
    pull_status(state).await;
    notify_changed(state, conn, notifier).await;
}

async fn apply_status(state: &SharedState, ds: DeviceState) {
    let mut s = state.write().await;
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
    s.mode = ds.settings.mode;
    s.conv_awareness = ds.settings.conv_awareness;
}

async fn apply_event(state: &SharedState, event: Event) {
    let mut s = state.write().await;
    match event {
        Event::Connected { name, .. } => {
            s.connected = true;
            s.name = Some(name);
        }
        Event::Disconnected => {
            s.connected = false;
            s.battery = Battery::default();
            s.mode = None;
            s.conv_awareness = None;
        }
        Event::Battery(b) => s.battery = b,
        Event::Mode(m) => s.mode = Some(m),
        Event::ConvAwareness(c) => s.conv_awareness = Some(c),
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
        s.connected = false;
        s.battery = Battery::default();
    }
    notify_changed(state, conn, notifier).await;
}

async fn notify_changed(state: &SharedState, conn: &Connection, notifier: Option<&Notifier>) {
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

async fn emit_sni(conn: &Connection, snapshot: &TrayState) -> zbus::Result<()> {
    let iface_ref = conn.object_server().interface::<_, Item>(ITEM_PATH).await?;
    let emitter = iface_ref.signal_emitter();
    Item::new_icon(emitter).await?;
    Item::new_tool_tip(emitter).await?;
    Item::new_title(emitter).await?;
    Item::new_status(emitter, snapshot.status()).await?;
    Ok(())
}

async fn emit_menu(conn: &Connection, snapshot: &TrayState) -> zbus::Result<()> {
    let iface_ref = conn.object_server().interface::<_, Menu>(MENU_PATH).await?;
    let emitter = iface_ref.signal_emitter();
    let updated = menu::properties_snapshot(snapshot);
    Menu::items_properties_updated(emitter, updated, Vec::new()).await?;
    Ok(())
}
