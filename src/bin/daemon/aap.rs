//! AAP/L2CAP runtime. Owns the socket to the AirPods at PSM 0x1001 and
//! pumps notifications into the daemon's `DeviceState` + event bus.
//!
//! Wire layout was confirmed against AirPods Pro 2 USB-C btmon captures
//! on 2026-05-11.

use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use tracing::{debug, info, warn};

use podctl::{
    Event,
    aap::{self, Frame, battery_kind, battery_status, op},
    l2cap::L2capStream,
};

use super::Daemon;

/// Deadline for one connect attempt, and the timeout on reads and
/// writes afterwards. A link that has gone quiet is noticed within
/// this instead of holding a thread for the kernel's own timeout.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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
    /// The worker's socket, weakly. A raw fd number would outlive the
    /// socket: the worker closes it on its way out, the kernel hands the
    /// number to the next connection the daemon opens, and a late
    /// shutdown would cut that one instead.
    stream: Arc<OnceLock<Weak<L2capStream>>>,
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
        // Nothing to shut down once the worker has dropped the socket.
        if let Some(stream) = self.stream.get().and_then(Weak::upgrade) {
            let _ = stream.shutdown_both();
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

pub fn spawn(daemon: Arc<Daemon>, mac: String) -> AapTask {
    let cancelled = Arc::new(AtomicBool::new(false));
    let stream = Arc::new(OnceLock::new());
    let frames = Arc::new(AtomicU64::new(0));
    let handle = tokio::runtime::Handle::current();
    let join = {
        let cancelled = cancelled.clone();
        let stream = stream.clone();
        let frames = frames.clone();
        std::thread::Builder::new()
            .name(format!("aap-{}", short_mac(&mac)))
            .spawn(move || run(daemon, mac, cancelled, stream, frames, handle))
            .expect("spawn aap thread")
    };
    AapTask {
        cancelled,
        stream,
        frames,
        join: Some(join),
    }
}

fn run(
    daemon: Arc<Daemon>,
    mac: String,
    cancelled: Arc<AtomicBool>,
    stream_slot: Arc<OnceLock<Weak<L2capStream>>>,
    frames: Arc<AtomicU64>,
    rt: tokio::runtime::Handle,
) {
    info!(mac = %short_mac(&mac), "opening AAP socket on PSM 0x1001");
    let stream = match connect_with_retry(&mac, &cancelled) {
        Some(s) => Arc::new(s),
        None => return,
    };
    let _ = stream_slot.set(Arc::downgrade(&stream));

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

    // The buds only answer this while the link is fresh, and the answer
    // is the same every time, so ask once and keep it.
    if podctl::config::load().ble && podctl::keys::load(&mac).is_none() {
        std::thread::sleep(std::time::Duration::from_millis(200));
        match stream.send(&aap::request_keys().encode()) {
            Ok(()) => info!(mac = %short_mac(&mac), "asked for the proximity keys"),
            Err(e) => warn!(error = %e, "proximity key request failed"),
        }
    }

    // Hand the stream to the daemon so CLI-driven setters can write to it.
    rt.block_on(daemon.set_aap_stream(Some(stream.clone())));

    let mut buf = [0u8; 2048];
    let mut frame_count: u64 = 0;
    loop {
        let n_res = unsafe {
            let n = libc_read(stream.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len());
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
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                // The read timeout is there so this loop looks at the
                // cancel flag now and then; a quiet link is not a
                // broken one, and a signal is no reason to reconnect.
                if cancelled.load(Ordering::SeqCst) {
                    info!(frame_count, "AAP cancelled");
                    break;
                }
            }
            Ok(n) => {
                frame_count += 1;
                frames.store(frame_count, Ordering::Relaxed);
                let opcode = if n >= 5 { buf[4] } else { 0xFF };
                let preview = rx_preview(&buf[..n], opcode);
                debug!(frame_count, n, opcode = format!("{opcode:02x}"), %preview, "AAP rx");
                if ends_session(&buf[..n]) {
                    warn!(frame_count, "AirPods ended the AAP session — reopening");
                    break;
                }
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
        let cancel = || cancelled.load(Ordering::SeqCst);
        match L2capStream::connect(mac, aap::AAP_PSM, CONNECT_TIMEOUT, &cancel) {
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
    fn libc_read(fd: i32, buf: *mut std::ffi::c_void, count: usize) -> isize;
}

async fn handle_frame(daemon: &Arc<Daemon>, frame: &Frame) {
    match frame.opcode {
        op::BATTERY => apply_battery(daemon, &frame.payload).await,
        op::EAR_DETECTION => apply_ear(daemon, &frame.payload).await,
        op::CONV_AWARENESS_LVL => apply_conv_level(daemon, &frame.payload).await,
        op::SETTINGS => apply_settings(daemon, &frame.payload).await,
        op::PROXIMITY_KEYS => store_keys(daemon, &frame.payload).await,
        _ => debug!(
            opcode = format!("{:02x}", frame.opcode),
            len = frame.payload.len(),
            "unhandled AAP frame"
        ),
    }
}

async fn apply_settings(daemon: &Daemon, payload: &[u8]) {
    use podctl::aap::{setting, wire};
    let Some((&id, value)) = payload.split_first() else {
        debug!(bytes = ?payload, "settings frame too short");
        return;
    };
    // A missing value is not a zero: zero is "auto" for the mic
    // selection and silence for the chime.
    let Some(&v0) = value.first() else {
        debug!(id = format!("{id:02x}"), "settings frame without a value");
        return;
    };
    match id {
        setting::NOISE_CONTROL => {
            let mode = wire::mode_from_byte(v0);
            if let Some(m) = mode {
                let changed = daemon
                    .update_settings_if_changed(|s| {
                        let prev = s.mode;
                        s.mode = Some(m);
                        prev != Some(m)
                    })
                    .await;
                if changed {
                    daemon.broadcast_event(podctl::Event::Mode { mode: m });
                }
            }
        }
        setting::CONV_AWARE => {
            let conv = wire::conv_from_byte(v0);
            if let Some(c) = conv {
                let changed = daemon
                    .update_settings_if_changed(|s| {
                        let prev = s.conv_awareness;
                        s.conv_awareness = Some(c);
                        prev != Some(c)
                    })
                    .await;
                if changed {
                    daemon.broadcast_event(podctl::Event::ConvAwareness { conv: c });
                }
            }
        }
        setting::EAR_DETECTION => {
            // Some models keep this out of their dump (the Pro 2 USB-C
            // do), which is why podctl remembers the switch itself. A
            // model that does report it has the last word: the value may
            // have been changed on a phone since.
            let on = wire::switch_from_byte(v0);
            if let Some(b) = on {
                let changed = daemon
                    .update_settings_if_changed(|s| {
                        let prev = s.ear_detection;
                        s.ear_detection = Some(b);
                        prev != Some(b)
                    })
                    .await;
                if changed {
                    let _ = podctl::config::set_ear_detection(b);
                    daemon.broadcast_event(podctl::Event::SettingsChanged);
                }
            }
        }
        setting::MIC_MODE => {
            let mic = wire::mic_from_byte(v0);
            if let Some(m) = mic {
                let changed = daemon
                    .update_settings_if_changed(|s| {
                        let prev = s.mic_selection;
                        s.mic_selection = Some(m);
                        prev != Some(m)
                    })
                    .await;
                if changed {
                    let _ = podctl::config::set_mic(m);
                    daemon.broadcast_event(podctl::Event::SettingsChanged);
                }
            }
        }
        setting::ONE_BUD_ANC => {
            let on = wire::switch_from_byte(v0);
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

async fn apply_ear(daemon: &Arc<Daemon>, payload: &[u8]) {
    let Some(in_ear) = aap::parse_in_ear(payload) else {
        debug!(bytes = ?payload, "ear-detection frame too short");
        return;
    };
    debug!(primary = %in_ear.primary, secondary = %in_ear.secondary, "ear-detection update");
    let Some(prev) = daemon.set_in_ear(in_ear).await else {
        return;
    };
    daemon.broadcast_event(Event::InEar(in_ear));
    // The device-side ear-detection switch doubles as the auto-pause
    // switch, as on iOS. Media calls can wait on PipeWire for seconds,
    // so they must not hold up the frame reader.
    let enabled = daemon.state.read().await.settings.ear_detection != Some(false);
    let d = Arc::clone(daemon);
    tokio::spawn(async move { d.media.on_in_ear(prev, in_ear, enabled).await });
}

/// The reply carries the identity resolving key of the buds. Never log
/// the bytes: they are what tells this pair of buds from anyone else's.
async fn store_keys(daemon: &Arc<Daemon>, payload: &[u8]) {
    let Some(parsed) = aap::parse_keys(payload) else {
        debug!(len = payload.len(), "proximity key reply not understood");
        return;
    };
    let Some(mac) = daemon.state.read().await.address.clone() else {
        return;
    };
    let keys = podctl::keys::Keys {
        irk: parsed.irk,
        enc: parsed.enc,
    };
    match podctl::keys::save(&mac, &keys) {
        Ok(()) => info!(path = %podctl::keys::path(&mac).display(), "proximity keys stored"),
        Err(e) => warn!(error = %e, "could not store the proximity keys"),
    }
}

async fn apply_conv_level(daemon: &Arc<Daemon>, payload: &[u8]) {
    let Some(&level) = payload.last() else {
        return;
    };
    debug!(level, "conversation-awareness level");
    let enabled =
        daemon.state.read().await.settings.conv_awareness != Some(podctl::ConvAwareness::Off);
    let d = Arc::clone(daemon);
    tokio::spawn(async move { d.media.on_conv_level(level, enabled).await });
}

/// Seen mid-session on a Pro 2, right after a handshake reply nobody
/// asked for: the buds start over, and nothing more comes on this
/// socket until it is opened again.
fn ends_session(frame: &[u8]) -> bool {
    frame.starts_with(&[0x02, 0x00, 0x04, 0x00])
}

/// Hex dump of a received frame for the debug log, cut off at 24
/// bytes. The reply to the key request is the identity resolving key
/// itself: that one never goes to the log.
fn rx_preview(frame: &[u8], opcode: u8) -> String {
    if opcode == op::PROXIMITY_KEYS {
        return "<keys withheld>".into();
    }
    frame[..frame.len().min(24)]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn short_mac(mac: &str) -> String {
    mac.replace(':', "").chars().take(6).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::FromRawFd;

    unsafe extern "C" {
        fn socketpair(domain: i32, ty: i32, protocol: i32, sv: *mut i32) -> i32;
        fn dup2(oldfd: i32, newfd: i32) -> i32;
        fn close(fd: i32) -> i32;
        fn shutdown(fd: i32, how: i32) -> i32;
        #[link_name = "write"]
        fn libc_write(fd: i32, buf: *const std::ffi::c_void, count: usize) -> isize;
    }

    const AF_UNIX: i32 = 1;
    const SOCK_STREAM: i32 = 1;

    fn connected_pair() -> (i32, i32) {
        let mut fds = [0i32; 2];
        let rc = unsafe { socketpair(AF_UNIX, SOCK_STREAM, 0, fds.as_mut_ptr()) };
        assert_eq!(rc, 0, "socketpair");
        (fds[0], fds[1])
    }

    /// The key reply is 13 of the 16 IRK bytes within the first 24 of
    /// the frame — exactly what the preview used to print.
    #[test]
    fn the_key_reply_never_reaches_the_log() {
        let mut frame = vec![0x04, 0x00, 0x04, 0x00, op::PROXIMITY_KEYS, 0x00, 0x01];
        frame.extend_from_slice(&[0xAB; 20]);
        let shown = rx_preview(&frame, op::PROXIMITY_KEYS);
        assert!(!shown.contains("ab"), "{shown}");
        assert_eq!(rx_preview(&frame, op::BATTERY).len(), 24 * 3 - 1);
    }

    /// The worker closes its socket on the way out and `reap()` calls
    /// `shutdown` afterwards. Holding the fd *number* meant cutting
    /// whatever connection the kernel handed that number to next — an
    /// IPC client, a zbus connection — at random.
    #[test]
    fn shutdown_leaves_a_recycled_fd_alone() {
        let (aap, peer) = connected_pair();
        let stream = Arc::new(unsafe { L2capStream::from_raw_fd(aap) });
        let slot: Arc<OnceLock<Weak<L2capStream>>> = Arc::new(OnceLock::new());
        slot.set(Arc::downgrade(&stream)).expect("slot is empty");
        drop(stream);
        unsafe { close(peer) };

        // Put an unrelated connection on the number the AAP socket just
        // gave up — the kernel hands out the lowest free one, so this is
        // what happens by itself. `dup2` only to be sure of it.
        let (other, other_peer) = connected_pair();
        if other != aap {
            assert_eq!(unsafe { dup2(other, aap) }, aap, "dup2 onto the number");
            unsafe { close(other) };
        }

        let task = AapTask {
            cancelled: Arc::new(AtomicBool::new(false)),
            stream: slot,
            frames: Arc::new(AtomicU64::new(0)),
            join: None,
        };
        task.shutdown();

        let ping = |fd: i32| unsafe { libc_write(fd, b"ping".as_ptr().cast(), 4) };
        assert_eq!(ping(aap), 4, "the recycled socket is still open");

        // What the old code did, to show the line above would have
        // caught it: a shutdown by number hits whoever holds it now.
        unsafe { shutdown(aap, 2) };
        assert_eq!(ping(aap), -1, "a shutdown does break this socket");

        unsafe { close(aap) };
        unsafe { close(other_peer) };
    }

    #[test]
    fn only_the_close_packet_ends_a_session() {
        assert!(ends_session(&[0x02, 0x00, 0x04, 0x00, 0x00, 0x00]));
        let reply = [0x01, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x03, 0x00];
        assert!(!ends_session(&reply));
        let in_ear = [0x04, 0x00, 0x04, 0x00, 0x06, 0x00, 0x02, 0x02];
        assert!(!ends_session(&in_ear));
        assert!(!ends_session(&[0x02, 0x00]));
    }
}
