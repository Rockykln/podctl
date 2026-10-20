//! What an iPhone does on the host side: pause on ear removal, resume on
//! return, lower the volume while the wearer is talking.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use zbus::Connection;

use podctl::{InEar, audio};

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const MPRIS_PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// Buds that come back after this long don't restart the music.
const RESUME_WINDOW: Duration = Duration::from_secs(15 * 60);
/// After a reconnect from the case the AirPods sink needs a moment to
/// reappear; resuming earlier would play through the speakers.
const SINK_WAIT: Duration = Duration::from_secs(15);
/// The in-ear sensor flickers while a bud is pulled out: on a Pro 2 it
/// reported "in" again for well over a second while the bud was in a hand.
/// Right after a pause the buds have to stay in longer before the music
/// comes back; later on, putting one back in is taken at its word.
const RESUME_SETTLE_EARLY: Duration = Duration::from_secs(2);
const RESUME_SETTLE: Duration = Duration::from_millis(500);
const FLICKER_AFTER_PAUSE: Duration = Duration::from_secs(3);
/// Proxies (browser integration, scrobblers) mirror another player's
/// track, and some turn Pause/Play into a toggle — calling both would
/// pause and unpause the same song. Only one player per track is touched,
/// after a short wait for the mirrors to catch up.
const SETTLE: Duration = Duration::from_millis(400);
/// Speech ducks the music by an amount that grows with how loud it
/// plays (measured before the sink volume, plus the sink volume in dB).
/// Tuned by ear: 10 dB at a normal -35 dBFS, 0.6 dB more per dB louder —
/// a 1:1 rule left loud-ish music too quiet.
const DUCK_REF_DB: f64 = -35.0;
const DUCK_REF_CUT: f64 = 10.0;
const DUCK_SLOPE: f64 = 0.6;
/// Always a noticeable dip, never all the way to silence.
const DUCK_MIN_DB: f64 = 6.0;
const DUCK_MAX_DB: f64 = 30.0;
/// Below this the music is effectively silent; nothing to duck.
const SILENCE_DB: f64 = -70.0;
/// The buds report the end of speech at every pause between sentences;
/// only restore once it has stayed quiet this long.
const RESTORE_HOLD: Duration = Duration::from_millis(1500);
/// Measured in 50 ms blocks; the duck follows the loud parts, so a quiet
/// bar at the wrong moment does not leave the music too loud.
const MEASURE: Duration = Duration::from_millis(600);
const BLOCK_SAMPLES: usize = 44_100 / 20 * 2;
/// Volume changes are faded in 2.5-point steps, not jumped.
const FADE_STEP_TENTHS: i32 = 25;
/// Down quickly so the first words are heard, back up gently.
const FADE_TICK: Duration = Duration::from_millis(25);
const RESTORE_TICK: Duration = Duration::from_millis(80);
/// The buds report the end of speech with a level of 4 or more. That
/// frame can go missing — a bud comes out, conversation awareness is
/// switched off at the phone, the link hangs — and the volume would
/// stay down with nothing to bring it back. The buds send nothing while
/// someone keeps talking, so this has to outlast a long sentence (12 s
/// without a frame seen on a Pro 2).
const DUCK_WATCHDOG: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Media {
    inner: Mutex<Inner>,
    /// Bumped whenever speech is (still) detected. Kept outside the lock:
    /// a restore holds it while fading up, and speech has to stop that.
    speech: Arc<AtomicU64>,
}

#[derive(Default)]
struct Inner {
    conn: Option<Connection>,
    /// Address of the buds. Every audio call needs it; without one the
    /// first Bluetooth sink on the system would be taken for theirs.
    addr: Option<String>,
    /// Players we paused, and how many buds were in ear right before.
    paused: Vec<String>,
    paused_at: Option<Instant>,
    resume_at_count: u8,
    /// Volume to restore once speech ends, and the level we set.
    ducked_from: Option<u8>,
    ducked_to: Option<u8>,
    /// A restore that speech stopped halfway; the next duck goes back
    /// down to `ducked_to` instead of starting over from there.
    cut_short: bool,
    /// When speech was last reported, for the watchdog.
    spoke_at: Option<Instant>,
    /// Bumped on every in-ear change so a pending resume can tell it
    /// was overtaken.
    ear_gen: u64,
}

