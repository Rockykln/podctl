//! What an iPhone does on the host side: pause on ear removal, resume on
//! return, lower the volume while the wearer is talking.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::{debug, info, warn};
use zbus::Connection;

use podctl::audio::SinkInfo;
use podctl::{InEar, audio};

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const MPRIS_PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// Buds that come back after this long don't restart the music.
const RESUME_WINDOW: Duration = Duration::from_secs(15 * 60);
/// The same after the link dropped. Shorter: opening the case connects
/// them long before anyone listens again.
const LINK_RESUME_WINDOW: Duration = Duration::from_secs(5 * 60);
/// How old the last look at the players may be when the link drops.
const PLAYING_FRESH: Duration = Duration::from_secs(7);
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

/// Everything the media logic asks of the system: the sound server, the
/// players on the session bus and the note on disk.
pub trait Host: Send + Sync {
    fn sink(&self, addr: &str) -> impl Future<Output = Option<SinkInfo>> + Send;
    /// Whether the sink carries audio, `None` without one.
    fn sink_running(&self, addr: &str) -> impl Future<Output = Option<bool>> + Send;
    fn volume(&self, addr: &str) -> impl Future<Output = Option<u8>> + Send;
    fn set_volume_tenths(&self, sink: &SinkInfo, tenths: u16) -> impl Future<Output = bool> + Send;
    /// Level of what plays, before the sink volume; takes `MEASURE`.
    fn measure_db(&self, addr: &str) -> impl Future<Output = Option<f64>> + Send;
    fn players(&self) -> impl Future<Output = Vec<String>> + Send;
    fn status(&self, player: &str) -> impl Future<Output = Option<String>> + Send;
    fn title(&self, player: &str) -> impl Future<Output = Option<String>> + Send;
    fn call(&self, player: &str, method: &str) -> impl Future<Output = Result<(), String>> + Send;
    fn read_note(&self) -> Option<Ducked>;
    fn write_note(&self, d: &Ducked);
    fn clear_note(&self);
}

#[derive(Default)]
pub struct Media<H: Host = System> {
    inner: Mutex<Inner>,
    /// Bumped whenever speech is (still) detected. Kept outside the lock:
    /// a restore holds it while fading up, and speech has to stop that.
    speech: Arc<AtomicU64>,
    host: H,
}

