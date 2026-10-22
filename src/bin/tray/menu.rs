use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use tracing::{debug, warn};
use zbus::object_server::SignalEmitter;
use zbus::{Array, Dict, OwnedVariant, Structure, Variant};

use podctl::caps::Model;
use podctl::{ConvAwareness, MicSelection, Mode, Profile, Request};

use crate::config::LeftClick;
use crate::ipc;
use crate::sni::SharedState;
use crate::state::{TrayState, mode_label};

const HEADER: i32 = 1;
const SUBHEAD: i32 = 2;
const DAEMON_DOWN: i32 = 3;
const START_DAEMON: i32 = 4;
const SEP_HEAD: i32 = 5;
const BAT_LEFT: i32 = 10;
const BAT_RIGHT: i32 = 11;
const BAT_CASE: i32 = 12;
const BAT_ONE: i32 = 13;
const BAT_LAST: i32 = 14;
const SEP_BAT: i32 = 15;
const CONNECT: i32 = 16;
const NOT_PAIRED: i32 = 17;
const NOISE: i32 = 20;
const MODE_ANC: i32 = 21;
const MODE_ADAPT: i32 = 22;
const MODE_TRANS: i32 = 23;
const MODE_OFF: i32 = 24;
const CONV: i32 = 25;
const SEP_NOISE: i32 = 26;
const VOLUME: i32 = 30;
const VOL_STEPS: [(i32, u8); 4] = [(31, 25), (32, 50), (33, 75), (34, 100)];
const SEP_VOL: i32 = 35;
const MUTE: i32 = 36;
const OUTPUT: i32 = 40;
const OUT_MUSIC: i32 = 41;
const OUT_CALLS: i32 = 42;
const SEP_OUT: i32 = 43;
const OUT_DEFAULT: i32 = 44;
const MORE: i32 = 50;
const EAR: i32 = 51;
const MIC: i32 = 52;
const MIC_AUTO: i32 = 53;
const MIC_LEFT: i32 = 54;
const MIC_RIGHT: i32 = 55;
const SEP_LINK: i32 = 60;
const DISCONNECT: i32 = 61;
const POPUP: i32 = 62;
const SEP_END: i32 = 63;
const QUIT: i32 = 64;

/// A status older than this is fetched again when the menu opens.
const STALE: Duration = Duration::from_secs(2);

pub struct Menu {
    state: SharedState,
    quit: Arc<Notify>,
    left_click: LeftClick,
    revision: AtomicU32,
    /// The state the host last saw, to send only what changed.
    shown: Mutex<Option<TrayState>>,
}

impl Menu {
    pub fn new(state: SharedState, quit: Arc<Notify>, left_click: LeftClick) -> Self {
        Self {
            state,
            quit,
            left_click,
            revision: AtomicU32::new(1),
            shown: Mutex::new(None),
        }
    }

    /// What the host has to be told for the menu to show `now`: the
    /// changed items, and whether which items are visible changed too.
    pub fn changes(&self, now: &TrayState) -> Changes {
        let before = self
            .shown
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .replace(now.clone());
        let new = flatten(tree(now, self.left_click));
        let Some(before) = before else {
            return Changes {
                updated: new,
                removed: Vec::new(),
                revision: None,
            };
        };
        let old: HashMap<i32, Props> = flatten(tree(&before, self.left_click))
            .into_iter()
            .collect();
        let shape = |items: &mut dyn Iterator<Item = (&i32, &Props)>| -> Vec<i32> {
            let mut ids: Vec<i32> = items
                .filter(|(_, p)| is_visible(p))
                .map(|(id, _)| *id)
                .collect();
            ids.sort_unstable();
            ids
        };
        let reshaped = shape(&mut old.iter()) != shape(&mut new.iter().map(|(i, p)| (i, p)));
        // A property that went back to its default has to be named, or
        // the host keeps the old value: Plasma kept a whole menu hidden.
        let removed = new
            .iter()
            .filter_map(|(id, p)| {
                let gone: Vec<String> = old
                    .get(id)?
                    .keys()
                    .filter(|k| !p.contains_key(*k))
                    .cloned()
                    .collect();
                (!gone.is_empty()).then_some((*id, gone))
            })
            .collect();
        let updated = new
            .into_iter()
            // Hidden before and after: nothing on screen changes.
            .filter(|(id, p)| {
                old.get(id)
                    .is_none_or(|o| o != p && (is_visible(o) || is_visible(p)))
            })
            .collect();
        let revision = reshaped.then(|| self.revision.fetch_add(1, Ordering::SeqCst) + 1);
        Changes {
            updated,
            removed,
            revision,
        }
    }