impl Media {
    /// Told by the link task on every poll, so the audio calls act on
    /// the right device even across a reconnect.
    pub async fn set_address(&self, addr: Option<String>) {
        self.inner.lock().await.addr = addr;
    }

    pub async fn on_in_ear(&self, prev: InEar, now: InEar, enabled: bool) {
        if !ear_change_matters(prev, now, enabled) {
            return;
        }
        let before = prev.count_in_ear();
        let after = now.count_in_ear();
        let parked = parked_to_charge(now);
        let mut inner = self.inner.lock().await;
        inner.ear_gen += 1;
        let my_gen = inner.ear_gen;
        match ear_action(prev, now, !inner.paused.is_empty(), inner.resume_at_count) {
            EarAction::Nothing => return,
            EarAction::Pause => {
                let Some(addr) = inner.addr.clone() else {
                    return;
                };
                if !airpods_are_output(&addr).await {
                    return;
                }
                let paused = inner.pause_playing().await;
                if !paused.is_empty() {
                    info!(players = ?paused, before, after, "paused on ear removal");
                    inner.paused = paused;
                    inner.paused_at = Some(Instant::now());
                    inner.resume_at_count = before;
                }
                return;
            }
            EarAction::Resume => {}
        }
        let settle = resume_settle(inner.paused_at.map(|t| t.elapsed()));
        drop(inner);
        tokio::time::sleep(settle).await;
        let mut inner = self.inner.lock().await;
        if inner.ear_gen != my_gen || inner.paused.is_empty() {
            return;
        }
        let fresh = inner.paused_at.is_some_and(|t| t.elapsed() < RESUME_WINDOW);
        let players = std::mem::take(&mut inner.paused);
        inner.paused_at = None;
        if !fresh {
            debug!("buds back after the resume window — leaving playback paused");
            return;
        }
        let Some(addr) = inner.addr.clone() else {
            return;
        };
        // The sink can take seconds to appear. Everything else that
        // touches the player list — a bud coming back out, the ducking
        // on speech — waits behind this lock, so let go of it first.
        drop(inner);
        if !wait_for_airpods_output(&addr).await {
            warn!("AirPods sink never became the default — not resuming");
            return;
        }
        let mut inner = self.inner.lock().await;
        if inner.ear_gen != my_gen {
            debug!("ears changed while waiting for the sink — not resuming");
            return;
        }
        let resumed = inner.play(&players).await;
        if parked {
            inner.resume_at_count = 1;
            info!(players = ?resumed, "resumed — one bud charging, one in ear");
        } else {
            info!(players = ?resumed, "resumed after buds went back in");
        }
    }

    /// `level` is the last byte of an AAP 0x4B frame: 1–3 while speech
    /// starts and continues, 4 and up as it fades and ends.
    pub async fn on_conv_level(&self, level: u8, enabled: bool) {
        if enabled && matches!(level, 1..=3) {
            // Cancels a restore still waiting out the pause between
            // sentences, or one already fading up.
            self.speech.fetch_add(1, Ordering::SeqCst);
        }
        let mut inner = self.inner.lock().await;
        match level {
            1..=3 if enabled => {
                inner.spoke_at = Some(Instant::now());
                let Some(addr) = inner.addr.clone() else {
                    return;
                };
                if let Some(to) = inner.ducked_to
                    && inner.cut_short
                {
                    inner.cut_short = false;
                    if let Some(vol) = current_volume(&addr).await
                        && vol > to
                    {
                        fade(&addr, vol, to, FADE_TICK, None).await;
                        debug!(from = vol, to, "speech again — back down");
                    }
                    return;
                }
                if inner.ducked_from.is_some() || !inner.music_playing().await {
                    return;
                }
                let Some(vol) = current_volume(&addr).await else {
                    return;
                };
                // The level takes MEASURE to read; the first, smallest dip
                // runs meanwhile so the duck starts with the speech.
                let first = duck_first(vol);
                let (dipped, level) =
                    tokio::join!(fade(&addr, vol, first, FADE_TICK, None), measure_db(&addr));
                if !dipped {
                    return;
                }
                let Some(level) = level.filter(|l| *l >= SILENCE_DB) else {
                    fade(&addr, first, vol, RESTORE_TICK, None).await;
                    return;
                };
                let target = duck_target(vol, level).min(first);
                if target >= vol {
                    return;
                }
                if fade(&addr, first, target, FADE_TICK, None).await {
                    info!(
                        from = vol,
                        to = target,
                        level_db = format!("{level:.1}"),
                        "speech detected — volume lowered"
                    );
                    inner.ducked_from = Some(vol);
                    inner.ducked_to = Some(target);
                    inner.spoke_at = Some(Instant::now());
                    write_duck(&Ducked {
                        addr,
                        from: vol,
                        to: target,
                    });
                }
            }
            1..=3 => {}
            _ => {
                if inner.ducked_from.is_none() {
                    return;
                }
                let my_gen = self.speech.load(Ordering::SeqCst);
                drop(inner);
                tokio::time::sleep(RESTORE_HOLD).await;
                let mut inner = self.inner.lock().await;
                if self.speech.load(Ordering::SeqCst) == my_gen {
                    inner.restore_volume(&self.speech).await;
                }
            }
        }
    }

