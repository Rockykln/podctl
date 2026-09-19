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
/// Proxies (browser integration, scrobblers) mirror another player's
/// track, and some turn Pause/Play into a toggle — calling both would
/// pause and unpause the same song. Only one player per track is touched,
/// after a short wait for the mirrors to catch up.
const SETTLE: Duration = Duration::from_millis(400);
/// Speech ducks to this share of the current volume. PipeWire's percent
/// scale is cubic, so 75 % is roughly -7.5 dB: clearly quieter, not muted.
const DUCK_PERCENT: u32 = 75;
/// Volume changes are faded in 2.5-point steps, not jumped.
const FADE_STEP_TENTHS: i32 = 25;
const FADE_TICK: Duration = Duration::from_millis(25);

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
}

impl Media {
    pub async fn on_in_ear(&self, prev: InEar, now: InEar, enabled: bool) {
        let before = prev.count_in_ear();
        let after = now.count_in_ear();
        if !enabled || before == after {
            return;
        }
        let mut inner = self.inner.lock().await;
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
                if inner.ducked_from.is_some() {
                    return;
                }
                let Some(vol) = current_volume().await else {
                    return;
                };
                let target = (u32::from(vol) * DUCK_PERCENT / 100) as u8;
                if target >= vol {
                    return;
                }
                if fade(vol, target).await {
                    info!(from = vol, to = target, "speech detected — volume lowered");
                    inner.ducked_from = Some(vol);
                }
            }
            1..=3 => {}
            _ => inner.restore_volume().await,
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
        if fade(from, vol).await {
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

async fn fade(from: u8, to: u8) -> bool {
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
            std::thread::sleep(FADE_TICK);
        }
        audio::set_volume(&sink, to).is_ok()
    })
    .await
    .unwrap_or(false)
}
