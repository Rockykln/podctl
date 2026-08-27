//! BlueZ link + state refresher.
//!
//! Polls BlueZ via `bluetoothctl` on a slow cadence to keep the cached
//! `DeviceState` in sync with reality — connection state, name, address,
//! capabilities, trust, RSSI. Audio side gets refreshed at the same time
//! out of PipeWire.
//!
//! The proper AAP/L2CAP loop will live alongside this once the byte
//! captures land; until then battery/in-ear/buttons show no live data
//! when the daemon is up (the rest of the snapshot is real).

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::time::sleep;
use tracing::{debug, info, warn};

use podctl::{Capabilities, Event, Response, audio, bluez, caps::Model, model::PairedDevice};

use super::{Daemon, aap::AapTask};

const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Wait after a BlueZ connect edge before the first L2CAP attempt — the
/// device is usually still bringing up its profiles.
const AAP_SETTLE: Duration = Duration::from_millis(800);
/// First and last delay of the respawn backoff after the AAP thread dies.
const AAP_RETRY_BASE: Duration = Duration::from_secs(2);
const AAP_RETRY_MAX: Duration = Duration::from_secs(60);
/// A task that survived this long counts as healthy — the next failure
/// starts a fresh backoff streak rather than continuing the old one.
const AAP_HEALTHY_AFTER: Duration = Duration::from_secs(30);
/// How long a live AAP link may report no battery at all before we ask
/// the device to re-send its state dump.
const BATTERY_GRACE: Duration = Duration::from_secs(6);
/// Re-subscribes to try before recycling the socket outright.
const RESUBSCRIBE_TRIES: u32 = 2;
/// Socket recycles per connected period. Two is enough to cover a dump
/// that got lost; more would just be a reconnect loop against buds that
/// genuinely have nothing to say (asleep in a closed case).
const RECYCLE_LIMIT: u32 = 2;

pub async fn run(daemon: Arc<Daemon>) {
    info!(
        "link task: polling BlueZ + PipeWire every {:?}",
        POLL_INTERVAL
    );
    let mut prev_addr: Option<String> = None;
    let mut prev_connected = false;
    let mut aap = AapSupervisor::default();
    loop {
        // BlueZ + PipeWire calls are sync subprocess work; park them on
        // a blocking thread so we don't stall the runtime if pactl is slow.
        let refresh = tokio::task::spawn_blocking(snapshot_now).await.ok();
        if let Some(snap) = refresh {
            let prev_addr_for_event = prev_addr.clone();
            let snap_addr = snap.address.clone();
            let snap_connected = snap.connected;
            // Preserve cached AAP-derived fields (battery / in_ear) — the
            // BlueZ snapshot doesn't know about them.
            {
                let mut s = daemon.state.write().await;
                let cached_battery = s.battery;
                let cached_aap = s.aap_linked;
                let cached_in_ear = s.in_ear;
                let cached_settings = s.settings.clone();
                let cached_caps = s.capabilities;
                let had_model = cached_caps.model != Model::Unknown;
                *s = snap;
                s.battery = cached_battery;
                s.aap_linked = cached_aap;
                s.in_ear = cached_in_ear;
                s.settings = cached_settings;
                // Capabilities are sticky: one bad `bluetoothctl` poll
                // returns no device → Unknown caps, which would make the
                // daemon reject set_mode/set_conv (require_caps) and the
                // UI show "unknown model". Keep the resolved model until
                // a *different* known one replaces it. (A real unpair
                // leaves a stale label until restart — acceptable vs the
                // flapping this prevents.)
                if had_model && s.capabilities.model == Model::Unknown {
                    s.capabilities = cached_caps;
                }
                Daemon::touch(&mut s);
                if let Some(addr) = &s.address {
                    if prev_addr.as_deref() != Some(addr.as_str()) || s.connected != prev_connected
                    {
                        prev_addr = Some(addr.clone());
                        prev_connected = s.connected;
                        if s.connected {
                            let _ = daemon.events.send(Event::Connected {
                                name: s.name.clone().unwrap_or_default(),
                                address: addr.clone(),
                            });
                        } else if prev_addr_for_event.is_some() {
                            let _ = daemon.events.send(Event::Disconnected);
                        }
                    }
                } else if prev_addr.is_some() {
                    prev_addr = None;
                    prev_connected = false;
                    let _ = daemon.events.send(Event::Disconnected);
                }
            }
            // Spawn, restart or stop the AAP loop.
            aap.sync(&daemon, &snap_addr, snap_connected).await;
            aap.watchdog(&daemon).await;
        }
        sleep(POLL_INTERVAL).await;
    }
}

