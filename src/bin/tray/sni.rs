use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tracing::warn;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedObjectPath;

use podctl::model::Mode;
use podctl::{Request, Response};

use crate::config::LeftClick;
use crate::ipc;
use crate::state::TrayState;

/// Percentage points per wheel tick.
const VOLUME_STEP: i16 = 5;
/// Two activations closer together than this are one double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

pub const ITEM_PATH: &str = "/StatusNotifierItem";
pub const MENU_PATH: &str = "/MenuBar";

pub type SharedState = Arc<RwLock<TrayState>>;

pub struct Item {
    state: SharedState,
    left_click: LeftClick,
    last_activate: Mutex<Option<Instant>>,
}

impl Item {
    pub fn new(state: SharedState, left_click: LeftClick) -> Self {
        Self {
            state,
            left_click,
            last_activate: Mutex::new(None),
        }
    }

    /// True when this activation follows one close enough to count as a
    /// double click. Hosts have no such event; they activate twice.
    fn double_click(&self) -> bool {
        let now = Instant::now();
        let mut last = match self.last_activate.lock() {
            Ok(l) => l,
            Err(poisoned) => poisoned.into_inner(),
        };
        let double = last.is_some_and(|t| now.duration_since(t) < DOUBLE_CLICK);
        // A third click starts over rather than toggling again.
        *last = (!double).then_some(now);
        double
    }

    async fn toggle_mode(&self) {
        let cur = self.state.read().await.mode;
        spawn_dispatch(Request::SetMode {
            mode: match cur {
                Some(Mode::Transparency) => Mode::NoiseCancellation,
                _ => Mode::Transparency,
            },
        });
    }
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    async fn context_menu(&self, _x: i32, _y: i32) {}

    async fn activate(&self, _x: i32, _y: i32) {
        if self.double_click() {
            self.toggle_mode().await;
            return;
        }
        let req = match self.left_click {
            LeftClick::Menu => return,
            LeftClick::Popup => Request::ShowPopup,
            LeftClick::ToggleAncTr => {
                let cur = self.state.read().await.mode;
                Request::SetMode {
                    mode: match cur {
                        Some(Mode::Transparency) => Mode::NoiseCancellation,
                        _ => Mode::Transparency,
                    },
                }
            }
            LeftClick::ModeCycle => {
                let s = self.state.read().await;
                let next = cycle_mode(s.mode.unwrap_or(Mode::Off), &s.capabilities);
                Request::SetMode { mode: next }
            }
        };
        spawn_dispatch(req);
    }

    /// Middle click, and a double click: straight between cancellation
    /// and transparency, the two modes worth a shortcut.
    async fn secondary_activate(&self, _x: i32, _y: i32) {
        self.toggle_mode().await;
    }

    async fn scroll(&self, delta: i32, orientation: &str) {
        if delta == 0 || orientation.eq_ignore_ascii_case("horizontal") {
            return;
        }
        let up = delta > 0;
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let cached = state.read().await.volume;
            let cur = match cached {
                Some(v) => v,
                None => {
                    crate::watch::pull_status(&state).await;
                    let Some(v) = state.read().await.volume else {
                        warn!("no volume known — not scrolling");
                        return;
                    };
                    v
                }
            };
            let step = if up { VOLUME_STEP } else { -VOLUME_STEP };
            let next = (i16::from(cur) + step).clamp(0, 100) as u8;
            if next == cur {
                return;
            }
            // Written back right away: a wheel sends ticks faster than
            // the daemon answers, and each one should move on from the
            // last rather than from the same stale value.
            state.write().await.volume = Some(next);
            spawn_dispatch(Request::SetVolume { percent: next });
        });
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
        self.state.read().await.icon_name().to_string()
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
    async fn attention_icon_name(&self) -> &'static str {
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
        let (title, body) = self.state.read().await.tooltip();
        let icon = self.state.read().await.icon_name().to_string();
        (icon, Vec::new(), title, body)
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

fn spawn_dispatch(req: Request) {
    tokio::spawn(async move {
        let label = format!("{req:?}");
        match ipc::send(&req).await {
            Ok(Response::Err(e)) => warn!(req = %label, error = %e, "daemon refused"),
            Err(e) => warn!(req = %label, error = %e, "ipc failure"),
            _ => {}
        }
    });
}
