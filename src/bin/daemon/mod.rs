//! Daemon core. Owns the live `DeviceState`, the IPC server, and the
//! BT-link task. Most of the per-request dispatch happens here; the
//! actual BlueZ + L2CAP + PipeWire calls live in the sibling modules.

pub mod aap;
mod battery;
mod link;
mod media;
mod scan;
mod server;

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{RwLock, broadcast};
use tracing::{debug, info};

use podctl::{
    Capabilities, ConvAwareness, DeviceState, Event, Mode, PressSide, Request, Response,
    ipc::ErrorCode, model::PressAction,
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
/// How long a request waits for the link task's first BlueZ read. One
/// `bluetoothctl` call takes a few hundred milliseconds; past that the
/// honest answer is better than more waiting.
const FIRST_POLL_WAIT: Duration = Duration::from_millis(1500);
/// How long the buds get to echo a listening mode back before the
/// command says they did not take it. Measured on AirPods Pro 3 in the
/// ear: the echo lands after about 1.1 s, so this leaves room for a
/// weaker link without ever calling a mode that did arrive a failure.
const MODE_ECHO_WAIT: Duration = Duration::from_secs(3);

const NO_POPUP: &str =
    "no popup process is listening — start it with 'systemctl --user enable --now podctl-popup'";

const NOT_IMPL: &str =
    "not implemented for this device — no verified AAP packet for this setting yet";

/// Drops a popup registration when its watch connection ends.
pub struct PopupGuard(Arc<Daemon>);

impl Drop for PopupGuard {
    fn drop(&mut self) {
        self.0.popup_listeners.fetch_sub(1, Ordering::Relaxed);
    }
}

pub struct Daemon {
    state: RwLock<DeviceState>,
    events: broadcast::Sender<Event>,
    aap_stream: RwLock<Option<std::sync::Arc<podctl::l2cap::L2capStream>>>,
    last_resubscribe: Mutex<Option<Instant>>,
    /// Watch connections that declared `WatchRole::Popup`, i.e. processes
    /// that will actually draw a bubble for `Event::ShowPopup`.
    popup_listeners: AtomicUsize,
    media: media::Media,
    /// Rung by the AAP thread when it exits, so the link task looks at
    /// BlueZ right away instead of at its next poll.
    link_wake: tokio::sync::Notify,
    /// Bumped when the AirPods turn up without connecting, which no event
    /// says: watchers then get the whole state again.
    resync: tokio::sync::watch::Sender<u64>,
    /// False until the link task has read BlueZ once. A `status` served
    /// before that would call a paired pair unpaired.
    polled: tokio::sync::watch::Sender<bool>,
    /// Counts the settings notifications the buds send. `set_mode` has
    /// to wait for a *new* one: with only the value to go by, asking
    /// for the mode the state already held answered "ok" without the
    /// buds having said anything at all.
    settings_echo: tokio::sync::watch::Sender<u64>,
}

impl Daemon {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(EVENT_BUS_CAPACITY);
        Self {
            // Ear detection and the mic mode are ours to remember: the
            // Pro 2 keep them out of their settings dump, so `status`
            // would show nothing until podctl had set them itself.
            state: RwLock::new(DeviceState {
                settings: podctl::config::load().pod_settings(),
                ..DeviceState::default()
            }),
            events: tx,
            aap_stream: RwLock::new(None),
            last_resubscribe: Mutex::new(None),
            popup_listeners: AtomicUsize::new(0),
            media: media::Media::default(),
            link_wake: tokio::sync::Notify::new(),
            resync: tokio::sync::watch::Sender::new(0),
            polled: tokio::sync::watch::Sender::new(false),
            settings_echo: tokio::sync::watch::Sender::new(0),
        }
    }

    /// Register a popup watcher for as long as the returned guard lives.
    pub fn attach_popup(self: &Arc<Self>) -> PopupGuard {
        self.popup_listeners.fetch_add(1, Ordering::Relaxed);
        PopupGuard(Arc::clone(self))
    }

    fn popup_attached(&self) -> bool {
        self.popup_listeners.load(Ordering::Relaxed) > 0
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
        // The write goes out on a blocking thread, and the lock is let
        // go of first: a full send window — buds out of range, the link
        // on its way out — used to hold a runtime worker and the read
        // guard at once, so `set_aap_stream` waited for the write lock
        // and the whole daemon stood still.
        let stream = {
            let guard = self.aap_stream.read().await;
            match guard.as_ref() {
                Some(s) => Arc::clone(s),
                None => return Err("AAP socket not connected".into()),
            }
        };
        let frame = frame.to_vec();
        match tokio::task::spawn_blocking(move || stream.send(&frame)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(format!("AAP write: {e}")),
            Err(e) => Err(format!("AAP write task: {e}")),
        }
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

    /// A settings notification arrived from the buds.
    pub fn note_settings_echo(&self) {
        self.settings_echo.send_modify(|n| *n += 1);
    }

    /// The link task has read BlueZ at least once.
    pub fn mark_polled(&self) {
        self.polled
            .send_if_modified(|seen| !std::mem::replace(seen, true));
    }

    /// Waits for that first read, at most `FIRST_POLL_WAIT`. Without it
    /// a `status` in the first moments after a restart reports "no
    /// AirPods paired" — the state is simply still empty.
    async fn wait_for_first_poll(&self) {
        let mut rx = self.polled.subscribe();
        let _ = tokio::time::timeout(FIRST_POLL_WAIT, async {
            while !*rx.borrow_and_update() {
                if rx.changed().await.is_err() {
                    return;
                }
            }
        })
        .await;
    }

    /// Snapshot for `status` / `battery`. When the link is up but no
    /// battery level has ever arrived, nudge the device once and give it
    /// a moment to answer rather than handing back three em-dashes.
    async fn snapshot_fresh(&self) -> DeviceState {
        self.wait_for_first_poll().await;
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
            podctl::aap::wire::switch_byte(on),
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

    pub fn subscribe_resync(&self) -> tokio::sync::watch::Receiver<u64> {
        self.resync.subscribe()
    }

    pub fn resync(&self) {
        self.resync.send_modify(|n| *n += 1);
    }

    pub async fn snapshot(&self) -> DeviceState {
        self.state.read().await.clone()
    }

    /// Apply `f` to the cached battery and return the updated value.
    /// Touches `updated_at`. Used by the AAP loop.
    pub async fn update_battery<F: FnOnce(&mut podctl::Battery)>(&self, f: F) -> podctl::Battery {
        let mut s = self.state.write().await;
        f(&mut s.battery);
        if s.battery.any_known() {
            s.battery_at = Some(link::now_secs());
        }
        Self::touch(&mut s);
        s.battery
    }

    /// Learn from a reported setting while the model is unknown. A
    /// pair podctl has no table row for would otherwise be refused
    /// every command it can in fact do.
    pub async fn learn_capability(&self, id: u8, value: u8) {
        let mut s = self.state.write().await;
        if s.capabilities.model != podctl::caps::Model::Unknown {
            return;
        }
        if s.capabilities.learn_from_setting(id, value) {
            info!(
                setting = format!("{id:02x}"),
                "unknown model: learned a capability from its own settings"
            );
            Self::touch(&mut s);
            drop(s);
            // Capabilities ride in the state, not in any event, so
            // without this the tray keeps its greyed-out entries until
            // the next reconnect.
            self.resync();
        }
    }

    /// Same, for what the ear-detection frame proves by itself.
    pub async fn learn_in_ear_capability(&self) {
        let mut s = self.state.write().await;
        if s.capabilities.model != podctl::caps::Model::Unknown {
            return;
        }
        if s.capabilities.learn_in_ear() {
            info!("unknown model: it reports in-ear state, so it has the detection");
            Self::touch(&mut s);
            drop(s);
            self.resync();
        }
    }

    pub async fn add_model_number(&self, number: String) {
        let mut s = self.state.write().await;
        if !s.info.model_numbers.contains(&number) {
            s.info.model_numbers.push(number);
            s.info.model_numbers.sort();
            // Two models can share a product code — the AirPods 5 and
            // the AirPods 5 with the wireless case do. The A-number is
            // the only thing that tells them apart, and it arrives here.
            let refined = s.capabilities.model.refine(&s.info.model_numbers);
            if refined != s.capabilities.model {
                info!(was = ?s.capabilities.model, now = ?refined, "model refined by its A-number");
                s.capabilities = refined.capabilities();
            }
            Self::touch(&mut s);
        }
    }

    /// Returns the previous state, or `None` when the device only
    /// repeated what we already hold.
    pub async fn set_in_ear(&self, in_ear: podctl::InEar) -> Option<podctl::InEar> {
        let mut s = self.state.write().await;
        if s.in_ear == in_ear {
            return None;
        }
        let prev = std::mem::replace(&mut s.in_ear, in_ear);
        Self::touch(&mut s);
        Some(prev)
    }

    /// Record lid state. Returns `Some(open)` only on an edge so callers
    /// emit one event per transition. The trigger itself is a heuristic
    /// (case-battery visibility) until a real lid opcode is captured.
    pub async fn update_case_lid(&self, open: bool) -> Option<bool> {
        let mut s = self.state.write().await;
        let prev = s.case_lid_open.replace(open);
        if prev == Some(open) {
            return None;
        }
        Self::touch(&mut s);
        // After a (re)connect the lid is unknown; "closed" is then no
        // edge, and announcing it would hide the bubble the connect
        // just opened.
        (open || prev.is_some()).then_some(open)
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

    /// Run the BT-link loop: poll BlueZ + PipeWire and supervise the AAP channel.
    pub async fn link_loop(self: Arc<Self>) {
        link::run(self).await;
    }

    /// Hand the battery to BlueZ for the desktop's own battery list.
    pub async fn battery_loop(self: Arc<Self>) {
        battery::run(self).await;
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
            Request::Hello => Response::ok_hello(self.state.read().await.address.clone()),

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
            Request::Pair { address, yes } => link::bt_pair(self, address.clone(), yes).await,
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
                // Nobody draws the bubble but podctl-popup. Answering
                // `ok` with the service down made both the tray click and
                // `podctl popup` silent no-ops.
                if self.popup_attached() {
                    self.broadcast_event(Event::ShowPopup);
                    Response::ok_done()
                } else {
                    Response::err(NO_POPUP)
                }
            }

            Request::Watch { .. } => {
                Response::err("watch is a streaming request — handled separately")
            }
        }
    }

    #[allow(clippy::result_large_err)]
    fn require_caps(
        caps: &Capabilities,
        want: impl Fn(&Capabilities) -> bool,
        feature: &str,
    ) -> Result<(), Response> {
        if !want(caps) {
            return Err(Response::err_code(
                ErrorCode::Unsupported,
                format!("{feature} is not supported on {}", caps.model.label()),
            ));
        }
        Ok(())
    }

    async fn set_mode(&self, mode: Mode) -> Response {
        let caps = self.state.read().await.capabilities;
        let supported = match mode {
            Mode::Off => caps.has_anc || caps.has_transparency,
            Mode::NoiseCancellation => caps.has_anc,
            Mode::Transparency => caps.has_transparency,
            Mode::Adaptive => caps.has_adaptive,
        };
        if !supported {
            return Response::err_code(
                ErrorCode::Unsupported,
                format!("{mode} is not supported on {}", caps.model.label()),
            );
        }
        // Subscribed before sending, so an echo that comes back
        // straight away is not missed.
        let mut echoes = self.settings_echo.subscribe();
        echoes.mark_unchanged();
        let frame = podctl::aap::set_noise_control(mode).encode();
        if let Err(e) = self.aap_send(&frame).await {
            return Response::err(e);
        }
        // The buds echo the new mode as a settings notification
        // (0x09 0d), which the AAP loop puts into state.settings.mode.
        // No echo means they did not take it — they ignore the mode
        // while both sit in the case — and answering "ok" to that sent
        // people looking for the fault in podctl.
        if self.mode_echoed(mode, &mut echoes).await {
            Response::ok_done()
        } else {
            Response::err_code(
                ErrorCode::Unavailable,
                format!(
                    "the buds did not confirm {mode} within {}s — they ignore the listening mode while both are in the case",
                    MODE_ECHO_WAIT.as_secs()
                ),
            )
        }
    }

    /// Waits for the buds' own word on the mode: a settings
    /// notification that arrives after the frame went out and carries
    /// the mode asked for. Generous on purpose — the echo is a local
    /// L2CAP round trip, so a wait this long means it is not coming.
    ///
    /// Reading only `settings.mode` was not enough: asking for the mode
    /// the state already held returned at once, and in the case, where
    /// the buds ignore the mode entirely, that is exactly the lie this
    /// is meant to catch.
    async fn mode_echoed(
        &self,
        mode: Mode,
        echoes: &mut tokio::sync::watch::Receiver<u64>,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + MODE_ECHO_WAIT;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            match tokio::time::timeout(left, echoes.changed()).await {
                // The sender lives as long as the daemon; a closed
                // channel means shutdown, not a confirmation.
                Ok(Err(_)) | Err(_) => return false,
                Ok(Ok(())) => {
                    if self.state.read().await.settings.mode == Some(mode) {
                        return true;
                    }
                }
            }
        }
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
            self.broadcast_event(Event::ConvAwareness { conv });
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
        Response::err_code(ErrorCode::NotImplemented, NOT_IMPL)
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
        // The switch also gates podctl's own auto-pause, and the buds
        // never report it back (not even in the settings dump the Pro 2
        // send after subscribing), so it is ours to keep.
        if let Err(e) = podctl::config::set_ear_detection(on) {
            return Response::err(format!("could not write the settings file: {e}"));
        }
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
        let frame = podctl::aap::set_mic_mode(mic).encode();
        if let Err(e) = self.aap_send(&frame).await {
            return Response::err(e);
        }
        if let Err(e) = podctl::config::set_mic(mic) {
            return Response::err(format!("could not write the settings file: {e}"));
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
        Response::err_code(ErrorCode::NotImplemented, NOT_IMPL)
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
        Response::err_code(ErrorCode::NotImplemented, NOT_IMPL)
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
        Response::err_code(ErrorCode::NotImplemented, NOT_IMPL)
    }

    async fn rename(&self, name: String) -> Response {
        if name.trim().is_empty() {
            return Response::err("name cannot be empty");
        }
        if name.len() > 64 {
            return Response::err("name too long (max 64 bytes UTF-8)");
        }
        // What BlueZ stores is what the tray, the popup and every
        // terminal will print later.
        let name = podctl::model::sanitize_display_name(&name);
        if name.is_empty() {
            return Response::err("name has nothing printable in it");
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
            Ok(Err(e)) => audio_err(e.to_string()),
            Err(e) => Response::err_code(ErrorCode::Internal, format!("rename task: {e}")),
        }
    }
    /// Undo a volume this session lowered for speech — on the way out,
    /// so a duck never outlives the daemon.
    pub async fn undo_duck(&self) {
        self.media.reset().await;
    }

    /// Undo one that an earlier session left behind.
    pub async fn recover_duck(&self) {
        self.media.recover_duck().await;
    }
}

// Audio handlers wrap the sync `podctl::audio` helpers on a blocking task
// and update cached state on success.

/// Every audio helper needs the address of the buds: without one, the
/// sink of the first Bluetooth device on the system would be taken for
/// theirs.
const NO_ADDRESS: &str = "no AirPods address known — none paired, or not seen yet";

/// An audio error, with the kind read off its message: `pactl` missing
/// or failing is not the same as AirPods that are not connected.
fn audio_err(message: String) -> Response {
    Response::err_code(podctl::audio::error_code_for(&message), message)
}

async fn audio_address(d: &Daemon) -> Option<String> {
    d.state.read().await.address.clone()
}

async fn audio_set_volume(d: &Daemon, percent: u8) -> Response {
    let Some(addr) = audio_address(d).await else {
        return Response::err(NO_ADDRESS);
    };
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let sink = podctl::audio::sink_for(&addr)
            .ok_or_else(|| anyhow::anyhow!(podctl::audio::nothing_to_act_on("audio sink")))?;
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
        Ok(Err(e)) => audio_err(e.to_string()),
        Err(e) => Response::err_code(ErrorCode::Internal, format!("audio task: {e}")),
    }
}