/// Keeps the AAP worker alive for as long as BlueZ says we are
/// connected.
///
/// Previously the task was spawned once per connect edge and never
/// looked at again. Two everyday cases left the slot holding a dead
/// thread — an L2CAP connect that lost the race with the device's own
/// profile setup, and a socket the AirPods dropped while the ACL link
/// stayed up. Either way the daemon kept reporting "connected" with an
/// empty battery until the user disconnected by hand. Now a finished
/// thread is reaped and respawned under an exponential backoff.
#[derive(Default)]
struct AapSupervisor {
    task: Option<AapTask>,
    started: Option<Instant>,
    /// Consecutive failures; drives the respawn delay.
    fails: u32,
    retry_at: Option<Instant>,
    /// Set while the link is up but no battery component has arrived —
    /// the deadline after which we re-ask the device.
    battery_due: Option<Instant>,
    resubscribes: u32,
    recycles: u32,
}

impl AapSupervisor {
    async fn sync(&mut self, daemon: &Arc<Daemon>, addr: &Option<String>, connected: bool) {
        self.reap();
        match (addr, connected) {
            (Some(mac), true) => {
                if self.task.is_some() {
                    return;
                }
                if self.retry_at.is_some_and(|t| Instant::now() < t) {
                    return;
                }
                if self.fails == 0 {
                    // Fresh connect edge: let the device settle first.
                    // On a retry the backoff has already provided the gap.
                    sleep(AAP_SETTLE).await;
                }
                self.spawn(daemon, mac);
            }
            _ => self.stop(daemon).await,
        }
    }

    fn spawn(&mut self, daemon: &Arc<Daemon>, mac: &str) {
        info!(attempt = self.fails + 1, "starting AAP task");
        self.task = Some(super::aap::spawn(daemon.clone(), mac.to_string()));
        self.started = Some(Instant::now());
        self.retry_at = None;
        self.battery_due = Some(Instant::now() + BATTERY_GRACE);
        self.resubscribes = 0;
    }

    /// Drop the AAP socket so the next `sync` opens a fresh one.
    ///
    /// Observed on a live Pro 2: the L2CAP connect succeeds, our
    /// handshake and subscribe both write cleanly, and the device then
    /// sends nothing at all — not even the handshake reply — for as long
    /// as the socket stays open. Re-subscribing on that socket changes
    /// nothing; reconnecting produces the full ~34-frame dump, battery
    /// included, within a second.
    async fn recycle(&mut self, reason: &str) {
        let Some(task) = self.task.take() else {
            return;
        };
        self.recycles += 1;
        warn!(reason, recycle = self.recycles, "recycling the AAP socket");
        tokio::task::spawn_blocking(move || task.shutdown())
            .await
            .ok();
        self.started = None;
        self.battery_due = None;
        // Reconnect on the next poll rather than inheriting the failure
        // backoff — this is a deliberate restart, not a crash.
        self.fails = 0;
        self.retry_at = None;
    }

    /// Collect a worker thread that has already returned and schedule the
    /// next attempt.
    fn reap(&mut self) {
        if !self.task.as_ref().is_some_and(AapTask::is_finished) {
            return;
        }
        let lived = self.started.map(|t| t.elapsed()).unwrap_or_default();
        if let Some(task) = self.task.take() {
            task.shutdown();
        }
        self.started = None;
        self.battery_due = None;
        // A long-lived session that ends is a normal drop, not a failing
        // one — restart it promptly instead of inheriting an old streak.
        if lived >= AAP_HEALTHY_AFTER {
            self.fails = 0;
        }
        self.fails = self.fails.saturating_add(1);
        let delay = backoff(self.fails);
        warn!(
            ?lived,
            ?delay,
            fails = self.fails,
            "AAP task exited — will retry"
        );
        self.retry_at = Some(Instant::now() + delay);
    }