    /// A host that just appeared has seen nothing yet.
    pub fn forget_shown(&self) {
        *self.shown.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

pub type Props = HashMap<String, OwnedVariant>;

/// What a host has to be told after a state change.
pub struct Changes {
    pub updated: Vec<(i32, Props)>,
    pub removed: Vec<(i32, Vec<String>)>,
    pub revision: Option<u32>,
}
type LayoutNode = (i32, Props, Vec<OwnedVariant>);

struct Node {
    id: i32,
    props: Props,
    children: Vec<Node>,
}

fn node(id: i32, props: Props) -> Node {
    Node {
        id,
        props,
        children: Vec::new(),
    }
}

fn submenu(id: i32, mut props: Props, children: Vec<Node>) -> Node {
    props.insert("children-display".into(), str_v("submenu"));
    Node {
        id,
        props,
        children,
    }
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    async fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, LayoutNode) {
        let state = self.state.read().await;
        let rev = self.revision.load(Ordering::SeqCst);
        let root = Node {
            id: 0,
            props: Props::new(),
            children: tree(&state, self.left_click),
        };
        let layout = match find(&root, parent_id) {
            Some(n) => layout(n, recursion_depth),
            None => (parent_id, Props::new(), Vec::new()),
        };
        (rev, layout)
    }

    async fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, Props)> {
        let state = self.state.read().await;
        let mut all: HashMap<i32, Props> =
            flatten(tree(&state, self.left_click)).into_iter().collect();
        if ids.is_empty() {
            return all.into_iter().collect();
        }
        ids.into_iter()
            .filter_map(|id| all.remove(&id).map(|p| (id, p)))
            .collect()
    }

    async fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedVariant> {
        let state = self.state.read().await;
        flatten(tree(&state, self.left_click))
            .into_iter()
            .find(|(i, _)| *i == id)
            .and_then(|(_, mut p)| p.remove(name))
            .ok_or_else(|| {
                zbus::fdo::Error::InvalidArgs(format!("no property {name} on item {id}"))
            })
    }

    async fn about_to_show(&self, #[zbus(connection)] conn: &zbus::Connection, _id: i32) -> bool {
        self.refresh(conn).await
    }

    async fn about_to_show_group(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        ids: Vec<i32>,
    ) -> (Vec<i32>, Vec<i32>) {
        let pulled = self.refresh(conn).await;
        // (updates_needed, id_errors)
        (if pulled { ids } else { Vec::new() }, Vec::new())
    }

    async fn event(&self, id: i32, event_id: &str, _data: Variant<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        debug!(item = id, "menu click");
        let s = self.state.read().await.clone();
        let set_mode = |mode| ipc::dispatch("Changing the mode", Request::SetMode { mode });
        match id {
            MODE_ANC => set_mode(Mode::NoiseCancellation),
            MODE_ADAPT => set_mode(Mode::Adaptive),
            MODE_TRANS => set_mode(Mode::Transparency),
            MODE_OFF => set_mode(Mode::Off),
            CONV => {
                let conv = match s.settings.conv_awareness {
                    Some(ConvAwareness::On) => ConvAwareness::Off,
                    _ => ConvAwareness::On,
                };
                ipc::dispatch("Conversation Awareness", Request::SetConv { conv });
            }
            MUTE => ipc::dispatch(
                "Muting",
                Request::SetMuted {
                    muted: !s.audio.muted,
                },
            ),
            OUT_MUSIC => ipc::dispatch(
                "Switching to music",
                Request::SetProfile {
                    profile: Profile::HighQuality,
                },
            ),
            OUT_CALLS => ipc::dispatch(
                "Switching to calls",
                Request::SetProfile {
                    profile: Profile::Headset,
                },
            ),
            OUT_DEFAULT if !s.audio.is_default_sink => {
                ipc::dispatch("Making them the default output", Request::MakeDefaultSink)
            }
            EAR => ipc::dispatch(
                "Ear detection",
                Request::SetEarDetection {
                    on: s.settings.ear_detection != Some(true),
                },
            ),
            MIC_AUTO => set_mic(MicSelection::Auto),
            MIC_LEFT => set_mic(MicSelection::AlwaysLeft),
            MIC_RIGHT => set_mic(MicSelection::AlwaysRight),
            CONNECT => ipc::dispatch("Connecting", Request::Connect),
            DISCONNECT => ipc::dispatch("Disconnecting", Request::Disconnect),
            POPUP => ipc::dispatch("Opening the bubble", Request::ShowPopup),
            START_DAEMON => start_daemon(),
            QUIT => self.quit.notify_one(),
            _ => {
                if let Some(&(_, percent)) = VOL_STEPS.iter().find(|(i, _)| *i == id) {
                    ipc::dispatch("Changing the volume", Request::SetVolume { percent });
                }
            }
        }
    }