    /// Called when the link drops so a duck never outlives the session.
    pub async fn reset(&self) {
        self.inner.lock().await.restore_volume(&self.speech).await;
    }

    /// Checked on every poll of the link task: speech that never ended
    /// must not leave the volume down.
    pub async fn watchdog(&self) {
        let mut inner = self.inner.lock().await;
        if inner.ducked_from.is_none() {
            return;
        }
        if inner.spoke_at.is_some_and(|t| t.elapsed() > DUCK_WATCHDOG) {
            warn!("no end of speech reported — restoring the volume");
            inner.restore_volume(&self.speech).await;
        }
    }

    /// A duck noted on disk belongs to a session that did not get to
    /// undo it. Undone here once the sink is back — unless the volume
    /// has moved since, in which case it is somebody's choice now.
    pub async fn recover_duck(&self) {
        // The sink shows up a moment after the connect edge.
        let Some(note) = read_duck() else {
            return;
        };
        if !wait_for_sink(&note.addr).await {
            return;
        }
        let mut inner = self.inner.lock().await;
        if inner.ducked_from.is_some() {
            return;
        }
        let Some(note) = read_duck() else {
            return;
        };
        let Some(now) = current_volume(&note.addr).await else {
            return;
        };
        if now.abs_diff(note.to) > 1 {
            debug!(
                now,
                ducked_to = note.to,
                "volume moved since — dropping the note"
            );
            clear_duck();
            return;
        }
        let gen_now = self.speech.load(Ordering::SeqCst);
        let stop = Some((Arc::clone(&self.speech), gen_now));
        if fade(&note.addr, now, note.from, RESTORE_TICK, stop).await {
            info!(
                to = note.from,
                "undid a duck left behind by an earlier session"
            );
            clear_duck();
        } else if self.speech.load(Ordering::SeqCst) != gen_now {
            // Speech came in meanwhile: the note's duck is ours now, so
            // that the volume ends up back at the level before it.
            inner.ducked_from = Some(note.from);
            inner.ducked_to = Some(note.to);
            inner.cut_short = true;
            inner.spoke_at = Some(Instant::now());
        }
    }
}

impl Inner {
    async fn restore_volume(&mut self, speech: &Arc<AtomicU64>) {
        let Some(addr) = self.addr.clone() else {
            return;
        };
        let Some(vol) = self.ducked_from else {
            return;
        };
        let Some(from) = current_volume(&addr).await else {
            // The sink went with the link. The note on disk stays for
            // `recover_duck` to finish this once it is back.
            debug!("no sink to restore on — keeping the note");
            self.ducked_from = None;
            self.ducked_to = None;
            self.cut_short = false;
            self.spoke_at = None;
            return;
        };
        // Someone reached for the volume while we had it down: that is
        // their level now, not ours to undo. iOS behaves the same way.
        // After a restore cut short the volume sits between the two
        // levels by our own doing.
        if let Some(to) = self.ducked_to
            && !self.cut_short
            && from.abs_diff(to) > 1
        {
            debug!(from, to, "volume moved during the duck — leaving it alone");
            self.forget_duck();
            return;
        }
        let gen_now = speech.load(Ordering::SeqCst);
        let stop = Some((Arc::clone(speech), gen_now));
        if fade(&addr, from, vol, RESTORE_TICK, stop).await {
            info!(to = vol, "speech ended — volume restored");
            self.forget_duck();
        } else if speech.load(Ordering::SeqCst) != gen_now {
            debug!("speech again while restoring");
            self.cut_short = true;
        }
    }