    async fn stop(&mut self, daemon: &Arc<Daemon>) {
        self.fails = 0;
        self.retry_at = None;
        self.battery_due = None;
        self.resubscribes = 0;
        self.recycles = 0;
        let Some(task) = self.task.take() else {
            return;
        };
        self.started = None;
        tokio::task::spawn_blocking(move || task.shutdown())
            .await
            .ok();
        // Clear the AAP-derived state — values are stale once the link
        // drops. `case_lid_open` goes with them: leaving it at `Some(true)`
        // would swallow the lid edge on the next connect, and with it the
        // popup that edge triggers.
        let mut s = daemon.state.write().await;
        s.battery = podctl::Battery::default();
        s.in_ear = podctl::InEar::default();
        s.case_lid_open = None;
        Daemon::touch(&mut s);
    }

    /// The AirPods only push battery on change and once after subscribing.
    /// If that one dump is lost — a frame dropped during profile setup, a
    /// resubscribe the device ignored while busy — nothing ever refills
    /// it and the tray/popup show "—" for the whole session. Ask again.
    async fn watchdog(&mut self, daemon: &Arc<Daemon>) {
        if self.task.is_none() {
            return;
        }
        let known = {
            let s = daemon.state.read().await;
            s.battery.any_known()
        };
        if known {
            self.battery_due = None;
            self.resubscribes = 0;
            self.recycles = 0;
            return;
        }
        let Some(due) = self.battery_due else {
            self.battery_due = Some(Instant::now() + BATTERY_GRACE);
            return;
        };
        if Instant::now() < due {
            return;
        }
        if self.resubscribes < RESUBSCRIBE_TRIES {
            self.resubscribes += 1;
            match daemon.aap_resubscribe().await {
                Ok(()) => info!(
                    attempt = self.resubscribes,
                    "battery missing — re-subscribed"
                ),
                Err(e) => debug!(error = %e, "battery watchdog re-subscribe failed"),
            }
            self.battery_due = Some(Instant::now() + BATTERY_GRACE);
            return;
        }
        // Subscribing again didn't help. A socket that has produced no
        // frames at all is wedged — only a new connection revives it.
        if self.recycles < RECYCLE_LIMIT {
            let silent = self.task.as_ref().is_some_and(|t| t.frames() == 0);
            let reason = if silent {
                "no frames at all"
            } else {
                "no battery"
            };
            self.recycle(reason).await;
            return;
        }
        // Out of options. Stop poking; the next connect edge, or any
        // frame the device sends on its own, starts over.
        self.battery_due = None;
    }
}

fn backoff(fails: u32) -> Duration {
    AAP_RETRY_BASE
        .saturating_mul(1u32 << fails.saturating_sub(1).min(6))
        .min(AAP_RETRY_MAX)
}

/// Build a full DeviceState by querying BlueZ + PipeWire right now.
/// Returns the default (everything empty) when no AirPods are paired.
fn snapshot_now() -> podctl::DeviceState {
    let mut s = podctl::DeviceState::default();
    if let Some(dev) = bluez::primary_airpods() {
        s.address = Some(dev.address.clone());
        s.name = Some(dev.name.clone());
        s.connected = dev.connected;
        s.capabilities = dev.capabilities();
        s.bluetooth = bluez::bt_state(&dev);
    } else {
        s.capabilities = Capabilities::default();
    }
    s.audio = audio::snapshot();
    s.updated_at = now_secs();
    debug!(
        connected = s.connected,
        addr = s.address.as_deref().map(redact_mac).unwrap_or_default(),
        "snapshot refreshed"
    );
    s
}

