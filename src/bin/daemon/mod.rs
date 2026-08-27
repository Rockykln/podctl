//! Daemon core. Owns the live `DeviceState`, the IPC server, and the
//! BT-link task. Most of the per-request dispatch happens here; the
//! actual BlueZ + L2CAP + PipeWire calls live in the sibling modules.

pub mod aap;
mod link;
mod server;

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{RwLock, broadcast};
use tracing::debug;

use podctl::{
    Capabilities, ConvAwareness, DeviceState, Event, Mode, PressSide, Request, Response,
    model::PressAction,
};

const EVENT_BUS_CAPACITY: usize = 64;

/// Minimum gap between two re-subscribe requests. The link watchdog and
/// every incoming `status` / `battery` may ask; the device only needs
/// one nudge.
const RESUBSCRIBE_COOLDOWN: Duration = Duration::from_secs(5);
/// How long a `status` / `battery` request waits for the dump it just
/// asked for. Long enough for a healthy link, short enough that the CLI
/// still feels instant.
const BATTERY_WAIT: Duration = Duration::from_millis(900);

const NOT_IMPL: &str =
    "not implemented for this device — no verified AAP packet for this setting yet";

pub struct Daemon {
    state: RwLock<DeviceState>,
    events: broadcast::Sender<Event>,
    aap_stream: RwLock<Option<std::sync::Arc<podctl::l2cap::L2capStream>>>,
    last_resubscribe: Mutex<Option<Instant>>,
}