async fn audio_set_muted(d: &Daemon, muted: bool) -> Response {
    let Some(addr) = audio_address(d).await else {
        return Response::err(NO_ADDRESS);
    };
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let sink = podctl::audio::sink_for(&addr)
            .ok_or_else(|| anyhow::anyhow!(podctl::audio::nothing_to_act_on("audio sink")))?;
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
        Ok(Err(e)) => audio_err(e.to_string()),
        Err(e) => Response::err_code(ErrorCode::Internal, format!("audio task: {e}")),
    }
}

async fn audio_set_profile(d: &Daemon, profile: podctl::Profile) -> Response {
    let Some(addr) = audio_address(d).await else {
        return Response::err(NO_ADDRESS);
    };
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let card = podctl::audio::card_for(&addr)
            .ok_or_else(|| anyhow::anyhow!(podctl::audio::nothing_to_act_on("card")))?;
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
        Ok(Err(e)) => audio_err(e.to_string()),
        Err(e) => Response::err_code(ErrorCode::Internal, format!("audio task: {e}")),
    }
}

async fn audio_set_codec(d: &Daemon, codec: String) -> Response {
    let codec_for_task = codec.clone();
    let Some(addr) = audio_address(d).await else {
        return Response::err(NO_ADDRESS);
    };
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let card = podctl::audio::card_for(&addr)
            .ok_or_else(|| anyhow::anyhow!(podctl::audio::nothing_to_act_on("card")))?;
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
        Ok(Err(e)) => audio_err(e.to_string()),
        Err(e) => Response::err_code(ErrorCode::Internal, format!("audio task: {e}")),
    }
}

