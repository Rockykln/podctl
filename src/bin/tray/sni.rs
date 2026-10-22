use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tracing::warn;
use zbus::OwnedObjectPath;
use zbus::object_server::SignalEmitter;

use podctl::Request;
use podctl::model::Mode;

use crate::config::LeftClick;
use crate::ipc;
use crate::state::TrayState;

/// Percentage points per wheel notch.
const VOLUME_STEP: i16 = 5;
/// What hosts send for one notch of a wheel; touchpads send a little
/// at a time and add up to it.
const NOTCH: i32 = 120;
/// A pause this long starts a new scroll, read from a fresh volume:
/// the keyboard or the mixer may have moved it in between.
const SCROLL_PAUSE: Duration = Duration::from_secs(1);

pub const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";

pub const ITEM_PATH: &str = "/StatusNotifierItem";
pub const MENU_PATH: &str = "/MenuBar";

pub type SharedState = Arc<RwLock<TrayState>>;

pub struct Item {
    state: SharedState,
    left_click: LeftClick,
    scroll: tokio::sync::Mutex<Scroll>,
}

#[derive(Default)]
struct Scroll {
    rest: i32,
    last: Option<Instant>,
}

impl Item {
    pub fn new(state: SharedState, left_click: LeftClick) -> Self {
        Self {
            state,
            left_click,
            scroll: tokio::sync::Mutex::new(Scroll::default()),
        }
    }

    async fn toggle_mode(&self) {
        let s = self.state.read().await;
        if let Some(mode) = toggled(s.settings.mode, &s.capabilities) {
            ipc::dispatch("Changing the mode", Request::SetMode { mode });
        }
    }
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    async fn context_menu(&self, _x: i32, _y: i32) {}

    // No double click: hosts report one as two activations, so the
    // first always ran the single-click action before the second could
    // tell. The middle click is the gesture for the toggle.
    async fn activate(&self, _x: i32, _y: i32) {
        match self.left_click {
            LeftClick::Menu => {}
            LeftClick::Popup => ipc::dispatch("Opening the bubble", Request::ShowPopup),
            LeftClick::ToggleAncTr => self.toggle_mode().await,
            LeftClick::ModeCycle => {
                let s = self.state.read().await;
                if s.capabilities.has_anc || s.capabilities.has_transparency {
                    let mode = cycle_mode(s.settings.mode.unwrap_or(Mode::Off), &s.capabilities);
                    ipc::dispatch("Changing the mode", Request::SetMode { mode });
                }
            }
        }
    }

    /// Middle click: straight between cancellation and transparency,
    /// the two modes worth a shortcut.
    async fn secondary_activate(&self, _x: i32, _y: i32) {
        self.toggle_mode().await;
    }

    async fn scroll(&self, delta: i32, orientation: &str) {
        if delta == 0 || orientation.eq_ignore_ascii_case("horizontal") {
            return;
        }
        // One lock for the whole read-step-write, or two quick ticks
        // both step from the same volume.
        let mut scroll = self.scroll.lock().await;
        let now = Instant::now();
        let fresh = scroll
            .last
            .is_none_or(|t| now.duration_since(t) >= SCROLL_PAUSE);
        scroll.last = Some(now);
        if fresh {
            scroll.rest = 0;
            crate::watch::pull_status(&self.state, Some(ipc::MENU_WAIT)).await;
        }
        let notches = if delta.abs() == 1 {
            // Hosts like waybar add the deltas up themselves and send
            // one unit per notch.
            delta
        } else {
            scroll.rest += delta;
            let n = scroll.rest / NOTCH;
            scroll.rest -= n * NOTCH;
            n
        };
        if notches == 0 {
            return;
        }
        let mut s = self.state.write().await;
        let Some(cur) = s.audio.volume_percent else {
            warn!("no volume known — not scrolling");
            return;
        };
        let step = (notches as i16).saturating_mul(VOLUME_STEP);
        let next = (i16::from(cur) + step).clamp(0, 100) as u8;
        if next == cur {
            return;
        }
        // Written back right away: a wheel sends ticks faster than
        // the daemon answers, and each one should move on from the
        // last rather than from the same stale value.
        s.audio.volume_percent = Some(next);
        ipc::dispatch("Changing the volume", Request::SetVolume { percent: next });
    }

    #[zbus(property)]
    async fn category(&self) -> &'static str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    async fn id(&self) -> &'static str {
        "podctl-tray"
    }

    #[zbus(property)]
    async fn title(&self) -> String {
        self.state.read().await.title()
    }

    #[zbus(property)]
    async fn status(&self) -> String {
        self.state.read().await.status().to_string()
    }

    #[zbus(property)]
    async fn window_id(&self) -> u32 {
        0
    }

    #[zbus(property)]
    async fn icon_name(&self) -> String {
        self.state.read().await.icon_name()
    }

    #[zbus(property)]
    async fn icon_theme_path(&self) -> String {
        crate::icons::theme_path()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }

    #[zbus(property)]
    async fn overlay_icon_name(&self) -> &'static str {
        ""
    }

    #[zbus(property)]
    async fn overlay_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }

    #[zbus(property)]
    async fn attention_icon_name(&self) -> String {
        self.state.read().await.attention_icon_name()
    }

    #[zbus(property)]
    async fn attention_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }

    #[zbus(property)]
    async fn attention_movie_name(&self) -> &'static str {
        ""
    }

    #[zbus(property)]
    async fn tool_tip(&self) -> (String, Vec<(i32, i32, Vec<u8>)>, String, String) {
        let s = self.state.read().await;
        let (title, body) = s.tooltip();
        (s.icon_name(), Vec::new(), title, body)
    }

    #[zbus(property)]
    async fn item_is_menu(&self) -> bool {
        matches!(self.left_click, LeftClick::Menu)
    }

    #[zbus(property)]
    async fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(MENU_PATH).expect("MENU_PATH is a valid object path")
    }

    #[zbus(signal)]
    pub async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_title(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_attention_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_overlay_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
pub trait StatusNotifierWatcher {
    fn register_status_notifier_item(&self, service: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> zbus::Result<Vec<String>>;

    #[zbus(property)]
    fn protocol_version(&self) -> zbus::Result<i32>;
}

fn cycle_mode(cur: Mode, caps: &podctl::caps::Capabilities) -> Mode {
    let candidates = [
        (Mode::Off, true),
        (Mode::NoiseCancellation, caps.has_anc),
        (Mode::Transparency, caps.has_transparency),
        (Mode::Adaptive, caps.has_adaptive),
    ];
    let pos = candidates.iter().position(|(m, _)| *m == cur).unwrap_or(0);
    for i in 1..=candidates.len() {
        let idx = (pos + i) % candidates.len();
        if candidates[idx].1 {
            return candidates[idx].0;
        }
    }
    cur
}

/// Cancellation and transparency trade places; from any other mode the
/// toggle goes to whichever of the two the model has.
fn toggled(cur: Option<Mode>, caps: &podctl::caps::Capabilities) -> Option<Mode> {
    let want = match cur {
        Some(Mode::Transparency) => Mode::NoiseCancellation,
        Some(Mode::NoiseCancellation) => Mode::Transparency,
        _ if caps.has_transparency => Mode::Transparency,
        _ => Mode::NoiseCancellation,
    };
    let can = match want {
        Mode::NoiseCancellation => caps.has_anc,
        _ => caps.has_transparency,
    };
    can.then_some(want)
}
