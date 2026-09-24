use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::Notify;
use tracing::{debug, warn};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{Array, Dict, OwnedValue, StructureBuilder, Value};

use podctl::{ConvAwareness, Mode, Request, Response};

use crate::ipc;
use crate::sni::SharedState;
use crate::state::TrayState;

const BATTERY: i32 = 6;
const SEP_TOP: i32 = 7;
const MODE_OFF: i32 = 10;
const MODE_ANC: i32 = 11;
const MODE_TRANS: i32 = 12;
const MODE_ADAPT: i32 = 13;
const CONV: i32 = 2;
const SEP: i32 = 3;
const DISCONNECT: i32 = 4;
const QUIT: i32 = 5;

pub struct Menu {
    state: SharedState,
    quit: Arc<Notify>,
    revision: AtomicU32,
}

impl Menu {
    pub fn new(state: SharedState, quit: Arc<Notify>) -> Self {
        Self {
            state,
            quit,
            revision: AtomicU32::new(1),
        }
    }
}

pub type Props = HashMap<String, OwnedValue>;
type LayoutNode = (i32, Props, Vec<OwnedValue>);

pub fn properties_snapshot(state: &TrayState) -> Vec<(i32, Props)> {
    vec![
        (
            MODE_OFF,
            radio(
                "Off",
                state.mode == Some(Mode::Off),
                off_enabled(&state.capabilities),
            ),
        ),
        (
            MODE_ANC,
            radio(
                "Noise Cancellation",
                state.mode == Some(Mode::NoiseCancellation),
                state.capabilities.has_anc,
            ),
        ),
        (
            MODE_TRANS,
            radio(
                "Transparency",
                state.mode == Some(Mode::Transparency),
                state.capabilities.has_transparency,
            ),
        ),
        (
            MODE_ADAPT,
            radio(
                "Adaptive",
                state.mode == Some(Mode::Adaptive),
                state.capabilities.has_adaptive,
            ),
        ),
        (CONV, conv_props(state)),
        (BATTERY, battery_props(state)),
        (DISCONNECT, link_props(state)),
    ]
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    async fn get_layout(
        &self,
        parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, LayoutNode) {
        let state = self.state.read().await;
        let rev = self.revision.load(Ordering::SeqCst);
        // Honour parent_id: the host asks for the root (0) and then,
        // separately, for the Mode submenu. Returning the full tree for
        // every parent makes the submenu mirror the whole menu.
        let node = match parent_id {
            0 => build_layout(&state),
            other => (
                other,
                item_props(other, &state).unwrap_or_default(),
                Vec::new(),
            ),
        };
        (rev, node)
    }

    async fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, Props)> {
        let state = self.state.read().await;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(p) = item_props(id, &state) {
                out.push((id, p));
            }
        }
        out
    }

    async fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        let state = self.state.read().await;
        item_props(id, &state)
            .and_then(|mut p| p.remove(name))
            .ok_or_else(|| {
                zbus::fdo::Error::InvalidArgs(format!("no property {name} on item {id}"))
            })
    }

    async fn about_to_show(&self, _id: i32) -> bool {
        // Refresh from the daemon so capabilities / mode / conv are
        // current even if no event fired since the tray started.
        crate::watch::pull_status(&self.state).await;
        true
    }

    async fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        crate::watch::pull_status(&self.state).await;
        // (updates_needed, id_errors): re-pull these groups, none errored.
        (ids, Vec::new())
    }

    async fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        debug!(item = id, "menu click");
        match id {
            MODE_OFF => spawn_dispatch(Request::SetMode { mode: Mode::Off }),
            MODE_ANC => spawn_dispatch(Request::SetMode {
                mode: Mode::NoiseCancellation,
            }),
            MODE_TRANS => spawn_dispatch(Request::SetMode {
                mode: Mode::Transparency,
            }),
            MODE_ADAPT => spawn_dispatch(Request::SetMode {
                mode: Mode::Adaptive,
            }),
            CONV => {
                let next = match self.state.read().await.conv_awareness {
                    Some(ConvAwareness::On) => ConvAwareness::Off,
                    _ => ConvAwareness::On,
                };
                spawn_dispatch(Request::SetConv { conv: next });
            }
            DISCONNECT => {
                let connected = self.state.read().await.connected;
                spawn_dispatch(if connected {
                    Request::Disconnect
                } else {
                    Request::Connect
                });
            }
            QUIT => self.quit.notify_one(),
            _ => {}
        }
    }

    async fn event_group(
        &self,
        #[zbus(signal_emitter)] _emitter: SignalEmitter<'_>,
        events: Vec<(i32, String, Value<'_>, u32)>,
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
        Vec::new()
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

fn build_layout(state: &TrayState) -> LayoutNode {
    let mut children = vec![
        wrap(BATTERY, battery_props(state), Vec::new()),
        wrap(SEP_TOP, separator_props(), Vec::new()),
    ];
    // The four modes sit at the top level: a submenu cost a second
    // click for the one thing the icon is opened for.
    children.extend(mode_children(state));
    children.extend([
        wrap(CONV, conv_props(state), Vec::new()),
        wrap(SEP, separator_props(), Vec::new()),
        wrap(DISCONNECT, link_props(state), Vec::new()),
        wrap(QUIT, leaf_props("Quit tray", true), Vec::new()),
    ]);
    (0, Props::new(), children)
}

/// The battery line is the reason most people open the menu, so it sits
/// at the top and is not clickable.
fn battery_props(state: &TrayState) -> Props {
    let (_, body) = state.tooltip();
    leaf_props(&body, false)
}

fn link_props(state: &TrayState) -> Props {
    if state.connected {
        leaf_props("Disconnect", true)
    } else {
        leaf_props("Connect", true)
    }
}

fn wrap(id: i32, props: Props, children: Vec<OwnedValue>) -> OwnedValue {
    let prop_dict: Dict<'_, '_> = props.into();
    let child_array: Array<'_> = children.into();
    let s = StructureBuilder::new()
        .append_field(Value::I32(id))
        .append_field(Value::Dict(prop_dict))
        .append_field(Value::Array(child_array))
        .build()
        .expect("layout structure has three fields");
    Value::Structure(s)
        .try_to_owned()
        .expect("layout node is serialisable")
}

fn item_props(id: i32, state: &TrayState) -> Option<Props> {
    match id {
        0 => Some(Props::new()),
        BATTERY => Some(battery_props(state)),
        SEP_TOP => Some(separator_props()),
        MODE_OFF => Some(radio(
            "Off",
            state.mode == Some(Mode::Off),
            off_enabled(&state.capabilities),
        )),
        MODE_ANC => Some(radio(
            "Noise Cancellation",
            state.mode == Some(Mode::NoiseCancellation),
            state.capabilities.has_anc,
        )),
        MODE_TRANS => Some(radio(
            "Transparency",
            state.mode == Some(Mode::Transparency),
            state.capabilities.has_transparency,
        )),
        MODE_ADAPT => Some(radio(
            "Adaptive",
            state.mode == Some(Mode::Adaptive),
            state.capabilities.has_adaptive,
        )),
        CONV => Some(conv_props(state)),
        SEP => Some(separator_props()),
        DISCONNECT => Some(link_props(state)),
        QUIT => Some(leaf_props("Quit tray", true)),
        _ => None,
    }
}

fn mode_children(state: &TrayState) -> Vec<OwnedValue> {
    let mode = state.mode;
    let caps = state.capabilities;
    vec![
        wrap(
            MODE_OFF,
            radio("Off", mode == Some(Mode::Off), off_enabled(&caps)),
            Vec::new(),
        ),
        wrap(
            MODE_ANC,
            radio(
                "Noise Cancellation",
                mode == Some(Mode::NoiseCancellation),
                caps.has_anc,
            ),
            Vec::new(),
        ),
        wrap(
            MODE_TRANS,
            radio(
                "Transparency",
                mode == Some(Mode::Transparency),
                caps.has_transparency,
            ),
            Vec::new(),
        ),
        wrap(
            MODE_ADAPT,
            radio("Adaptive", mode == Some(Mode::Adaptive), caps.has_adaptive),
            Vec::new(),
        ),
    ]
}

// "Off" is selectable whenever the device has any noise-control mode,
// matching the daemon's set_mode(Off) gate (has_anc || has_transparency).
fn off_enabled(c: &podctl::caps::Capabilities) -> bool {
    c.has_anc || c.has_transparency
}

fn radio(label: &str, selected: bool, enabled: bool) -> Props {
    let mut p = Props::new();
    p.insert("label".into(), str_v(label));
    p.insert("toggle-type".into(), str_v("radio"));
    p.insert("toggle-state".into(), int_v(if selected { 1 } else { 0 }));
    p.insert("enabled".into(), bool_v(enabled));
    p
}

fn conv_props(state: &TrayState) -> Props {
    let mut p = Props::new();
    p.insert("label".into(), str_v("Conversation Awareness"));
    p.insert("toggle-type".into(), str_v("checkmark"));
    let on = matches!(state.conv_awareness, Some(ConvAwareness::On));
    p.insert("toggle-state".into(), int_v(if on { 1 } else { 0 }));
    // Gate on capability only, like the Mode radios — the daemon's
    // set_conv fails cleanly if the link is down, same as set_mode.
    p.insert(
        "enabled".into(),
        bool_v(state.capabilities.has_conv_awareness),
    );
    p
}

fn separator_props() -> Props {
    let mut p = Props::new();
    p.insert("type".into(), str_v("separator"));
    p
}

fn leaf_props(label: &str, enabled: bool) -> Props {
    let mut p = Props::new();
    p.insert("label".into(), str_v(label));
    p.insert("enabled".into(), bool_v(enabled));
    p
}

fn str_v(s: &str) -> OwnedValue {
    Value::new(s.to_string())
        .try_to_owned()
        .expect("string is a value")
}

fn int_v(i: i32) -> OwnedValue {
    Value::I32(i).try_to_owned().expect("i32 is a value")
}

fn bool_v(b: bool) -> OwnedValue {
    Value::Bool(b).try_to_owned().expect("bool is a value")
}