async fn audio_make_default(d: &Daemon) -> Response {
    let Some(addr) = audio_address(d).await else {
        return Response::err(NO_ADDRESS);
    };
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let sink = podctl::audio::sink_for(&addr)
            .ok_or_else(|| anyhow::anyhow!(podctl::audio::nothing_to_act_on("audio sink")))?;
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
        Ok(Err(e)) => audio_err(e.to_string()),
        Err(e) => Response::err_code(ErrorCode::Internal, format!("audio task: {e}")),
    }
}

async fn audio_set_latency(d: &Daemon, ms: i32) -> Response {
    let Some(addr) = audio_address(d).await else {
        return Response::err(NO_ADDRESS);
    };
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let card = podctl::audio::card_for(&addr)
            .ok_or_else(|| anyhow::anyhow!(podctl::audio::nothing_to_act_on("card")))?;
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
        Ok(Err(e)) => audio_err(e.to_string()),
        Err(e) => Response::err_code(ErrorCode::Internal, format!("audio task: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Daemon, Duration, FIRST_POLL_WAIT};
    use podctl::Mode;

    #[tokio::test]
    async fn the_mode_in_the_state_is_not_a_confirmation() {
        // This is what went wrong: the mode asked for already sat in
        // the state, so the command answered at once — while the buds,
        // both in the case, had ignored it.
        let d = Daemon::new();
        d.state.write().await.settings.mode = Some(Mode::Transparency);
        let mut echoes = d.settings_echo.subscribe();
        echoes.mark_unchanged();
        let waited = tokio::time::timeout(
            Duration::from_millis(250),
            d.mode_echoed(Mode::Transparency, &mut echoes),
        )
        .await;
        assert!(waited.is_err(), "without an echo there is no yes");
    }

    #[tokio::test]
    async fn an_echo_carrying_the_mode_confirms_it() {
        let d = std::sync::Arc::new(Daemon::new());
        let mut echoes = d.settings_echo.subscribe();
        echoes.mark_unchanged();
        let buds = std::sync::Arc::clone(&d);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            buds.state.write().await.settings.mode = Some(Mode::Adaptive);
            buds.note_settings_echo();
        });
        assert!(d.mode_echoed(Mode::Adaptive, &mut echoes).await);
    }

    #[tokio::test]
    async fn an_echo_about_something_else_is_not_the_answer() {
        let d = std::sync::Arc::new(Daemon::new());
        let mut echoes = d.settings_echo.subscribe();
        echoes.mark_unchanged();
        let buds = std::sync::Arc::clone(&d);
        tokio::spawn(async move {
            // The buds report the chime volume, say — a notification,
            // but not the one being waited for.
            tokio::time::sleep(Duration::from_millis(30)).await;
            buds.note_settings_echo();
        });
        let waited = tokio::time::timeout(
            Duration::from_millis(250),
            d.mode_echoed(Mode::Transparency, &mut echoes),
        )
        .await;
        assert!(waited.is_err(), "it keeps waiting for the mode asked for");
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_waits_for_the_first_bluez_read() {
        let d = Daemon::new();
        let start = tokio::time::Instant::now();
        d.wait_for_first_poll().await;
        // Nothing read BlueZ, so the wait runs out instead of hanging.
        assert!(start.elapsed() >= FIRST_POLL_WAIT);
    }

    #[tokio::test(start_paused = true)]
    async fn the_read_landing_releases_the_request() {
        let d = std::sync::Arc::new(Daemon::new());
        let marker = std::sync::Arc::clone(&d);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            marker.mark_polled();
        });
        let start = tokio::time::Instant::now();
        d.wait_for_first_poll().await;
        assert!(start.elapsed() >= Duration::from_millis(200));
        assert!(start.elapsed() < FIRST_POLL_WAIT);
    }

    #[tokio::test(start_paused = true)]
    async fn later_requests_do_not_wait_again() {
        let d = Daemon::new();
        d.mark_polled();
        let start = tokio::time::Instant::now();
        d.wait_for_first_poll().await;
        assert!(start.elapsed() < Duration::from_millis(10));
    }
}