    async fn event_group(
        &self,
        #[zbus(signal_emitter)] _emitter: SignalEmitter<'_>,
        events: Vec<(i32, String, Variant<'_>, u32)>,
    ) -> Vec<i32> {
        for (id, event_id, data, ts) in events {
            self.event(id, &event_id, data, ts).await;
        }
        Vec::new()
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> &'static str {
        "ltr"
    }

    #[zbus(property)]
    fn status(&self) -> &'static str {
        "normal"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        crate::icons::theme_path()
            .map(|p| vec![p.display().to_string()])
            .unwrap_or_default()
    }

    #[zbus(signal)]
    pub async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn items_properties_updated(
        emitter: &SignalEmitter<'_>,
        updated: Vec<(i32, Props)>,
        removed: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;
}

impl Menu {
    /// Fetch a fresh status before the menu opens, but never keep it
    /// waiting on a daemon that is stuck: what we have is shown instead.
    async fn refresh(&self, conn: &zbus::Connection) -> bool {
        let stale = self
            .state
            .read()
            .await
            .pulled
            .is_none_or(|t| t.elapsed() >= STALE);
        if stale {
            crate::watch::pull_status(&self.state, Some(ipc::MENU_WAIT)).await;
            crate::watch::notify_changed(&self.state, conn, crate::notify::get()).await;
        }
        stale
    }
}

fn set_mic(mic: MicSelection) {
    ipc::dispatch("Choosing the microphone", Request::SetMic { mic });
}

fn start_daemon() {
    tokio::spawn(async {
        let out = tokio::process::Command::new("systemctl")
            .args(["--user", "start", "podctld"])
            .output()
            .await;
        let why = match out {
            Ok(o) if o.status.success() => return,
            Ok(o) => String::from_utf8_lossy(&o.stderr).trim().to_string(),
            Err(e) => e.to_string(),
        };
        warn!(error = %why, "starting podctld failed");
        if let Some(n) = crate::notify::get() {
            n.failed("Starting podctld", &why).await;
        }
    });
}

/// The whole menu. Every item is always there and only shown or hidden,
/// so ids stay put whatever the state.
fn tree(s: &TrayState, left_click: LeftClick) -> Vec<Node> {
    let up = s.daemon_up;
    let on = up && s.connected;
    let caps = &s.capabilities;
    let b = &s.battery;
    let single = matches!(caps.model, Model::Max | Model::MaxUsbC | Model::Max2);
    let noise = caps.has_anc || caps.has_transparency;
    let mode = s.settings.mode;

    let subhead = match (&s.model, on) {
        (Some(m), true) => format!("{m} · Connected"),
        (None, true) => "Connected".into(),
        (_, false) if !s.paired => "Not paired".into(),
        (_, false) => "Not connected".into(),
    };
    let volume = match s.audio.volume_percent {
        Some(_) if s.audio.muted => "Volume (muted)".to_string(),
        Some(v) => format!("Volume {v}%"),
        None => "Volume".into(),
    };

    let mut items = vec![
        node(
            HEADER,
            with_icon(
                text(&s.title(), true),
                if crate::icons::theme_path().is_some() {
                    crate::icons::PLAIN
                } else {
                    "audio-headphones-symbolic"
                },
            )
            .shown(up),
        ),
        node(SUBHEAD, text(&subhead, false).shown(up)),
        node(
            DAEMON_DOWN,
            with_icon(
                text("podctld is not running", true),
                "dialog-warning-symbolic",
            )
            .shown(!up),
        ),
        node(START_DAEMON, text("Start podctld", true).shown(!up)),
        node(SEP_HEAD, separator()),
        node(
            BAT_LEFT,
            level("Left", b.left, b.left_charging).shown(on && !single),
        ),
        node(
            BAT_RIGHT,
            level("Right", b.right, b.right_charging).shown(on && !single),
        ),
        node(
            BAT_CASE,
            level("Case", b.case, b.case_charging).shown(on && !single),
        ),
        node(
            BAT_ONE,
            level(
                "Battery",
                b.left.or(b.right),
                b.left_charging || b.right_charging,
            )
            .shown(on && single),
        ),
        node(
            BAT_LAST,
            text(
                &format!("Last seen: {}", s.levels().unwrap_or_default()),
                true,
            )
            .shown(up && !s.connected && s.battery.any_known()),
        ),
        node(SEP_BAT, separator()),
        node(
            CONNECT,
            text("Connect", true).shown(up && !s.connected && s.paired),
        ),
        node(
            NOT_PAIRED,
            text("Not paired — run 'podctl pair'", false).shown(up && !s.paired),
        ),
        node(NOISE, text("Noise Control", false).shown(on && noise)),
        node(
            MODE_ANC,
            radio(
                mode_label(Mode::NoiseCancellation),
                mode == Some(Mode::NoiseCancellation),
            )
            .shown(on && caps.has_anc),
        ),
        node(
            MODE_ADAPT,
            radio(mode_label(Mode::Adaptive), mode == Some(Mode::Adaptive))
                .shown(on && caps.has_adaptive),
        ),
        node(
            MODE_TRANS,
            radio(
                mode_label(Mode::Transparency),
                mode == Some(Mode::Transparency),
            )
            .shown(on && caps.has_transparency),
        ),
        node(
            MODE_OFF,
            radio(mode_label(Mode::Off), mode == Some(Mode::Off)).shown(on && noise),
        ),
        node(
            CONV,
            check(
                "Conversation Awareness",
                s.settings.conv_awareness == Some(ConvAwareness::On),
            )
            .shown(on && caps.has_conv_awareness),
        ),
        node(SEP_NOISE, separator()),
        submenu(
            VOLUME,
            text(&volume, true).shown(on && s.audio.volume_percent.is_some()),
            VOL_STEPS
                .iter()
                .map(|&(id, pct)| {
                    node(
                        id,
                        radio(&format!("{pct}%"), s.audio.volume_percent == Some(pct)),
                    )
                })
                .chain([
                    node(SEP_VOL, separator()),
                    node(MUTE, check("Mute", s.audio.muted)),
                ])
                .collect(),
        ),
        submenu(
            OUTPUT,
            text("Audio Output", true).shown(on && s.audio.profile.is_some()),
            vec![
                node(
                    OUT_MUSIC,
                    radio(
                        "Music (high quality)",
                        s.audio.profile == Some(Profile::HighQuality),
                    ),
                ),
                node(
                    OUT_CALLS,
                    radio("Calls (headset)", s.audio.profile == Some(Profile::Headset)),
                ),
                node(SEP_OUT, separator()),
                node(
                    OUT_DEFAULT,
                    check("Default output", s.audio.is_default_sink),
                ),
            ],
        ),
        submenu(
            MORE,
            text("More", true)
                .shown(on && (caps.has_ear_detection_setting || caps.has_mic_selection)),
            vec![
                node(
                    EAR,
                    check(
                        "Automatic Ear Detection",
                        s.settings.ear_detection == Some(true),
                    )
                    .shown(caps.has_ear_detection_setting),
                ),
                submenu(
                    MIC,
                    text("Microphone", true).shown(caps.has_mic_selection),
                    vec![
                        node(
                            MIC_AUTO,
                            radio(
                                "Automatic",
                                s.settings.mic_selection == Some(MicSelection::Auto),
                            ),
                        ),
                        node(
                            MIC_LEFT,
                            radio(
                                "Always Left",
                                s.settings.mic_selection == Some(MicSelection::AlwaysLeft),
                            ),
                        ),
                        node(
                            MIC_RIGHT,
                            radio(
                                "Always Right",
                                s.settings.mic_selection == Some(MicSelection::AlwaysRight),
                            ),
                        ),
                    ],
                ),
            ],
        ),
        node(SEP_LINK, separator()),
        node(DISCONNECT, text("Disconnect", true).shown(on)),
        node(
            POPUP,
            text("Show bubble", true).shown(up && left_click != LeftClick::Popup),
        ),
        node(SEP_END, separator()),
        node(QUIT, text("Quit", true)),
    ];
    tidy_separators(&mut items);
    items
}

/// A separator shows only between two visible items, never twice in a
/// row or at either end.
fn tidy_separators(items: &mut [Node]) {
    let is_sep = |n: &Node| n.props.contains_key("type");
    let visible: Vec<bool> = items
        .iter()
        .map(|n| !is_sep(n) && is_visible(&n.props))
        .collect();
    let mut seen_item = false;
    let mut pending: Option<usize> = None;
    for i in 0..items.len() {
        if is_sep(&items[i]) {
            items[i].props.insert("visible".into(), bool_v(false));
            if seen_item && pending.is_none() {
                pending = Some(i);
            }
        } else if visible[i] {
            if let Some(p) = pending.take() {
                items[p].props.insert("visible".into(), bool_v(true));
            }
            seen_item = true;
        }
    }
}

fn find(n: &Node, id: i32) -> Option<&Node> {
    if n.id == id {
        return Some(n);
    }
    n.children.iter().find_map(|c| find(c, id))
}

fn layout(n: &Node, depth: i32) -> LayoutNode {
    let children = if depth == 0 {
        Vec::new()
    } else {
        n.children
            .iter()
            .map(|c| {
                let (id, props, kids) = layout(c, depth - 1);
                wrap(id, props, kids)
            })
            .collect()
    };
    (n.id, clone_props(&n.props), children)
}

fn flatten(nodes: Vec<Node>) -> Vec<(i32, Props)> {
    let mut out = Vec::new();
    let mut stack = nodes;
    stack.reverse();
    while let Some(n) = stack.pop() {
        stack.extend(n.children.into_iter().rev());
        out.push((n.id, n.props));
    }
    out
}

fn is_visible(p: &Props) -> bool {
    !matches!(p.get("visible").map(|v| &**v), Some(Variant::Bool(false)))
}

fn clone_props(p: &Props) -> Props {
    p.iter()
        .filter_map(|(k, v)| v.try_clone().ok().map(|v| (k.clone(), v)))
        .collect()
}

fn wrap(id: i32, props: Props, children: Vec<OwnedVariant>) -> OwnedVariant {
    let prop_dict: Dict<'_, '_> = props.into();
    let child_array: Array<'_> = children.into();
    let s = Structure::builder()
        .append_field(Variant::I32(id))
        .append_field(Variant::Dict(prop_dict))
        .append_field(Variant::Array(child_array))
        .build()
        .expect("layout structure has three fields");
    Variant::Structure(s)
        .try_to_owned()
        .expect("layout node is serialisable")
}

trait Shown {
    fn shown(self, visible: bool) -> Self;
}

impl Shown for Props {
    /// Always said out loud: a host that rebuilds from GetLayout keeps a
    /// stale "false" when the key is simply left out.
    fn shown(mut self, visible: bool) -> Self {
        self.insert("visible".into(), bool_v(visible));
        self
    }
}

/// One battery line. It is enabled although clicking it does nothing:
/// a disabled line is greyed out, and this is the line people open the
/// menu for.
fn level(label: &str, pct: Option<u8>, charging: bool) -> Props {
    match pct {
        Some(p) if charging => text(&format!("{label}  {p}%  ⚡ charging"), true),
        Some(p) => text(&format!("{label}  {p}%"), true),
        None => text(label, true).shown(false),
    }
}

fn text(label: &str, enabled: bool) -> Props {
    let mut p = Props::new();
    p.insert("label".into(), str_v(label));
    if !enabled {
        p.insert("enabled".into(), bool_v(false));
    }
    p
}

fn with_icon(mut p: Props, icon: &str) -> Props {
    p.insert("icon-name".into(), str_v(icon));
    p
}

fn radio(label: &str, selected: bool) -> Props {
    let mut p = text(label, true);
    p.insert("toggle-type".into(), str_v("radio"));
    p.insert("toggle-state".into(), int_v(i32::from(selected)));
    p
}

fn check(label: &str, on: bool) -> Props {
    let mut p = text(label, true);
    p.insert("toggle-type".into(), str_v("checkmark"));
    p.insert("toggle-state".into(), int_v(i32::from(on)));
    p
}

fn separator() -> Props {
    let mut p = Props::new();
    p.insert("type".into(), str_v("separator"));
    p
}

fn str_v(s: &str) -> OwnedVariant {
    Variant::new(s.to_string())
        .try_to_owned()
        .expect("string is a value")
}

fn int_v(i: i32) -> OwnedVariant {
    Variant::I32(i).try_to_owned().expect("i32 is a value")
}

fn bool_v(b: bool) -> OwnedVariant {
    Variant::Bool(b).try_to_owned().expect("bool is a value")
}

#[cfg(test)]
mod tests {
    use super::*;
    use podctl::model::Battery;

