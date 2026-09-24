//! Listening for the AirPods' proximity advertisement over BlueZ.
//!
//! The buds announce a lid within a fraction of a second of it opening,
//! while the classic link needs several seconds to come back. Everyone
//! nearby hears those announcements, so only the ones that resolve
//! against our own identity key count.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::watch;
use tracing::{debug, info, warn};
use zbus::{Connection, MatchRule, MessageStream, Proxy, zvariant::OwnedValue};

use podctl::{Event, ble, keys};

use super::Daemon;

const BLUEZ: &str = "org.bluez";
const APPLE: u16 = 0x004c;
/// A busy adapter sends a signal per advertisement; too short a queue
/// would stall the connection rather than drop anything.
const QUEUE: usize = 256;

/// Runs while `active` says the classic link is down. Returns when the
/// channel closes, i.e. when the daemon shuts down.
pub async fn run(daemon: Arc<Daemon>, mut active: watch::Receiver<bool>) {
    loop {
        while !*active.borrow() {
            if active.changed().await.is_err() {
                return;
            }
        }
        if let Err(e) = scan_until_inactive(&daemon, &mut active).await {
            warn!(error = %e, "BLE scan stopped");
        }
        // A scan that never started — no keys yet, switched off, an
        // adapter that said no — must not be retried until something
        // changes, or this loop spins.
        let still_down = *active.borrow();
        if still_down && active.changed().await.is_err() {
            return;
        }
    }
}

async fn scan_until_inactive(
    daemon: &Arc<Daemon>,
    active: &mut watch::Receiver<bool>,
) -> zbus::Result<()> {
    if !super::config::load().ble {
        debug!("BLE listening switched off in daemon.toml");
        return Ok(());
    }
    let Some(mac) = daemon.state.read().await.address.clone() else {
        return Ok(());
    };
    let Some(k) = keys::load(&mac) else {
        debug!("no proximity keys for this pair — not scanning");
        return Ok(());
    };

    let conn = Connection::system().await?;
    let adapter = Proxy::new(&conn, BLUEZ, "/org/bluez/hci0", "org.bluez.Adapter1").await?;
    let mut filter: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    filter.insert("Transport", "le".into());
    filter.insert("DuplicateData", true.into());
    adapter
        .call_method("SetDiscoveryFilter", &(filter,))
        .await?;
    adapter.call_method("StartDiscovery", &()).await?;
    info!("listening for the case over BLE");

    // Both streams have to be polled side by side. Chaining them leaves
    // the second one unread until the first ends, its queue fills, and a
    // full queue stalls the whole connection: no signals at all.
    let props = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(BLUEZ)?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .build();
    let added = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(BLUEZ)?
        .interface("org.freedesktop.DBus.ObjectManager")?
        .member("InterfacesAdded")?
        .build();
    let mut props_stream = MessageStream::for_match_rule(props, &conn, Some(QUEUE)).await?;
    let mut added_stream = MessageStream::for_match_rule(added, &conn, Some(QUEUE)).await?;

    let mut state = LidState::default();
    let out = loop {
        tokio::select! {
            changed = active.changed() => {
                if changed.is_err() || !*active.borrow() {
                    break Ok(());
                }
            }
            msg = next_signal(&mut props_stream, &mut added_stream) => {
                let Some(msg) = msg else { break Ok(()) };
                if let Some(data) = manufacturer_data(&msg) {
                    let from = device_address(&msg).unwrap_or_default();
                    let mine = ble::parse_addr(&from).is_some_and(|a| ble::resolves(&k.irk, a));
                    debug!(%from, mine, bytes = data.len(), "apple advertisement");
                    if mine && let Some(adv) = ble::parse_advert(&data) {
                        state.apply(daemon, adv).await;
                    }
                }
            }
        }
    };

    let _ = adapter.call_method("StopDiscovery", &()).await;
    info!("stopped listening for the case");
    out
}

async fn next_signal(
    props: &mut MessageStream,
    added: &mut MessageStream,
) -> Option<zbus::Message> {
    loop {
        let msg = tokio::select! {
            m = props.next() => m,
            m = added.next() => m,
        };
        match msg {
            Some(Ok(m)) => return Some(m),
            // A dropped message is one advertisement of many; the stream
            // itself ending is the end of the scan.
            Some(Err(_)) => continue,
            None => return None,
        }
    }
}

/// The first advertisement after the case wakes still carries the old
/// lid state, so an edge only counts once a later one disagrees.
#[derive(Default)]
struct LidState {
    closed: Option<bool>,
    counter: Option<u8>,
}

impl LidState {
    async fn apply(&mut self, daemon: &Arc<Daemon>, adv: ble::Advert) {
        // The lid byte only says anything while the buds are in the
        // case; with one in an ear it reads "open" whatever the lid does.
        if !adv.both_in_case {
            self.closed = None;
            self.counter = None;
            return;
        }
        let moved = self.counter.is_some_and(|c| c != adv.lid_counter);
        let changed = self.closed != Some(adv.lid_closed);
        self.closed = Some(adv.lid_closed);
        self.counter = Some(adv.lid_counter);
        if !changed && !moved {
            return;
        }
        let snap = daemon
            .update_battery(|b| {
                b.left = adv.left.battery;
                b.right = adv.right.battery;
                b.case = adv.case.battery;
                b.left_charging = adv.left.charging;
                b.right_charging = adv.right.charging;
                b.case_charging = adv.case.charging;
            })
            .await;
        daemon.broadcast_event(Event::Battery(snap));
        if let Some(open) = daemon.update_case_lid(!adv.lid_closed).await {
            info!(open, "case lid over BLE");
            daemon.broadcast_event(Event::CaseLid { open });
        }
    }
}

fn device_address(msg: &zbus::Message) -> Option<String> {
    let path = msg.header().path()?.as_str().to_string();
    let dev = path.rsplit_once("/dev_")?.1;
    if dev.len() != 17 {
        return None;
    }
    Some(dev.replace('_', ":"))
}

fn manufacturer_data(msg: &zbus::Message) -> Option<Vec<u8>> {
    let body = msg.body();
    // PropertiesChanged(interface, changed, invalidated)
    if let Ok((iface, changed, _)) =
        body.deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
    {
        if iface != "org.bluez.Device1" {
            return None;
        }
        return apple_bytes(changed.get("ManufacturerData")?);
    }
    // InterfacesAdded(path, interfaces)
    let (_, ifaces) = body
        .deserialize::<(
            zbus::zvariant::OwnedObjectPath,
            HashMap<String, HashMap<String, OwnedValue>>,
        )>()
        .ok()?;
    apple_bytes(ifaces.get("org.bluez.Device1")?.get("ManufacturerData")?)
}

fn apple_bytes(value: &OwnedValue) -> Option<Vec<u8>> {
    let map: HashMap<u16, OwnedValue> = value.try_clone().ok()?.try_into().ok()?;
    let bytes: Vec<u8> = map.get(&APPLE)?.try_clone().ok()?.try_into().ok()?;
    Some(bytes)
}
