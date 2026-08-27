//! AAP/L2CAP runtime. Owns the socket to the AirPods at PSM 0x1001 and
//! pumps notifications into the daemon's `DeviceState` + event bus.
//!
//! Wire layout was confirmed against AirPods Pro 2 USB-C btmon captures
//! on 2026-05-11.

use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

use tracing::{debug, info, warn};

use podctl::{
    Event,
    aap::{self, Frame, battery_kind, battery_status, op},
    l2cap::L2capStream,
};

use super::Daemon;

const HANDSHAKE: &[u8] = &[
    0x00, 0x00, 0x04, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Set feature flags — required on Pro 2 to receive battery / conv-awareness
/// notifications. Exact bytes from LibrePods Linux source (airpods_packets.h:
/// SET_SPECIFIC_FEATURES). The `0xD7` mask (not `0xFF`) is what actually
/// enables battery notifications on the Pro 2.
pub const SET_FEATURES_PRO2: &[u8] = &[
    0x04, 0x00, 0x04, 0x00, 0x4D, 0x00, 0xD7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Subscribe to the full notification stream. Five payload bytes (not four
/// as the public AAP Definitions doc shows) — confirmed against the
/// LibrePods Linux source, which is what actually works on Pro 2.
pub const SUBSCRIBE_NOTIFICATIONS: &[u8] = &[
    0x04, 0x00, 0x04, 0x00, 0x0F, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

pub struct AapTask {
    cancelled: Arc<AtomicBool>,
    fd: Arc<AtomicI32>,
    frames: Arc<AtomicU64>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl AapTask {
    /// True once the worker thread has returned — L2CAP connect gave up,
    /// the device closed the socket, or a read errored. The supervisor in
    /// `link.rs` polls this so a dead task gets reaped and respawned
    /// instead of silently leaving the daemon without battery data.
    pub fn is_finished(&self) -> bool {
        self.join.as_ref().is_some_and(|j| j.is_finished())
    }

    /// Frames read off the socket so far. Zero on an open-but-mute link,
    /// which is the shape that needs a fresh connection rather than
    /// another subscribe.
    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    pub fn shutdown(mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let fd = self.fd.load(Ordering::SeqCst);
        if fd >= 0 {
            unsafe {
                libc_shutdown(fd, 2);
            }
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

pub fn spawn(daemon: Arc<Daemon>, mac: String) -> AapTask {
    let cancelled = Arc::new(AtomicBool::new(false));
    let fd = Arc::new(AtomicI32::new(-1));
    let frames = Arc::new(AtomicU64::new(0));
    let handle = tokio::runtime::Handle::current();
    let join = {
        let cancelled = cancelled.clone();
        let fd = fd.clone();
        let frames = frames.clone();
        std::thread::Builder::new()
            .name(format!("aap-{}", short_mac(&mac)))
            .spawn(move || run(daemon, mac, cancelled, fd, frames, handle))
            .expect("spawn aap thread")
    };
    AapTask {
        cancelled,
        fd,
        frames,
        join: Some(join),
    }
}

fn run(
    daemon: Arc<Daemon>,
    mac: String,
    cancelled: Arc<AtomicBool>,
    fd_slot: Arc<AtomicI32>,
    frames: Arc<AtomicU64>,
    rt: tokio::runtime::Handle,
) {
    info!(mac = %short_mac(&mac), "opening AAP socket on PSM 0x1001");
    let stream = match connect_with_retry(&mac, &cancelled) {
        Some(s) => Arc::new(s),
        None => return,
    };
    fd_slot.store(stream.as_raw_fd(), Ordering::SeqCst);

    if let Err(e) = stream.send(HANDSHAKE) {
        warn!(error = %e, "AAP handshake write failed");
        return;
    }
    info!(mac = %short_mac(&mac), "AAP handshake sent");
    std::thread::sleep(std::time::Duration::from_millis(200));
    if let Err(e) = stream.send(SET_FEATURES_PRO2) {
        warn!(error = %e, "AAP set-features write failed");
        return;
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    if let Err(e) = stream.send(SUBSCRIBE_NOTIFICATIONS) {
        warn!(error = %e, "AAP subscribe write failed");
        return;
    }
    info!(mac = %short_mac(&mac), "AAP set-features + subscribe sent — listening");

    // Hand the stream to the daemon so CLI-driven setters can write to it.
    rt.block_on(daemon.set_aap_stream(Some(stream.clone())));

    let mut buf = [0u8; 2048];
    let mut frame_count: u64 = 0;
    loop {
        let n_res = unsafe {
            let n = libc_read(stream.as_raw_fd(), buf.as_mut_ptr(), buf.len());
            if n < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(n as usize)
            }
        };
        match n_res {
            Ok(0) => {
                info!(frame_count, "AAP EOF");
                break;
            }
            Ok(n) => {
                frame_count += 1;
                frames.store(frame_count, Ordering::Relaxed);
                let opcode = if n >= 5 { buf[4] } else { 0xFF };
                let preview: String = buf[..n.min(24)]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                info!(frame_count, n, opcode = format!("{opcode:02x}"), %preview, "AAP rx");
                if let Some(frame) = Frame::parse(&buf[..n]) {
                    rt.block_on(handle_frame(&daemon, &frame));
                }
            }
            Err(e) => {
                if cancelled.load(Ordering::SeqCst) {
                    info!(frame_count, "AAP cancelled");
                } else {
                    warn!(error = %e, frame_count, "AAP read failed");
                }
                break;
            }
        }
    }
    rt.block_on(daemon.set_aap_stream(None));
    info!(mac = %short_mac(&mac), frame_count, "AAP loop exited");
}

/// Delays between L2CAP connect attempts. BlueZ reports "Connected" as
/// soon as the ACL link is up, but the AirPods only start accepting PSM
/// 0x1001 once their own profile setup finishes — the first attempt
/// routinely fails with ECONNREFUSED/EHOSTDOWN. One attempt was enough
/// to lose battery data for the whole session, so retry across ~5 s.
const CONNECT_BACKOFF_MS: &[u64] = &[0, 300, 700, 1200, 2000];

fn connect_with_retry(mac: &str, cancelled: &AtomicBool) -> Option<L2capStream> {
    let mut last: Option<std::io::Error> = None;
    for (attempt, wait) in CONNECT_BACKOFF_MS.iter().enumerate() {
        if cancelled.load(Ordering::SeqCst) {
            return None;
        }
        if *wait > 0 {
            std::thread::sleep(std::time::Duration::from_millis(*wait));
            if cancelled.load(Ordering::SeqCst) {
                return None;
            }
        }
        match L2capStream::connect(mac, aap::AAP_PSM) {
            Ok(s) => {
                if attempt > 0 {
                    info!(attempt = attempt + 1, "L2CAP connected after retry");
                }
                return Some(s);
            }
            Err(e) => {
                debug!(attempt = attempt + 1, error = %e, "L2CAP connect attempt failed");
                last = Some(e);
            }
        }
    }
    match last {
        Some(e) => warn!(error = %e, attempts = CONNECT_BACKOFF_MS.len(), "L2CAP connect failed"),
        None => warn!("L2CAP connect failed"),
    }
    None
}

unsafe extern "C" {
    #[link_name = "read"]
    fn libc_read(fd: i32, buf: *mut u8, count: usize) -> isize;
}

async fn handle_frame(daemon: &Daemon, frame: &Frame) {
    match frame.opcode {
        op::BATTERY => apply_battery(daemon, &frame.payload).await,
        op::EAR_DETECTION => apply_ear(daemon, &frame.payload).await,
        op::SETTINGS => apply_settings(daemon, &frame.payload).await,
        _ => debug!(
            opcode = format!("{:02x}", frame.opcode),
            len = frame.payload.len(),
            "unhandled AAP frame"
        ),
    }
}

async fn apply_settings(daemon: &Daemon, payload: &[u8]) {
    use podctl::aap::setting;
    let Some((&id, value)) = payload.split_first() else {
        debug!(bytes = ?payload, "settings frame too short");
        return;
    };
    let v0 = value.first().copied().unwrap_or(0);
    match id {
        setting::NOISE_CONTROL => {
            // 1=Off, 2=NoiseCancellation, 3=Transparency, 4=Adaptive
            let mode = match v0 {
                1 => Some(podctl::Mode::Off),
                2 => Some(podctl::Mode::NoiseCancellation),
                3 => Some(podctl::Mode::Transparency),
                4 => Some(podctl::Mode::Adaptive),
                _ => None,
            };
            if let Some(m) = mode {
                let changed = daemon
                    .update_settings_if_changed(|s| {
                        let prev = s.mode;
                        s.mode = Some(m);
                        prev != Some(m)
                    })
                    .await;
                if changed {
                    daemon.broadcast_event(podctl::Event::Mode(m));
                }
            }
        }
        setting::CONV_AWARE => {
            let conv = match v0 {
                1 => Some(podctl::ConvAwareness::On),
                2 => Some(podctl::ConvAwareness::Off),
                _ => None,
            };
            if let Some(c) = conv {
                let changed = daemon
                    .update_settings_if_changed(|s| {
                        let prev = s.conv_awareness;
                        s.conv_awareness = Some(c);
                        prev != Some(c)
                    })
                    .await;
                if changed {
                    daemon.broadcast_event(podctl::Event::ConvAwareness(c));
                }
            }
        }
        setting::ONE_BUD_ANC => {
            let on = match v0 {
                1 => Some(true),
                2 => Some(false),
                _ => None,
            };
            if let Some(b) = on {
                daemon
                    .update_settings_if_changed(|s| {
                        let prev = s.one_bud_anc;
                        s.one_bud_anc = Some(b);
                        prev != Some(b)
                    })
                    .await;
            }
        }
        setting::CHIME_VOLUME => {
            let prev_changed = daemon
                .update_settings_if_changed(|s| {
                    let prev = s.chime_volume;
                    s.chime_volume = Some(v0);
                    prev != Some(v0)
                })
                .await;
            let _ = prev_changed;
        }
        setting::AUTO_ANC_LEVEL => {
            daemon
                .update_settings_if_changed(|s| {
                    let prev = s.auto_anc_level;
                    s.auto_anc_level = Some(v0);
                    prev != Some(v0)
                })
                .await;
        }
        _ => debug!(id = format!("{:02x}", id), bytes = ?value,
                    "settings frame (id not mapped)"),
    }
}

async fn apply_battery(daemon: &Daemon, payload: &[u8]) {
    let comps = aap::parse_battery(payload);
    if comps.is_empty() {
        debug!(bytes = ?payload, "battery frame with no components");
        return;
    }
    let snap = daemon
        .update_battery(|b| {
            for c in &comps {
                let level = if c.status == battery_status::DISCONNECTED || c.level > 100 {
                    None
                } else {
                    Some(c.level)
                };
                let charging = c.status == battery_status::CHARGING;
                match c.kind {
                    battery_kind::LEFT => {
                        b.left = level;
                        b.left_charging = charging;
                    }
                    battery_kind::RIGHT => {
                        b.right = level;
                        b.right_charging = charging;
                    }
                    battery_kind::CASE => {
                        b.case = level;
                        b.case_charging = charging;
                    }
                    _ => debug!(kind = format!("{:02x}", c.kind), "unknown battery kind"),
                }
            }
        })
        .await;
    debug!(?comps, "battery updated");
    daemon.broadcast_event(Event::Battery(snap));

    if let Some(open) = daemon.update_case_lid(snap.case.is_some()).await {
        debug!(open, "case lid edge (battery-visibility heuristic)");
        daemon.broadcast_event(Event::CaseLid { open });
    }
}

async fn apply_ear(daemon: &Daemon, payload: &[u8]) {
    let Some(in_ear) = aap::parse_in_ear(payload) else {
        debug!(bytes = ?payload, "ear-detection frame too short");
        return;
    };
    debug!(primary = %in_ear.primary, secondary = %in_ear.secondary, "ear-detection update");
    daemon.set_in_ear(in_ear).await;
    daemon.broadcast_event(Event::InEar(in_ear));
}

fn short_mac(mac: &str) -> String {
    mac.replace(':', "").chars().take(6).collect()
}

unsafe extern "C" {
    #[link_name = "shutdown"]
    fn libc_shutdown(fd: i32, how: i32) -> i32;
}
