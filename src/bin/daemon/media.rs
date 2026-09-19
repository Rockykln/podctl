//! What an iPhone does on the host side: pause on ear removal, resume on
//! return, lower the volume while the wearer is talking.

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
/// The in-ear sensor flickers while a bud is pulled out (out, in, out
/// within half a second). Resume only once the buds have stayed in.
const RESUME_SETTLE: Duration = Duration::from_secs(1);
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

#[derive(Default)]
pub struct Media {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    conn: Option<Connection>,
    /// Players we paused, and how many buds were in ear right before.
    paused: Vec<String>,
    paused_at: Option<Instant>,
    resume_at_count: u8,
    /// Volume to restore once speech ends.
    ducked_from: Option<u8>,
    /// Bumped on every in-ear change so a pending resume can tell it
    /// was overtaken.
    ear_gen: u64,
    /// Bumped whenever speech is (still) detected.
    speech_gen: u64,
}

impl Media {
    pub async fn on_in_ear(&self, prev: InEar, now: InEar, enabled: bool) {
        let before = prev.count_in_ear();
        let after = now.count_in_ear();
        if !enabled || before == after {
            return;
        }
        let mut inner = self.inner.lock().await;
        inner.ear_gen += 1;
        let my_gen = inner.ear_gen;
        if after < before {
            // Second bud out after the first: already paused, and the
            // resume target stays what it was before the first removal.
            if !inner.paused.is_empty() || !airpods_are_output().await {
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
        if inner.paused.is_empty() || after < inner.resume_at_count {
            return;
        }
        drop(inner);
        tokio::time::sleep(RESUME_SETTLE).await;
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
        if !wait_for_airpods_output().await {
            warn!("AirPods sink never became the default — not resuming");
            return;
        }
        let resumed = inner.play(&players).await;
        info!(players = ?resumed, "resumed after buds went back in");
    }

    /// `level` is the last byte of an AAP 0x4B frame: 1–3 while speech
    /// starts and continues, 4 and up as it fades and ends.
    pub async fn on_conv_level(&self, level: u8, enabled: bool) {
        let mut inner = self.inner.lock().await;
        match level {
            1..=3 if enabled => {
                // Cancels a restore still waiting out the pause between sentences.
                inner.speech_gen += 1;
                if inner.ducked_from.is_some() || !inner.music_playing().await {
                    return;
                }
                let Some(vol) = current_volume().await else {
                    return;
                };
                let Some(level) = measure_db().await else {
                    return;
                };
                if level < SILENCE_DB {
                    return;
                }
                let target = duck_target(vol, level);
                if target >= vol {
                    return;
                }
                if fade(vol, target, FADE_TICK).await {
                    info!(
                        from = vol,
                        to = target,
                        level_db = format!("{level:.1}"),
                        "speech detected — volume lowered"
                    );
                    inner.ducked_from = Some(vol);
                }
            }
            1..=3 => {}
            _ => {
                if inner.ducked_from.is_none() {
                    return;
                }
                let my_gen = inner.speech_gen;
                drop(inner);
                tokio::time::sleep(RESTORE_HOLD).await;
                let mut inner = self.inner.lock().await;
                if inner.speech_gen == my_gen {
                    inner.restore_volume().await;
                }
            }
        }
    }

    /// Called when the link drops so a duck never outlives the session.
    pub async fn reset(&self) {
        self.inner.lock().await.restore_volume().await;
    }
}

impl Inner {
    async fn restore_volume(&mut self) {
        let Some(vol) = self.ducked_from.take() else {
            return;
        };
        let from = current_volume().await.unwrap_or(vol);
        if fade(from, vol, RESTORE_TICK).await {
            info!(to = vol, "speech ended — volume restored");
        }
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
        let running = tokio::task::spawn_blocking(audio::primary_sink_running)
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

async fn airpods_are_output() -> bool {
    tokio::task::spawn_blocking(|| audio::primary_sink().is_some_and(|s| s.is_default))
        .await
        .unwrap_or(false)
}

async fn wait_for_airpods_output() -> bool {
    let deadline = Instant::now() + SINK_WAIT;
    loop {
        if airpods_are_output().await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn current_volume() -> Option<u8> {
    tokio::task::spawn_blocking(|| audio::snapshot().volume_percent)
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

/// RMS level of what is playing to the AirPods, before the sink volume.
/// Captured with `pw-record` on the sink itself: PipeWire's pulse
/// `.monitor` source hands `parec` silence for Bluetooth sinks.
async fn measure_db() -> Option<f64> {
    tokio::task::spawn_blocking(|| {
        let sink = audio::primary_sink()?;
        let mut child = std::process::Command::new("pw-record")
            .args(["--target", &sink.name, "-P", "{ stream.capture.sink=true }"])
            .args(["--format", "s16", "--rate", "44100", "--channels", "2", "-"])
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

async fn fade(from: u8, to: u8, tick: Duration) -> bool {
    tokio::task::spawn_blocking(move || {
        let Some(sink) = audio::primary_sink() else {
            return false;
        };
        let (mut v, end) = (i32::from(from) * 10, i32::from(to) * 10);
        let step = if end < v {
            -FADE_STEP_TENTHS
        } else {
            FADE_STEP_TENTHS
        };
        while (end - v).abs() > FADE_STEP_TENTHS {
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
    use super::{BLOCK_SAMPLES, duck_target, loud_db};

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
}