#[derive(Default)]
struct Inner {
    /// Address of the buds. Every audio call needs it; without one the
    /// first Bluetooth sink on the system would be taken for theirs.
    addr: Option<String>,
    /// Players we paused, and how many buds were in ear right before.
    paused: Vec<String>,
    paused_at: Option<Instant>,
    resume_at_count: u8,
    /// `paused` holds what played when the link dropped, not what a bud
    /// coming out paused.
    paused_by_link: bool,
    /// Players heard through the buds at the last poll. By the time the
    /// link is reported down, the sink is gone and some players have
    /// paused themselves.
    last_playing: Option<(Instant, Vec<String>)>,
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

impl<H: Host> Media<H> {
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
        // Left over from a link that dropped with the buds never put back
        // in: it must not stop the next bud that comes out from pausing.
        let since = inner.paused_at.map(|t| t.elapsed());
        let expired = since.is_none_or(|d| d >= LINK_RESUME_WINDOW);
        if inner.paused_by_link && expired {
            inner.paused.clear();
            inner.paused_by_link = false;
        }
        match ear_action(prev, now, !inner.paused.is_empty(), inner.resume_at_count) {
            EarAction::Nothing => return,
            EarAction::Pause => {
                let Some(addr) = inner.addr.clone() else {
                    return;
                };
                if !airpods_are_output(&self.host, &addr).await {
                    return;
                }
                let paused = inner.pause_playing(&self.host).await;
                if !paused.is_empty() {
                    info!(players = ?paused, before, after, "paused on ear removal");
                    inner.paused = paused;
                    inner.paused_by_link = false;
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
        let window = if inner.paused_by_link {
            LINK_RESUME_WINDOW
        } else {
            RESUME_WINDOW
        };
        let fresh = inner.paused_at.is_some_and(|t| t.elapsed() < window);
        inner.paused_by_link = false;
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
        if !wait_for_airpods_output(&self.host, &addr).await {
            warn!("AirPods sink never became the default — not resuming");
            return;
        }
        let mut inner = self.inner.lock().await;
        if inner.ear_gen != my_gen {
            debug!("ears changed while waiting for the sink — not resuming");
            return;
        }
        let resumed = inner.play(&self.host, &players).await;
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
                    if let Some(vol) = self.host.volume(&addr).await
                        && vol > to
                    {
                        fade(&self.host, &addr, vol, to, FADE_TICK, None).await;
                        debug!(from = vol, to, "speech again — back down");
                    }
                    return;
                }
                if inner.ducked_from.is_some() || !inner.music_playing(&self.host).await {
                    return;
                }
                let Some(vol) = self.host.volume(&addr).await else {
                    return;
                };
                // The level takes MEASURE to read; the first, smallest dip
                // runs meanwhile so the duck starts with the speech.
                let first = duck_first(vol);
                let (dipped, level) = tokio::join!(
                    fade(&self.host, &addr, vol, first, FADE_TICK, None),
                    self.host.measure_db(&addr)
                );
                if !dipped {
                    return;
                }
                let Some(level) = level.filter(|l| *l >= SILENCE_DB) else {
                    fade(&self.host, &addr, first, vol, RESTORE_TICK, None).await;
                    return;
                };
                let target = duck_target(vol, level).min(first);
                if target >= vol {
                    return;
                }
                if fade(&self.host, &addr, first, target, FADE_TICK, None).await {
                    info!(
                        from = vol,
                        to = target,
                        level_db = format!("{level:.1}"),
                        "speech detected — volume lowered"
                    );
                    inner.ducked_from = Some(vol);
                    inner.ducked_to = Some(target);
                    inner.spoke_at = Some(Instant::now());
                    self.host.write_note(&Ducked {
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
                    inner.restore_volume(&self.host, &self.speech).await;
                }
            }
        }
    }

    /// Called when the daemon stops so a duck never outlives it.
    pub async fn reset(&self) {
        let mut inner = self.inner.lock().await;
        inner.restore_volume(&self.host, &self.speech).await;
    }

    /// Called when the link drops. Music that was playing through the
    /// buds counts as paused by us, so a bud back in after the reconnect
    /// resumes it like one put back after being taken out.
    pub async fn link_dropped(&self) {
        let mut inner = self.inner.lock().await;
        inner.restore_volume(&self.host, &self.speech).await;
        let Some((at, names)) = inner.last_playing.take() else {
            return;
        };
        if at.elapsed() > PLAYING_FRESH || !inner.paused.is_empty() {
            return;
        }
        info!(players = ?names, "link dropped while playing — resuming once a bud is back in");
        inner.paused = names;
        inner.paused_at = Some(Instant::now());
        inner.paused_by_link = true;
        inner.resume_at_count = 1;
    }

    /// Checked on every poll of the link task, see `last_playing`. A
    /// sink that is already gone says nothing about what played, but one
    /// that is idle does: music paused by hand stays paused.
    pub async fn note_playback(&self) {
        let mut inner = self.inner.lock().await;
        if let Some(names) = inner.playing_players(&self.host).await {
            inner.last_playing = (!names.is_empty()).then(|| (Instant::now(), names));
        }
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
            inner.restore_volume(&self.host, &self.speech).await;
        }
    }