    /// Only after a fade that worked: a sink that went away mid-restore
    /// must stay on the list, or the duck is lost for good.
    fn forget_duck(&mut self) {
        self.ducked_from = None;
        self.ducked_to = None;
        self.cut_short = false;
        self.spoke_at = None;
        clear_duck();
    }

    async fn connection(&mut self) -> Option<Connection> {
        if self.conn.is_none() {
            match Connection::session().await {
                Ok(c) => self.conn = Some(c),
                Err(e) => {
                    warn!(error = %e, "session bus unavailable — no media control");
                    return None;
                }
            }
        }
        self.conn.clone()
    }

    /// Ducking silence only makes the volume jump in the OSD and mixer.
    /// Both must agree: the sink carries audio and a player says Playing.
    async fn music_playing(&mut self) -> bool {
        let Some(addr) = self.addr.clone() else {
            return false;
        };
        let running = tokio::task::spawn_blocking(move || audio::sink_running_for(&addr))
            .await
            .unwrap_or(false);
        if !running {
            return false;
        }
        let Some(conn) = self.connection().await else {
            return false;
        };
        for name in players(&conn).await {
            if status(&conn, &name).await.as_deref() == Some("Playing") {
                return true;
            }
        }
        false
    }

    async fn pause_playing(&mut self) -> Vec<String> {
        let Some(conn) = self.connection().await else {
            return Vec::new();
        };
        let mut playing = Vec::new();
        for name in players(&conn).await {
            if status(&conn, &name).await.as_deref() == Some("Playing") {
                let track = title(&conn, &name).await;
                playing.push((name, track));
            }
        }
        let mut paused = Vec::new();
        let mut tracks: Vec<String> = Vec::new();
        let mut done: Vec<String> = Vec::new();
        for (name, track) in &playing {
            if done.contains(name) {
                continue;
            }
            if track.as_ref().is_some_and(|t| tracks.contains(t)) {
                debug!(player = %name, "mirrors a track already paused — skipped");
                continue;
            }
            if status(&conn, name).await.as_deref() != Some("Playing") {
                continue;
            }
            if let Err(e) = call(&conn, name, "Pause").await {
                debug!(player = %name, error = %e, "pause failed");
                continue;
            }
            paused.push(name.clone());
            tracks.extend(track.clone());
            done.push(name.clone());
            tokio::time::sleep(SETTLE).await;
            // Whatever stopped along with it is a mirror; its title may
            // differ from ours but match a third player's.
            for (other, other_track) in &playing {
                if done.contains(other) {
                    continue;
                }
                if status(&conn, other).await.as_deref() != Some("Playing") {
                    done.push(other.clone());
                    tracks.extend(other_track.clone());
                }
            }
        }
        paused
    }

    async fn play(&mut self, names: &[String]) -> Vec<String> {
        let Some(conn) = self.connection().await else {
            return Vec::new();
        };
        let mut resumed = Vec::new();
        for name in names {
            // Paused by someone else in the meantime, or already brought
            // back by a player we resumed before it.
            if status(&conn, name).await.as_deref() != Some("Paused") {
                continue;
            }
            match call(&conn, name, "Play").await {
                Ok(()) => resumed.push(name.clone()),
                Err(e) => debug!(player = %name, error = %e, "play failed"),
            }
            tokio::time::sleep(SETTLE).await;
        }
        resumed
    }
}

async fn players(conn: &Connection) -> Vec<String> {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return Vec::new();
    };
    let Ok(names) = dbus.list_names().await else {
        return Vec::new();
    };
    names
        .into_iter()
        .map(|n| n.to_string())
        .filter(|n| n.starts_with(MPRIS_PREFIX))
        .collect()
}

async fn player_proxy<'a>(conn: &Connection, name: &'a str) -> zbus::Result<zbus::Proxy<'a>> {
    zbus::proxy::Builder::new(conn)
        .destination(name)?
        .path(MPRIS_PATH)?
        .interface(MPRIS_PLAYER)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
}