/// Apple-OUI only — keep the vendor visible in logs, drop the device-unique bytes.
fn redact_mac(mac: &str) -> String {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() == 6 {
        format!("{}:{}:{}:**:**:**", parts[0], parts[1], parts[2])
    } else {
        "<mac>".into()
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// Request-side helpers: daemon dispatch calls into these for BlueZ ops.
// They shell out via `podctl::bluez` and update cached state on success.

pub async fn bt_connect(daemon: &Daemon) -> Response {
    let Some(addr) = daemon.state.read().await.address.clone() else {
        return Response::err("no AirPods paired — run 'podctl pair' first.");
    };
    match tokio::task::spawn_blocking(move || bluez::connect(&addr)).await {
        Ok(Ok(())) => {
            let mut s = daemon.state.write().await;
            s.connected = true;
            Daemon::touch(&mut s);
            Response::ok_done()
        }
        Ok(Err(e)) => Response::err(format!("{e}")),
        Err(e) => Response::err(format!("connect task: {e}")),
    }
}

pub async fn bt_disconnect(daemon: &Daemon) -> Response {
    let Some(addr) = daemon.state.read().await.address.clone() else {
        return Response::err("no AirPods paired.");
    };
    match tokio::task::spawn_blocking(move || bluez::disconnect(&addr)).await {
        Ok(Ok(())) => {
            let mut s = daemon.state.write().await;
            s.connected = false;
            Daemon::touch(&mut s);
            let _ = daemon.events.send(Event::Disconnected);
            Response::ok_done()
        }
        Ok(Err(e)) => Response::err(format!("{e}")),
        Err(e) => Response::err(format!("disconnect task: {e}")),
    }
}

pub async fn bt_pair(_daemon: &Daemon) -> Response {
    let res = tokio::task::spawn_blocking(|| {
        let found = bluez::discover(12)?;
        let target = found.into_iter().find(|d| !d.paired).ok_or_else(|| {
            anyhow::anyhow!("no new AirPods spotted — open the case until the LED blinks white.")
        })?;
        bluez::pair(&target.address)?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
    match res {
        Ok(Ok(())) => Response::ok_done(),
        Ok(Err(e)) => Response::err(format!("{e}")),
        Err(e) => Response::err(format!("pair task: {e}")),
    }
}

pub async fn bt_unpair(daemon: &Daemon) -> Response {
    let Some(addr) = daemon.state.read().await.address.clone() else {
        return Response::err("nothing to unpair.");
    };
    match tokio::task::spawn_blocking(move || bluez::unpair(&addr)).await {
        Ok(Ok(())) => {
            let mut s = daemon.state.write().await;
            *s = podctl::DeviceState::default();
            Daemon::touch(&mut s);
            Response::ok_done()
        }
        Ok(Err(e)) => Response::err(format!("{e}")),
        Err(e) => Response::err(format!("unpair task: {e}")),
    }
}

pub async fn bt_list() -> Response {
    let res = tokio::task::spawn_blocking(|| {
        bluez::paired_airpods().map(|v| {
            v.iter()
                .map(bluez::to_paired_device)
                .collect::<Vec<PairedDevice>>()
        })
    })
    .await;
    match res {
        Ok(Ok(items)) => Response::ok_list(items),
        Ok(Err(e)) => Response::err(format!("{e}")),
        Err(e) => Response::err(format!("list task: {e}")),
    }
}

pub async fn bt_set_trusted(daemon: &Daemon, on: bool) -> Response {
    let Some(addr) = daemon.state.read().await.address.clone() else {
        return Response::err("no AirPods paired.");
    };
    match tokio::task::spawn_blocking(move || bluez::set_trusted(&addr, on)).await {
        Ok(Ok(())) => {
            let mut s = daemon.state.write().await;
            s.bluetooth.trusted = on;
            s.bluetooth.auto_connect = on;
            Daemon::touch(&mut s);
            Response::ok_done()
        }
        Ok(Err(e)) => Response::err(format!("{e}")),
        Err(e) => Response::err(format!("trust task: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff(1), AAP_RETRY_BASE);
        assert_eq!(backoff(2), AAP_RETRY_BASE * 2);
        assert_eq!(backoff(3), AAP_RETRY_BASE * 4);
        // Saturates rather than overflowing the shift or the Duration.
        assert_eq!(backoff(7), AAP_RETRY_MAX);
        assert_eq!(backoff(u32::MAX), AAP_RETRY_MAX);
        // fails is never 0 at a call site, but it must not panic there.
        assert_eq!(backoff(0), AAP_RETRY_BASE);
    }

    #[test]
    fn redact_keeps_only_the_oui() {
        assert_eq!(redact_mac("AA:BB:CC:DD:EE:FF"), "AA:BB:CC:**:**:**");
        assert_eq!(redact_mac("garbage"), "<mac>");
    }
}