    /// A duck noted on disk belongs to a session that did not get to
    /// undo it. Undone here once the sink is back — unless the volume
    /// has moved since, in which case it is somebody's choice now.
    pub async fn recover_duck(&self) {
        // The sink shows up a moment after the connect edge.
        let Some(note) = self.host.read_note() else {
            return;
        };
        if !wait_for_sink(&self.host, &note.addr).await {
            return;
        }
        let mut inner = self.inner.lock().await;
        if inner.ducked_from.is_some() {
            return;
        }
        let Some(note) = self.host.read_note() else {
            return;
        };
        let Some(now) = self.host.volume(&note.addr).await else {
            return;
        };
        if now.abs_diff(note.to) > 1 {
            debug!(
                now,
                ducked_to = note.to,
                "volume moved since — dropping the note"
            );
            self.host.clear_note();
            return;
        }
        let gen_now = self.speech.load(Ordering::SeqCst);
        let stop = Some((Arc::clone(&self.speech), gen_now));
        if fade(&self.host, &note.addr, now, note.from, RESTORE_TICK, stop).await {
            info!(
                to = note.from,
                "undid a duck left behind by an earlier session"
            );
            self.host.clear_note();
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
    async fn restore_volume<H: Host>(&mut self, host: &H, speech: &Arc<AtomicU64>) {
        let Some(addr) = self.addr.clone() else {
            return;
        };
        let Some(vol) = self.ducked_from else {
            return;
        };
        let Some(from) = host.volume(&addr).await else {
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
            self.forget_duck(host);
            return;
        }
        let gen_now = speech.load(Ordering::SeqCst);
        let stop = Some((Arc::clone(speech), gen_now));
        if fade(host, &addr, from, vol, RESTORE_TICK, stop).await {
            info!(to = vol, "speech ended — volume restored");
            self.forget_duck(host);
        } else if speech.load(Ordering::SeqCst) != gen_now {
            debug!("speech again while restoring");
            self.cut_short = true;
        }
    }

    /// Only after a fade that worked: a sink that went away mid-restore
    /// must stay on the list, or the duck is lost for good.
    fn forget_duck<H: Host>(&mut self, host: &H) {
        self.ducked_from = None;
        self.ducked_to = None;
        self.cut_short = false;
        self.spoke_at = None;
        host.clear_note();
    }

    /// One per track: a mirror plays the same title, and `play` skips a
    /// player that is already back by the time it gets to it.
    /// `None` while the buds have no sink to look at.
    async fn playing_players<H: Host>(&mut self, host: &H) -> Option<Vec<String>> {
        let addr = self.addr.clone()?;
        if !host.sink_running(&addr).await? {
            return Some(Vec::new());
        }
        let mut names = Vec::new();
        let mut tracks: Vec<String> = Vec::new();
        for name in host.players().await {
            if host.status(&name).await.as_deref() != Some("Playing") {
                continue;
            }
            let track = host.title(&name).await;
            if track.as_ref().is_some_and(|t| tracks.contains(t)) {
                continue;
            }
            tracks.extend(track);
            names.push(name);
        }
        Some(names)
    }

    /// Ducking silence only makes the volume jump in the OSD and mixer.
    /// Both must agree: the sink carries audio and a player says Playing.
    async fn music_playing<H: Host>(&mut self, host: &H) -> bool {
        let Some(addr) = self.addr.clone() else {
            return false;
        };
        if host.sink_running(&addr).await != Some(true) {
            return false;
        }
        for name in host.players().await {
            if host.status(&name).await.as_deref() == Some("Playing") {
                return true;
            }
        }
        false
    }

    async fn pause_playing<H: Host>(&mut self, host: &H) -> Vec<String> {
        let mut playing = Vec::new();
        for name in host.players().await {
            if host.status(&name).await.as_deref() == Some("Playing") {
                let track = host.title(&name).await;
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
            if host.status(name).await.as_deref() != Some("Playing") {
                continue;
            }
            if let Err(e) = host.call(name, "Pause").await {
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
                if host.status(other).await.as_deref() != Some("Playing") {
                    done.push(other.clone());
                    tracks.extend(other_track.clone());
                }
            }
        }
        paused
    }

    async fn play<H: Host>(&mut self, host: &H, names: &[String]) -> Vec<String> {
        let mut resumed = Vec::new();
        for name in names {
            // Paused by someone else in the meantime, or already brought
            // back by a player we resumed before it.
            if host.status(name).await.as_deref() != Some("Paused") {
                continue;
            }
            match host.call(name, "Play").await {
                Ok(()) => resumed.push(name.clone()),
                Err(e) => debug!(player = %name, error = %e, "play failed"),
            }
            tokio::time::sleep(SETTLE).await;
        }
        resumed
    }
}

/// The real thing: `pactl` and `pw-record`, MPRIS on the session bus,
/// and the note in `$XDG_STATE_HOME`.
#[derive(Default)]
pub struct System {
    conn: Mutex<Option<Connection>>,
}

impl System {
    async fn connection(&self) -> Option<Connection> {
        let mut conn = self.conn.lock().await;
        if conn.is_none() {
            match Connection::session().await {
                Ok(c) => *conn = Some(c),
                Err(e) => {
                    warn!(error = %e, "session bus unavailable — no media control");
                    return None;
                }
            }
        }
        conn.clone()
    }
}

impl Host for System {
    async fn sink(&self, addr: &str) -> Option<SinkInfo> {
        let addr = addr.to_string();
        tokio::task::spawn_blocking(move || audio::sink_for(&addr))
            .await
            .ok()
            .flatten()
    }

    async fn sink_running(&self, addr: &str) -> Option<bool> {
        let addr = addr.to_string();
        tokio::task::spawn_blocking(move || audio::sink_state_for(&addr))
            .await
            .ok()
            .flatten()
    }

    async fn volume(&self, addr: &str) -> Option<u8> {
        let addr = addr.to_string();
        tokio::task::spawn_blocking(move || audio::snapshot(&addr).volume_percent)
            .await
            .ok()
            .flatten()
    }

    async fn set_volume_tenths(&self, sink: &SinkInfo, tenths: u16) -> bool {
        let sink = sink.clone();
        tokio::task::spawn_blocking(move || audio::set_volume_tenths(&sink, tenths).is_ok())
            .await
            .unwrap_or(false)
    }

    async fn measure_db(&self, addr: &str) -> Option<f64> {
        let addr = addr.to_string();
        tokio::task::spawn_blocking(move || measure_db(&addr))
            .await
            .ok()
            .flatten()
    }

    async fn players(&self) -> Vec<String> {
        match self.connection().await {
            Some(conn) => players(&conn).await,
            None => Vec::new(),
        }
    }

    async fn status(&self, player: &str) -> Option<String> {
        status(&self.connection().await?, player).await
    }

    async fn title(&self, player: &str) -> Option<String> {
        title(&self.connection().await?, player).await
    }

    async fn call(&self, player: &str, method: &str) -> Result<(), String> {
        let conn = self.connection().await.ok_or("no session bus")?;
        call(&conn, player, method).await.map_err(|e| e.to_string())
    }

    fn read_note(&self) -> Option<Ducked> {
        read_duck()
    }

    fn write_note(&self, d: &Ducked) {
        write_duck(d);
    }

    fn clear_note(&self) {
        clear_duck();
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
        .destination(name)
        .path(MPRIS_PATH)
        .interface(MPRIS_PLAYER)
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
    let meta: std::collections::HashMap<String, zbus::OwnedVariant> =
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
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Ducked {
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

async fn airpods_are_output<H: Host>(host: &H, addr: &str) -> bool {
    host.sink(addr).await.is_some_and(|s| s.is_default)
}

async fn wait_for_airpods_output<H: Host>(host: &H, addr: &str) -> bool {
    let deadline = Instant::now() + SINK_WAIT;
    loop {
        if airpods_are_output(host, addr).await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn wait_for_sink<H: Host>(host: &H, addr: &str) -> bool {
    let deadline = Instant::now() + SINK_WAIT;
    loop {
        if host.sink(addr).await.is_some() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
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
fn measure_db(addr: &str) -> Option<f64> {
    let sink = audio::sink_for(addr)?;
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
async fn fade<H: Host>(
    host: &H,
    addr: &str,
    from: u8,
    to: u8,
    tick: Duration,
    stop: Option<(Arc<AtomicU64>, u64)>,
) -> bool {
    let Some(sink) = host.sink(addr).await else {
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
        if !host.set_volume_tenths(&sink, v as u16).await {
            return false;
        }
        tokio::time::sleep(tick).await;
    }
    host.set_volume_tenths(&sink, u16::from(to) * 10).await
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

/// The timing of the media logic against a stand-in for the system, on a
/// paused clock.
#[cfg(test)]
mod timing {
    use super::*;
    use podctl::model::EarStatus::{InCase, InEar as In, OutOfEar};
    use std::sync::Mutex as StdMutex;

    const ADDR: &str = "AA:BB:CC:DD:EE:01";

    #[derive(Default)]
    struct World {
        sink: bool,
        default: bool,
        running: bool,
        tenths: u16,
        level: Option<f64>,
        /// Name, status, title.
        players: Vec<(String, String, Option<String>)>,
        note: Option<Ducked>,
    }

    #[derive(Default)]
    struct Fake(StdMutex<World>);

    impl Fake {
        fn w(&self) -> std::sync::MutexGuard<'_, World> {
            self.0.lock().unwrap()
        }
    }

    impl Host for Fake {
        async fn sink(&self, addr: &str) -> Option<SinkInfo> {
            let w = self.w();
            w.sink.then(|| SinkInfo {
                name: "bluez_output.AA_BB_CC_DD_EE_01.1".into(),
                address: addr.into(),
                id: 1,
                is_default: w.default,
            })
        }

        async fn sink_running(&self, _addr: &str) -> Option<bool> {
            let w = self.w();
            w.sink.then_some(w.running)
        }

        async fn volume(&self, _addr: &str) -> Option<u8> {
            let w = self.w();
            w.sink.then_some(((w.tenths + 5) / 10) as u8)
        }

        async fn set_volume_tenths(&self, _sink: &SinkInfo, tenths: u16) -> bool {
            let mut w = self.w();
            if w.sink {
                w.tenths = tenths;
            }
            w.sink
        }

        async fn measure_db(&self, _addr: &str) -> Option<f64> {
            tokio::time::sleep(MEASURE).await;
            self.w().level
        }

        async fn players(&self) -> Vec<String> {
            self.w().players.iter().map(|p| p.0.clone()).collect()
        }

        async fn status(&self, player: &str) -> Option<String> {
            let w = self.w();
            w.players
                .iter()
                .find(|p| p.0 == player)
                .map(|p| p.1.clone())
        }

        async fn title(&self, player: &str) -> Option<String> {
            let w = self.w();
            w.players.iter().find(|p| p.0 == player)?.2.clone()
        }

        async fn call(&self, player: &str, method: &str) -> Result<(), String> {
            let mut w = self.w();
            let p = w.players.iter_mut().find(|p| p.0 == player).ok_or("gone")?;
            p.1 = if method == "Pause" {
                "Paused"
            } else {
                "Playing"
            }
            .into();
            w.running = method == "Play";
            Ok(())
        }

        fn read_note(&self) -> Option<Ducked> {
            self.w().note.clone()
        }

        fn write_note(&self, d: &Ducked) {
            self.w().note = Some(d.clone());
        }

        fn clear_note(&self) {
            self.w().note = None;
        }
    }

    /// Music at full volume through the buds, one player.
    async fn playing() -> Arc<Media<Fake>> {
        let m = Arc::new(Media::<Fake>::default());
        {
            let mut w = m.host.w();
            w.sink = true;
            w.default = true;
            w.running = true;
            w.tenths = 1000;
            w.level = Some(-35.0);
            w.players.push((
                "org.mpris.MediaPlayer2.music".into(),
                "Playing".into(),
                Some("Song".into()),
            ));
        }
        m.set_address(Some(ADDR.into())).await;
        m
    }

    fn vol(m: &Media<Fake>) -> u16 {
        (m.host.w().tenths + 5) / 10
    }

    fn status(m: &Media<Fake>) -> String {
        m.host.w().players[0].1.clone()
    }

    async fn after(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    fn speech(m: &Arc<Media<Fake>>, level: u8) {
        let m = Arc::clone(m);
        tokio::spawn(async move { m.on_conv_level(level, true).await });
    }

    fn ears(m: &Arc<Media<Fake>>, prev: InEar, now: InEar) {
        let m = Arc::clone(m);
        tokio::spawn(async move { m.on_in_ear(prev, now, true).await });
    }

    fn ear(primary: podctl::model::EarStatus, secondary: podctl::model::EarStatus) -> InEar {
        InEar { primary, secondary }
    }

    #[tokio::test(start_paused = true)]
    async fn the_duck_starts_with_the_speech_and_comes_back() {
        let m = playing().await;
        speech(&m, 1);
        after(150).await;
        assert!(
            vol(&m) < 100,
            "the first dip runs while the level is measured"
        );
        after(2000).await;
        assert_eq!(vol(&m), 68);
        assert!(m.host.w().note.is_some());
        speech(&m, 7);
        after(RESTORE_HOLD.as_millis() as u64 + 2000).await;
        assert_eq!(vol(&m), 100);
        assert!(m.host.w().note.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn speech_during_the_restore_goes_back_down_and_ends_where_it_was() {
        let m = playing().await;
        speech(&m, 1);
        after(2000).await;
        speech(&m, 7);
        after(RESTORE_HOLD.as_millis() as u64 + 400).await;
        let halfway = vol(&m);
        assert!(halfway > 68 && halfway < 100, "restoring, at {halfway}");
        speech(&m, 1);
        after(1000).await;
        assert_eq!(
            vol(&m),
            68,
            "back down to the duck, not a new one from {halfway}"
        );
        speech(&m, 7);
        after(RESTORE_HOLD.as_millis() as u64 + 2000).await;
        assert_eq!(vol(&m), 100);
    }

    #[tokio::test(start_paused = true)]
    async fn a_volume_moved_during_the_duck_is_left_alone() {
        let m = playing().await;
        speech(&m, 1);
        after(2000).await;
        m.host.w().tenths = 700;
        speech(&m, 7);
        after(RESTORE_HOLD.as_millis() as u64 + 2000).await;
        assert_eq!(vol(&m), 70);
    }

    #[tokio::test(start_paused = true)]
    async fn a_long_sentence_outlasts_the_watchdog() {
        let m = playing().await;
        speech(&m, 1);
        after(2000).await;
        after(18_000).await;
        m.watchdog().await;
        assert_eq!(vol(&m), 68, "20 s of talking without a frame");
        after(12_000).await;
        m.watchdog().await;
        assert_eq!(vol(&m), 100, "no end of speech after 30 s");
    }

    #[tokio::test(start_paused = true)]
    async fn a_duck_survives_a_link_that_takes_the_sink_with_it() {
        let m = playing().await;
        speech(&m, 1);
        after(2000).await;
        m.host.w().sink = false;
        m.link_dropped().await;
        assert!(
            m.host.w().note.is_some(),
            "the note stays for the reconnect"
        );
        // PipeWire brings the sink back at the level it had.
        m.host.w().sink = true;
        m.recover_duck().await;
        assert_eq!(vol(&m), 100);
        assert!(m.host.w().note.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn speech_during_a_recovery_takes_the_duck_over() {
        let m = playing().await;
        {
            let mut w = m.host.w();
            w.tenths = 510;
            w.note = Some(Ducked {
                addr: ADDR.into(),
                from: 100,
                to: 51,
            });
        }
        let r = Arc::clone(&m);
        tokio::spawn(async move { r.recover_duck().await });
        after(500).await;
        assert!(vol(&m) > 51 && vol(&m) < 100);
        speech(&m, 1);
        after(1000).await;
        assert_eq!(vol(&m), 51);
        speech(&m, 7);
        after(RESTORE_HOLD.as_millis() as u64 + 3000).await;
        assert_eq!(vol(&m), 100, "not the level the recovery had reached");
    }

    #[tokio::test(start_paused = true)]
    async fn a_bud_flickering_on_its_way_out_does_not_resume() {
        let m = playing().await;
        let both = ear(In, In);
        let one = ear(In, OutOfEar);
        ears(&m, both, one);
        after(600).await;
        assert_eq!(status(&m), "Paused");
        ears(&m, one, both);
        after(1400).await;
        let flicker = "\"in\" for 1.4 s right after the pause";
        assert_eq!(status(&m), "Paused", "{flicker}");
        ears(&m, both, one);
        after(3000).await;
        assert_eq!(status(&m), "Paused");
        ears(&m, one, both);
        after(RESUME_SETTLE.as_millis() as u64 + 600).await;
        assert_eq!(status(&m), "Playing", "back in later resumes after 0.5 s");
    }

    #[tokio::test(start_paused = true)]
    async fn music_that_stopped_with_the_link_resumes_with_a_bud_in() {
        let m = playing().await;
        m.note_playback().await;
        {
            let mut w = m.host.w();
            w.sink = false;
            w.players[0].1 = "Paused".into();
        }
        // A poll between the sink going and BlueZ reporting the drop.
        m.note_playback().await;
        m.link_dropped().await;
        after(10_000).await;
        m.host.w().sink = true;
        ears(&m, InEar::default(), ear(InCase, InCase));
        after(3000).await;
        assert_eq!(
            status(&m),
            "Paused",
            "reconnecting in the case is not listening"
        );
        ears(&m, ear(InCase, InCase), ear(In, InCase));
        after(3000).await;
        assert_eq!(status(&m), "Playing");
    }

    #[tokio::test(start_paused = true)]
    async fn music_paused_by_hand_stays_paused_across_the_link() {
        let m = playing().await;
        {
            let mut w = m.host.w();
            w.players[0].1 = "Paused".into();
            w.running = false;
        }
        m.note_playback().await;
        m.host.w().sink = false;
        m.link_dropped().await;
        m.host.w().sink = true;
        ears(&m, InEar::default(), ear(In, In));
        after(5000).await;
        assert_eq!(status(&m), "Paused");
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_pause_runs_out_and_leaves_the_next_removal_working() {
        let m = playing().await;
        m.note_playback().await;
        {
            let mut w = m.host.w();
            w.sink = false;
            w.players[0].1 = "Paused".into();
        }
        m.link_dropped().await;
        after(6 * 60 * 1000).await;
        {
            let mut w = m.host.w();
            w.sink = true;
            w.players[0].1 = "Playing".into();
        }
        // Ear detection was off meanwhile: the first change seen is a bud
        // coming out.
        ears(&m, ear(In, In), ear(In, OutOfEar));
        after(1000).await;
        assert_eq!(
            status(&m),
            "Paused",
            "a stale list must not swallow the pause"
        );
    }
}