async fn status(conn: &Connection, name: &str) -> Option<String> {
    let proxy = player_proxy(conn, name).await.ok()?;
    proxy.get_property::<String>("PlaybackStatus").await.ok()
}

async fn title(conn: &Connection, name: &str) -> Option<String> {
    let proxy = player_proxy(conn, name).await.ok()?;
    let meta: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
        proxy.get_property("Metadata").await.ok()?;
    let t = String::try_from(meta.get("xesam:title")?.try_clone().ok()?).ok()?;
    (!t.is_empty()).then_some(t)
}

async fn call(conn: &Connection, name: &str, method: &str) -> zbus::Result<()> {
    let proxy = player_proxy(conn, name).await?;
    proxy.call_method(method, &()).await.map(|_| ())
}

/// What a duck looks like on disk. A duck that outlives the daemon —
/// a crash, a logout, a SIGTERM — would otherwise leave the sink quiet
/// for good: PipeWire remembers the level per device, so the next
/// session starts quiet and nothing knows why.
#[derive(serde::Serialize, serde::Deserialize)]
struct Ducked {
    addr: String,
    from: u8,
    to: u8,
}

fn duck_path() -> std::path::PathBuf {
    podctl::xdg::state_dir().join("duck.json")
}

fn write_duck(d: &Ducked) {
    let p = duck_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_vec(d) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&p, bytes) {
                warn!(error = %e, "could not note the ducked volume");
            }
        }
        Err(e) => warn!(error = %e, "could not encode the ducked volume"),
    }
}