impl Daemon {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(EVENT_BUS_CAPACITY);
        Self {
            state: RwLock::new(DeviceState::default()),
            events: tx,
            aap_stream: RwLock::new(None),
            last_resubscribe: Mutex::new(None),
        }
    }

    /// Called by the AAP runtime once the socket is open and the
    /// handshake/subscribe sequence has been written. Stored here so
    /// CLI-driven setters can write outgoing frames.
    pub async fn set_aap_stream(&self, stream: Option<std::sync::Arc<podctl::l2cap::L2capStream>>) {
        let linked = stream.is_some();
        *self.aap_stream.write().await = stream;
        let mut s = self.state.write().await;
        s.aap_linked = linked;
        Self::touch(&mut s);
    }

    /// Send a raw AAP frame to the device. Returns an error if the AAP
    /// socket isn't open (e.g. AirPods are disconnected).
    pub async fn aap_send(&self, frame: &[u8]) -> Result<(), String> {
        let guard = self.aap_stream.read().await;
        let Some(stream) = guard.as_ref() else {
            return Err("AAP socket not connected".into());
        };
        stream.send(frame).map_err(|e| format!("AAP write: {e}"))
    }

    /// Ask the AirPods to re-send their whole state dump.
    ///
    /// The device pushes battery and settings on change plus exactly once
    /// after a subscribe. Miss that one dump — a frame lost while the
    /// device was still setting up its profiles — and the daemon shows an
    /// empty battery for the rest of the session. Re-sending the same
    /// feature-flags + subscribe pair is the only way to ask again; the
    /// device answers with a fresh dump and the frames are idempotent.
    ///
    /// Unthrottled — the link watchdog paces its own calls. Request
    /// handlers go through `aap_resubscribe_throttled` instead.
    pub async fn aap_resubscribe(&self) -> Result<(), String> {
        self.aap_send(aap::SET_FEATURES_PRO2).await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        self.aap_send(aap::SUBSCRIBE_NOTIFICATIONS).await
    }

    /// `aap_resubscribe` behind the cooldown. Returns whether it ran.
    async fn aap_resubscribe_throttled(&self) -> bool {
        {
            let mut last = match self.last_resubscribe.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if last.is_some_and(|t| t.elapsed() < RESUBSCRIBE_COOLDOWN) {
                return false;
            }
            *last = Some(Instant::now());
        }
        match self.aap_resubscribe().await {
            Ok(()) => true,
            Err(e) => {
                debug!(error = %e, "re-subscribe failed");
                false
            }
        }
    }

    /// Snapshot for `status` / `battery`. When the link is up but no
    /// battery level has ever arrived, nudge the device once and give it
    /// a moment to answer rather than handing back three em-dashes.
    async fn snapshot_fresh(&self) -> DeviceState {
        let snap = self.snapshot().await;
        if !snap.connected || snap.battery.any_known() {
            return snap;
        }
        if !self.aap_resubscribe_throttled().await {
            return snap;
        }
        let deadline = Instant::now() + BATTERY_WAIT;
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if self.state.read().await.battery.any_known() {
                break;
            }
        }
        self.snapshot().await
    }

    /// Send an AAP `09 00 [id] [val] 00 00 00` setting-write frame.
    /// Send a `0x09` setting frame, then reflect it in cached state
    /// optimistically (the device only re-reports these in the post-
    /// subscribe dump, not on change — same class as the set_conv bug).
    async fn aap_setting(
        &self,
        id: u8,
        value: u8,
        apply: impl FnOnce(&mut podctl::PodSettings),
    ) -> Response {
        let frame = podctl::aap::write_setting(id, value).encode();
        if let Err(e) = self.aap_send(&frame).await {
            return Response::err(e);
        }
        {
            let mut s = self.state.write().await;
            apply(&mut s.settings);
            Self::touch(&mut s);
        }
        let _ = self.events.send(Event::SettingsChanged);
        Response::ok_done()
    }

    async fn set_one_bud_anc(&self, on: bool) -> Response {
        self.aap_setting(
            podctl::aap::setting::ONE_BUD_ANC,
            if on { 1 } else { 2 },
            |s| {
                s.one_bud_anc = Some(on);
            },
        )
        .await
    }

    async fn set_chime_volume(&self, level: u8) -> Response {
        self.aap_setting(podctl::aap::setting::CHIME_VOLUME, level, |s| {
            s.chime_volume = Some(level);
        })
        .await
    }

    async fn set_auto_anc_level(&self, level: u8) -> Response {
        self.aap_setting(podctl::aap::setting::AUTO_ANC_LEVEL, level, |s| {
            s.auto_anc_level = Some(level);
        })
        .await
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub async fn snapshot(&self) -> DeviceState {
        self.state.read().await.clone()
    }

    /// Apply `f` to the cached battery and return the updated value.
    /// Touches `updated_at`. Used by the AAP loop.
    pub async fn update_battery<F: FnOnce(&mut podctl::Battery)>(&self, f: F) -> podctl::Battery {
        let mut s = self.state.write().await;
        f(&mut s.battery);
        Self::touch(&mut s);
        s.battery
    }

    pub async fn set_in_ear(&self, in_ear: podctl::InEar) {
        let mut s = self.state.write().await;
        s.in_ear = in_ear;
        Self::touch(&mut s);
    }

    /// Record lid state. Returns `Some(open)` only on an edge so callers
    /// emit one event per transition. The trigger itself is a heuristic
    /// (case-battery visibility) until a real lid opcode is captured.
    pub async fn update_case_lid(&self, open: bool) -> Option<bool> {
        let mut s = self.state.write().await;
        if s.case_lid_open == Some(open) {
            return None;
        }
        s.case_lid_open = Some(open);
        Self::touch(&mut s);
        Some(open)
    }

    /// Apply `f` to the cached settings; touches the timestamp and returns
    /// true only if the closure reports an actual change. Used to suppress
    /// duplicate echo notifications from the device.
    pub async fn update_settings_if_changed<F: FnOnce(&mut podctl::PodSettings) -> bool>(
        &self,
        f: F,
    ) -> bool {
        let mut s = self.state.write().await;
        let changed = f(&mut s.settings);
        if changed {
            Self::touch(&mut s);
        }
        changed
    }

    pub fn broadcast_event(&self, ev: Event) {
        let _ = self.events.send(ev);
    }

    /// Run the BT-link loop. Today: a mock heartbeat so the rest of the
    /// stack has something to look at. Real implementation will scan
    /// BlueZ for paired AirPods, open the AAP channel and pump frames.
    pub async fn link_loop(self: Arc<Self>) {
        link::run(self).await;
    }

    /// Listen on the per-user Unix socket and handle CLI requests.
    pub async fn serve(self: Arc<Self>) -> anyhow::Result<()> {
        server::serve(self).await
    }

    /// Bump `updated_at` to now. Called by every state mutation so the CLI
    /// can show staleness.
    fn touch(state: &mut DeviceState) {
        state.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
    }

    /// Top-level request handler. Returns a Response for short-lived
    /// (one-shot) requests; `Watch` is handled separately in the server
    /// because it's a long-lived stream.
    pub async fn handle(&self, req: Request) -> Response {
        debug!(?req, "request");
        match req {
            Request::Status => Response::ok_state(self.snapshot_fresh().await),
            Request::Battery => Response::ok_state(self.snapshot_fresh().await),
            Request::Ping => Response::ok_pong(),

            Request::SetMode { mode } => self.set_mode(mode).await,
            Request::SetConv { conv } => self.set_conv(conv).await,
            Request::SetSpatial { mode } => self.set_spatial(mode).await,

            Request::SetEarDetection { on } => self.set_ear_detection(on).await,
            Request::SetMic { mic } => self.set_mic(mic).await,
            Request::SetLoudReduction { on } => self.set_loud(on).await,
            Request::SetPressAction { side, action } => self.set_press(side, action).await,
            Request::SetToneOnPress { on } => self.set_tone(on).await,
            Request::Rename { name } => self.rename(name).await,

            Request::Connect => link::bt_connect(self).await,
            Request::Disconnect => link::bt_disconnect(self).await,
            Request::Pair => link::bt_pair(self).await,
            Request::Unpair => link::bt_unpair(self).await,
            Request::List => link::bt_list().await,
            Request::SetAutoConnect { on } => link::bt_set_trusted(self, on).await,

            Request::SetVolume { percent } => audio_set_volume(self, percent).await,
            Request::SetMuted { muted } => audio_set_muted(self, muted).await,
            Request::SetProfile { profile } => audio_set_profile(self, profile).await,
            Request::SetCodec { codec } => audio_set_codec(self, codec).await,
            Request::MakeDefaultSink => audio_make_default(self).await,
            Request::SetLatencyOffset { ms } => audio_set_latency(self, ms).await,

            Request::SetOneBudAnc { on } => self.set_one_bud_anc(on).await,
            Request::SetChimeVolume { level } => self.set_chime_volume(level).await,
            Request::SetAutoAncLevel { level } => self.set_auto_anc_level(level).await,

            Request::DebugEmitCaseLid { open } => {
                self.state.write().await.case_lid_open = Some(open);
                self.broadcast_event(Event::CaseLid { open });
                Response::ok_done()
            }
            Request::ShowPopup => {
                self.broadcast_event(Event::ShowPopup);
                Response::ok_done()
            }

            Request::Watch => Response::err("watch is a streaming request — handled separately"),
        }
    }

    #[allow(clippy::result_large_err)]
    fn require_caps(
        caps: &Capabilities,
        want: impl Fn(&Capabilities) -> bool,
        feature: &str,
    ) -> Result<(), Response> {
        if !want(caps) {
            return Err(Response::err(format!(
                "{feature} is not supported on {}",
                caps.model.label()
            )));
        }
        Ok(())
    }

    async fn set_mode(&self, mode: Mode) -> Response {
        let caps = self.state.read().await.capabilities;
        let (supported, code) = match mode {
            Mode::Off => (caps.has_anc || caps.has_transparency, 1u8),
            Mode::NoiseCancellation => (caps.has_anc, 2),
            Mode::Transparency => (caps.has_transparency, 3),
            Mode::Adaptive => (caps.has_adaptive, 4),
        };
        if !supported {
            return Response::err(format!("{mode} is not supported on {}", caps.model.label()));
        }
        let frame = podctl::aap::set_noise_control(code).encode();
        if let Err(e) = self.aap_send(&frame).await {
            return Response::err(e);
        }
        // The device's own settings notification (0x09 0d) will land via
        // the AAP loop and update state.settings.mode + emit Event::Mode.
        Response::ok_done()
    }

    async fn set_conv(&self, conv: ConvAwareness) -> Response {
        if let Err(r) = Self::require_caps(
            &self.state.read().await.capabilities,
            |c| c.has_conv_awareness,
            "conversation awareness",
        ) {
            return r;
        }
        let frame = podctl::aap::set_conv_awareness(conv == ConvAwareness::On).encode();
        if let Err(e) = self.aap_send(&frame).await {
            return Response::err(e);
        }
        // Unlike noise-control, the AirPods send no live 0x09/0x28 echo
        // when Conversational Awareness is toggled by command — the new
        // value only arrives in the settings dump after a reconnect. So
        // reflect it optimistically; a later echo is idempotent here.
        let changed = self
            .update_settings_if_changed(|s| {
                let prev = s.conv_awareness;
                s.conv_awareness = Some(conv);
                prev != Some(conv)
            })
            .await;
        if changed {
            self.broadcast_event(Event::ConvAwareness(conv));
        }
        Response::ok_done()
    }

    async fn set_spatial(&self, mode: podctl::SpatialAudio) -> Response {
        if let Err(r) = Self::require_caps(
            &self.state.read().await.capabilities,
            |c| c.has_spatial_audio,
            "spatial audio",
        ) {
            return r;
        }
        let _ = mode;
        Response::err(NOT_IMPL)
    }

    async fn set_ear_detection(&self, on: bool) -> Response {
        if let Err(r) = Self::require_caps(
            &self.state.read().await.capabilities,
            |c| c.has_ear_detection_setting,
            "ear detection",
        ) {
            return r;
        }
        let frame = podctl::aap::set_ear_detection(on).encode();
        if let Err(e) = self.aap_send(&frame).await {
            return Response::err(e);
        }
        // Echo-only would mean `podctl status` lags until reconnect (same
        // class as the old set_conv bug) — reflect optimistically.
        {
            let mut s = self.state.write().await;
            s.settings.ear_detection = Some(on);
            Self::touch(&mut s);
        }
        let _ = self.events.send(Event::SettingsChanged);
        Response::ok_done()
    }

    async fn set_mic(&self, mic: podctl::MicSelection) -> Response {
        if let Err(r) = Self::require_caps(
            &self.state.read().await.capabilities,
            |c| c.has_mic_selection,
            "mic selection",
        ) {
            return r;
        }
        let code = match mic {
            podctl::MicSelection::Auto => 0u8,
            podctl::MicSelection::AlwaysRight => 1,
            podctl::MicSelection::AlwaysLeft => 2,
        };
        let frame = podctl::aap::set_mic_mode(code).encode();
        if let Err(e) = self.aap_send(&frame).await {
            return Response::err(e);
        }
        {
            let mut s = self.state.write().await;
            s.settings.mic_selection = Some(mic);
            Self::touch(&mut s);
        }
        let _ = self.events.send(Event::SettingsChanged);
        Response::ok_done()
    }

    async fn set_loud(&self, on: bool) -> Response {
        if let Err(r) = Self::require_caps(
            &self.state.read().await.capabilities,
            |c| c.has_loud_sound_reduction,
            "loud-sound reduction",
        ) {
            return r;
        }
        let _ = on;
        Response::err(NOT_IMPL)
    }

    async fn set_press(&self, side: PressSide, action: PressAction) -> Response {
        if let Err(r) = Self::require_caps(
            &self.state.read().await.capabilities,
            |c| c.has_press_and_hold,
            "press-and-hold mapping",
        ) {
            return r;
        }
        let _ = (side, action);
        Response::err(NOT_IMPL)
    }

    async fn set_tone(&self, on: bool) -> Response {
        if let Err(r) = Self::require_caps(
            &self.state.read().await.capabilities,
            |c| c.has_tone_on_press,
            "press tone",
        ) {
            return r;
        }
        let _ = on;
        Response::err(NOT_IMPL)
    }

    async fn rename(&self, name: String) -> Response {
        if name.trim().is_empty() {
            return Response::err("name cannot be empty");
        }
        if name.len() > 64 {
            return Response::err("name too long (max 64 bytes UTF-8)");
        }
        let addr = {
            let state = self.state.read().await;
            state.address.clone()
        };
        let Some(addr) = addr else {
            return Response::err("no AirPods address known yet — try 'podctl status' first");
        };
        let name_for_task = name.clone();
        let res =
            tokio::task::spawn_blocking(move || podctl::bluez::set_alias(&addr, &name_for_task))
                .await;
        match res {
            Ok(Ok(())) => {
                let mut state = self.state.write().await;
                state.settings.custom_name = Some(name.clone());
                state.name = Some(name);
                Self::touch(&mut state);
                Response::ok_done()
            }
            Ok(Err(e)) => Response::err(format!("{e}")),
            Err(e) => Response::err(format!("rename task: {e}")),
        }
    }
}