    fn label(p: &Props) -> String {
        match p.get("label").map(|v| &**v) {
            Some(Variant::Str(s)) => s.to_string(),
            _ => String::new(),
        }
    }

    /// The labels a host would show, top level only.
    fn shown(s: &TrayState) -> Vec<String> {
        tree(s, LeftClick::Popup)
            .iter()
            .filter(|n| is_visible(&n.props))
            .map(|n| {
                if n.props.contains_key("type") {
                    "---".into()
                } else {
                    label(&n.props)
                }
            })
            .collect()
    }

    fn pro2() -> TrayState {
        let mut s = TrayState::new(20);
        s.daemon_up = true;
        s.connected = true;
        s.paired = true;
        s.name = Some("Test AirPods".into());
        s.model = Some("AirPods Pro 2".into());
        s.capabilities = Model::Pro2UsbC.capabilities();
        s.battery = Battery {
            left: Some(80),
            right: Some(75),
            case: Some(40),
            case_charging: true,
            ..Battery::default()
        };
        s.audio.volume_percent = Some(45);
        s.audio.profile = Some(Profile::HighQuality);
        s
    }

    #[test]
    fn connected_pro_shows_levels_modes_and_submenus() {
        assert_eq!(
            shown(&pro2()),
            [
                "Test AirPods",
                "AirPods Pro 2 · Connected",
                "---",
                "Left  80%",
                "Right  75%",
                "Case  40%  ⚡ charging",
                "---",
                "Noise Control",
                "Noise Cancellation",
                "Adaptive",
                "Transparency",
                "Off",
                "Conversation Awareness",
                "---",
                "Volume 45%",
                "Audio Output",
                "More",
                "---",
                "Disconnect",
                "---",
                "Quit",
            ]
        );
    }