fn read_duck() -> Option<Ducked> {
    let bytes = std::fs::read(duck_path()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn clear_duck() {
    let _ = std::fs::remove_file(duck_path());
}

/// Exactly one bud in the case and the other in an ear.
fn parked_to_charge(now: InEar) -> bool {
    use podctl::model::EarStatus::{InCase, InEar as In};
    matches!((now.primary, now.secondary), (InCase, In) | (In, InCase))
}

/// Whether the change concerns playback at all. The device reports the
/// same ear state more than once, and the generation counter behind
/// this would cut off a resume that is still waiting.
fn ear_change_matters(prev: InEar, now: InEar, enabled: bool) -> bool {
    enabled && (prev.count_in_ear() != now.count_in_ear() || parked_to_charge(now))
}

#[derive(Debug, PartialEq, Eq)]
enum EarAction {
    Nothing,
    Pause,
    Resume,
}

/// What the change means for playback, given what we did last time.
fn ear_action(prev: InEar, now: InEar, already_paused: bool, resume_at: u8) -> EarAction {
    let before = prev.count_in_ear();
    let after = now.count_in_ear();
    if after < before {
        // A second bud out must not move the resume target.
        return if already_paused {
            EarAction::Nothing
        } else {
            EarAction::Pause
        };
    }
    if !already_paused {
        return EarAction::Nothing;
    }
    if after < resume_at && !parked_to_charge(now) {
        return EarAction::Nothing;
    }
    EarAction::Resume
}

fn resume_settle(since_pause: Option<Duration>) -> Duration {
    if since_pause.is_some_and(|d| d < FLICKER_AFTER_PAUSE) {
        RESUME_SETTLE_EARLY
    } else {
        RESUME_SETTLE
    }
}

async fn airpods_are_output(addr: &str) -> bool {
    let addr = addr.to_string();
    tokio::task::spawn_blocking(move || audio::sink_for(&addr).is_some_and(|s| s.is_default))
        .await
        .unwrap_or(false)
}

async fn wait_for_airpods_output(addr: &str) -> bool {
    let deadline = Instant::now() + SINK_WAIT;
    loop {
        if airpods_are_output(addr).await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn wait_for_sink(addr: &str) -> bool {
    let deadline = Instant::now() + SINK_WAIT;
    loop {
        let a = addr.to_string();
        let found = tokio::task::spawn_blocking(move || audio::sink_for(&a).is_some())
            .await
            .unwrap_or(false);
        if found {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn current_volume(addr: &str) -> Option<u8> {
    let addr = addr.to_string();
    tokio::task::spawn_blocking(move || audio::snapshot(&addr).volume_percent)
        .await
        .ok()
        .flatten()
}

/// PipeWire maps volume percent to gain cubically: dB = 60 · log10(v).
fn duck_target(vol: u8, level_db: f64) -> u8 {
    let v = f64::from(vol.max(1)) / 100.0;
    let heard = level_db + 60.0 * v.log10();
    let cut = (DUCK_REF_CUT + DUCK_SLOPE * (heard - DUCK_REF_DB)).clamp(DUCK_MIN_DB, DUCK_MAX_DB);
    (f64::from(vol) * 10f64.powf(-cut / 60.0)).round() as u8
}

/// The cut every duck reaches, whatever the level turns out to be.
fn duck_first(vol: u8) -> u8 {
    (f64::from(vol) * 10f64.powf(-DUCK_MIN_DB / 60.0)).round() as u8
}

/// RMS level of what is playing to the AirPods, before the sink volume.
async fn measure_db(addr: &str) -> Option<f64> {
    let addr = addr.to_string();
    tokio::task::spawn_blocking(move || {
        let sink = audio::sink_for(&addr)?;
        let mut child = audio::capture_cmd(&sink.name, 44_100, 2)?
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let mut out = child.stdout.take()?;
        let want = (44_100.0 * 4.0 * MEASURE.as_secs_f64()) as usize;
        let mut buf = vec![0u8; want];
        let mut got = 0;
        while got < want {
            match std::io::Read::read(&mut out, &mut buf[got..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        loud_db(buf[..got].as_chunks::<2>().0)
    })
    .await
    .ok()
    .flatten()
}

/// 80th-percentile block RMS in dBFS.
fn loud_db(samples: &[[u8; 2]]) -> Option<f64> {
    let mut blocks: Vec<f64> = samples
        .chunks(BLOCK_SAMPLES)
        .filter(|c| c.len() == BLOCK_SAMPLES)
        .map(|c| {
            let sum: f64 = c
                .iter()
                .map(|b| f64::from(i16::from_le_bytes(*b)).powi(2))
                .sum();
            (sum / c.len() as f64).sqrt()
        })
        .collect();
    if blocks.is_empty() {
        return None;
    }
    blocks.sort_by(f64::total_cmp);
    let rms = blocks[blocks.len() * 4 / 5].max(1.0);
    Some(20.0 * (rms / 32768.0).log10())
}

/// Stops early, and reports failure, once `stop`'s counter has moved on
/// from the value given with it.
async fn fade(
    addr: &str,
    from: u8,
    to: u8,
    tick: Duration,
    stop: Option<(Arc<AtomicU64>, u64)>,
) -> bool {
    let addr = addr.to_string();
    tokio::task::spawn_blocking(move || {
        let Some(sink) = audio::sink_for(&addr) else {
            return false;
        };
        let (mut v, end) = (i32::from(from) * 10, i32::from(to) * 10);
        let step = if end < v {
            -FADE_STEP_TENTHS
        } else {
            FADE_STEP_TENTHS
        };
        while (end - v).abs() > FADE_STEP_TENTHS {
            if stop
                .as_ref()
                .is_some_and(|(n, at)| n.load(Ordering::SeqCst) != *at)
            {
                return false;
            }
            v += step;
            if audio::set_volume_tenths(&sink, v as u16).is_err() {
                return false;
            }
            std::thread::sleep(tick);
        }
        audio::set_volume(&sink, to).is_ok()
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{
        BLOCK_SAMPLES, Ducked, EarAction, RESUME_SETTLE, RESUME_SETTLE_EARLY, clear_duck,
        duck_first, duck_path, duck_target, ear_action, ear_change_matters, loud_db,
        parked_to_charge, read_duck, resume_settle, write_duck,
    };
    use podctl::InEar;
    use podctl::model::EarStatus::{InCase, InEar as In, OutOfEar, Unknown};
    use std::time::Duration;

    #[test]
    fn only_one_bud_charging_while_the_other_plays_counts_as_parked() {
        let ear = |primary, secondary| InEar { primary, secondary };
        assert!(parked_to_charge(ear(InCase, In)));
        assert!(parked_to_charge(ear(In, InCase)));
        assert!(!parked_to_charge(ear(InCase, InCase)));
        assert!(!parked_to_charge(ear(InCase, OutOfEar)));
        assert!(!parked_to_charge(ear(In, In)));
        assert!(!parked_to_charge(ear(InCase, Unknown)));
    }

    #[test]
    fn an_unchanged_ear_state_is_no_change() {
        let ear = |primary, secondary| InEar { primary, secondary };
        assert!(!ear_change_matters(ear(In, In), ear(In, In), true));
        assert!(!ear_change_matters(
            ear(OutOfEar, OutOfEar),
            ear(OutOfEar, Unknown),
            true
        ));
        // One bud gone to charge: same count, still a change.
        assert!(ear_change_matters(ear(In, In), ear(In, InCase), true));
        assert!(ear_change_matters(ear(In, In), ear(In, OutOfEar), true));
        assert!(!ear_change_matters(
            ear(In, In),
            ear(OutOfEar, OutOfEar),
            false
        ));
    }

    #[test]
    fn what_an_ear_change_means_for_playback() {
        let ear = |primary, secondary| InEar { primary, secondary };
        let both = ear(In, In);
        let one = ear(In, OutOfEar);
        let none = ear(OutOfEar, OutOfEar);
        let parked = ear(In, InCase);

        assert_eq!(ear_action(both, one, false, 0), EarAction::Pause);
        assert_eq!(ear_action(one, none, true, 2), EarAction::Nothing);
        assert_eq!(ear_action(none, one, false, 0), EarAction::Nothing);
        // One bud back in is not enough when two were in before —
        // unless the other one is charging.
        assert_eq!(ear_action(none, one, true, 2), EarAction::Nothing);
        assert_eq!(ear_action(one, both, true, 2), EarAction::Resume);
        assert_eq!(ear_action(none, parked, true, 2), EarAction::Resume);
        assert_eq!(ear_action(none, one, true, 1), EarAction::Resume);
    }

    #[test]
    fn a_quiet_moment_does_not_lower_the_measurement() {
        let loud = (8192i16).to_le_bytes();
        let quiet = (256i16).to_le_bytes();
        let mut s = vec![loud; BLOCK_SAMPLES * 8];
        s.extend(vec![quiet; BLOCK_SAMPLES * 2]);
        let db = loud_db(&s).unwrap();
        assert!((db - -12.04).abs() < 0.1, "{db}");
    }

    #[test]
    fn louder_music_is_ducked_further() {
        // A normal listening level: 10 dB down.
        assert_eq!(duck_target(100, -35.0), 68);
        // Loud: 19 dB, not the 25 a 1:1 rule gave (too quiet by ear).
        assert_eq!(duck_target(100, -20.0), 48);
        // Already quiet: only the minimum dip.
        assert_eq!(duck_target(100, -55.0), 79);
        // Never below the floor.
        assert_eq!(duck_target(100, 0.0), 32);
    }

    #[test]
    fn a_bud_back_right_after_the_pause_has_to_stay_in_longer() {
        let s = Duration::from_secs;
        assert_eq!(resume_settle(Some(s(1))), RESUME_SETTLE_EARLY);
        assert_eq!(resume_settle(Some(s(5))), RESUME_SETTLE);
        assert_eq!(resume_settle(None), RESUME_SETTLE);
    }

    /// The dip that starts before the level is known must never go
    /// further than the measured duck would.
    #[test]
    fn the_first_dip_is_the_smallest_duck() {
        assert_eq!(duck_first(100), duck_target(100, -55.0));
        for vol in 1..=100 {
            for level in [-69.0, -55.0, -35.0, -20.0, 0.0] {
                assert!(duck_target(vol, level) <= duck_first(vol), "{vol} {level}");
            }
        }
    }

    /// The note is what lets a killed daemon's duck be undone later, so
    /// it has to survive the trip through the file.
    #[test]
    fn the_duck_note_round_trips() {
        let dir = std::env::temp_dir().join(format!("podctl-duck-test-{}", std::process::id()));
        unsafe { std::env::set_var("XDG_STATE_HOME", &dir) };
        assert!(read_duck().is_none(), "nothing noted yet");

        write_duck(&Ducked {
            addr: "AA:BB:CC:DD:EE:01".into(),
            from: 70,
            to: 40,
        });
        assert!(duck_path().ends_with("podctl/duck.json"));
        let back = read_duck().expect("the note");
        assert_eq!(
            (back.addr.as_str(), back.from, back.to),
            ("AA:BB:CC:DD:EE:01", 70, 40)
        );

        clear_duck();
        assert!(read_duck().is_none(), "note cleared");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