// Audio handlers wrap the sync `podctl::audio` helpers on a blocking task
// and update cached state on success.

async fn audio_set_volume(d: &Daemon, percent: u8) -> Response {
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let sink = podctl::audio::primary_sink()
            .ok_or_else(|| anyhow::anyhow!("no AirPods audio sink found"))?;
        podctl::audio::set_volume(&sink, percent)
    })
    .await;
    match res {
        Ok(Ok(())) => {
            let mut s = d.state.write().await;
            s.audio.volume_percent = Some(percent);
            Daemon::touch(&mut s);
            Response::ok_done()
        }
        Ok(Err(e)) => Response::err(format!("{e}")),
        Err(e) => Response::err(format!("audio task: {e}")),
    }
}

async fn audio_set_muted(d: &Daemon, muted: bool) -> Response {
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let sink = podctl::audio::primary_sink()
            .ok_or_else(|| anyhow::anyhow!("no AirPods audio sink found"))?;
        podctl::audio::set_muted(&sink, muted)
    })
    .await;
    match res {
        Ok(Ok(())) => {
            let mut s = d.state.write().await;
            s.audio.muted = muted;
            Daemon::touch(&mut s);
            Response::ok_done()
        }
        Ok(Err(e)) => Response::err(format!("{e}")),
        Err(e) => Response::err(format!("audio task: {e}")),
    }
}