    #[test]
    fn disconnected_offers_connect_first() {
        let mut s = pro2();
        s.forget_link();
        s.battery.left = Some(80);
        assert_eq!(
            shown(&s),
            [
                "Test AirPods",
                "Not connected",
                "---",
                "Last seen: L 80%",
                "---",
                "Connect",
                "---",
                "Quit",
            ]
        );
    }

    #[test]
    fn nothing_paired_points_at_pair_instead_of_connect() {
        let mut s = pro2();
        s.forget_link();
        s.paired = false;
        s.name = None;
        s.model = None;
        assert_eq!(
            shown(&s),
            [
                "AirPods",
                "Not paired",
                "---",
                "Not paired — run 'podctl pair'",
                "---",
                "Quit",
            ]
        );
    }

    #[test]
    fn without_a_daemon_only_starting_it_is_left() {
        let mut s = pro2();
        s.daemon_up = false;
        s.forget_link();
        assert_eq!(
            shown(&s),
            ["podctld is not running", "Start podctld", "---", "Quit"]
        );
    }

    #[test]
    fn a_model_without_noise_control_hides_the_block() {
        let mut s = pro2();
        s.capabilities = Model::Gen3.capabilities();
        let labels = shown(&s);
        assert!(!labels.iter().any(|l| l == "Noise Control" || l == "Off"));
        assert!(labels.iter().any(|l| l == "Volume 45%"));
    }

    #[test]
    fn only_a_change_in_what_shows_bumps_the_revision() {
        let m = Menu::new(
            Arc::new(tokio::sync::RwLock::new(TrayState::default())),
            Arc::new(Notify::new()),
            LeftClick::Popup,
        );
        let mut s = pro2();
        let all = m.changes(&s);
        assert!(all.updated.len() > 20);
        assert_eq!(all.revision, None);

        s.battery.left = Some(79);
        let c = m.changes(&s);
        assert_eq!(
            c.updated.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [BAT_LEFT]
        );
        assert_eq!(c.revision, None);

        s.forget_link();
        assert_eq!(m.changes(&s).revision, Some(2));
    }

    #[test]
    fn a_reconnect_says_what_shows_again() {
        let m = Menu::new(
            Arc::new(tokio::sync::RwLock::new(TrayState::default())),
            Arc::new(Notify::new()),
            LeftClick::Popup,
        );
        let mut s = pro2();
        s.forget_link();
        m.changes(&s);
        let c = m.changes(&pro2());
        let left = c
            .updated
            .iter()
            .find(|(id, _)| *id == BAT_LEFT)
            .map(|(_, p)| p)
            .expect("the left bud's line comes back");
        assert!(matches!(
            left.get("visible").map(|v| &**v),
            Some(Variant::Bool(true))
        ));
    }
}