async fn audio_set_profile(d: &Daemon, profile: podctl::Profile) -> Response {
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let card = podctl::audio::primary_card()
            .ok_or_else(|| anyhow::anyhow!("no AirPods card found"))?;
        podctl::audio::set_profile(&card, profile)
    })
    .await;
    match res {
        Ok(Ok(())) => {
            let mut s = d.state.write().await;
            s.audio.profile = Some(profile);
            Daemon::touch(&mut s);
            Response::ok_done()
        }
        Ok(Err(e)) => Response::err(format!("{e}")),
        Err(e) => Response::err(format!("audio task: {e}")),
    }
}

async fn audio_set_codec(d: &Daemon, codec: String) -> Response {
    let codec_for_task = codec.clone();
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let card = podctl::audio::primary_card()
            .ok_or_else(|| anyhow::anyhow!("no AirPods card found"))?;
        podctl::audio::set_codec(&card, &codec_for_task)
    })
    .await;
    match res {
        Ok(Ok(())) => {
            let mut s = d.state.write().await;
            s.audio.codec = Some(codec);
            Daemon::touch(&mut s);
            Response::ok_done()
        }
        Ok(Err(e)) => Response::err(format!("{e}")),
        Err(e) => Response::err(format!("audio task: {e}")),
    }
}

async fn audio_make_default(d: &Daemon) -> Response {
    let res = tokio::task::spawn_blocking(|| -> anyhow::Result<()> {
        let sink = podctl::audio::primary_sink()
            .ok_or_else(|| anyhow::anyhow!("no AirPods audio sink found"))?;
        podctl::audio::set_default(&sink)
    })
    .await;
    match res {
        Ok(Ok(())) => {
            let mut s = d.state.write().await;
            s.audio.is_default_sink = true;
            Daemon::touch(&mut s);
            Response::ok_done()
        }
        Ok(Err(e)) => Response::err(format!("{e}")),
        Err(e) => Response::err(format!("audio task: {e}")),
    }
}

async fn audio_set_latency(d: &Daemon, ms: i32) -> Response {
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let card = podctl::audio::primary_card()
            .ok_or_else(|| anyhow::anyhow!("no AirPods card found"))?;
        podctl::audio::set_latency_offset(&card, ms)
    })
    .await;
    match res {
        Ok(Ok(())) => {
            let mut s = d.state.write().await;
            s.audio.latency_offset_ms = ms;
            Daemon::touch(&mut s);
            Response::ok_done()
        }
        Ok(Err(e)) => Response::err(format!("{e}")),
        Err(e) => Response::err(format!("audio task: {e}")),
    }
}
