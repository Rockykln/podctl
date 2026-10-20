use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::{
    ConvAwareness, DeviceState, Event, MicSelection, Mode, PairedDevice, PressAction, Profile,
    SpatialAudio,
};

/// Version of the socket protocol: raised when a request or a response
/// changes shape, not for a new command. A client that gets a different
/// one knows the two halves do not fit before anything goes wrong.
pub const PROTO: u32 = 1;

pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("podctl.sock");
    }
    let uid = unsafe { libc_getuid() };
    PathBuf::from(format!("/tmp/podctl-{uid}.sock"))
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

/// What a watch connection is for. The daemon cannot tell this from the
/// event channel alone — the tray and every `podctl watch` subscribe to
/// it too, so a non-zero receiver count says nothing about the popup in
/// particular, and `show-popup` needs to know whether anyone will draw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchRole {
    /// `podctl watch`, the tray — anything that only observes.
    #[default]
    Observer,
    /// `podctl-popup`, the one process that draws the bubble.
    Popup,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Status,
    Battery,
    Ping,
    /// Who is on the other end. A daemon too old to know this answers
    /// "malformed request", which is an answer as well.
    Hello,

    SetMode {
        mode: Mode,
    },
    SetConv {
        conv: ConvAwareness,
    },
    SetSpatial {
        mode: SpatialAudio,
    },

    SetEarDetection {
        on: bool,
    },
    SetMic {
        mic: MicSelection,
    },
    SetLoudReduction {
        on: bool,
    },
    SetPressAction {
        side: PressSide,
        action: PressAction,
    },
    SetToneOnPress {
        on: bool,
    },
    Rename {
        name: String,
    },

    Connect,
    Disconnect,
    Pair {
        /// Pair this address, instead of looking for candidates.
        #[serde(default)]
        address: Option<String>,
        /// Take the single candidate found without asking back.
        #[serde(default)]
        yes: bool,
    },
    Unpair,
    List,
    SetAutoConnect {
        on: bool,
    },

    SetVolume {
        percent: u8,
    },
    SetMuted {
        muted: bool,
    },
    SetProfile {
        profile: Profile,
    },
    SetCodec {
        codec: String,
    },
    MakeDefaultSink,
    SetLatencyOffset {
        ms: i32,
    },

    SetOneBudAnc {
        on: bool,
    },
    SetChimeVolume {
        level: u8,
    },
    SetAutoAncLevel {
        level: u8,
    },

    DebugEmitCaseLid {
        open: bool,
    },
    ShowPopup,

    /// `role` is defaulted so an older client sending a bare
    /// `{"op":"watch"}` still attaches as an observer.
    Watch {
        #[serde(default)]
        role: WatchRole,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PressSide {
    Left,
    Right,
}

// `State` is the dominant variant for view-style requests; boxing it
// would buy us bytes-on-stack at the cost of an allocation on every
// happy-path response. Not worth it for a single-shot IPC over Unix.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Response {
    State(DeviceState),
    List(Vec<PairedDevice>),
    Event(Event),
    Pong,
    Hello(Hello),
    Done,
    Pending(String),
    Err(String),
}

/// The daemon's half of the handshake.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hello {
    /// The daemon's own crate version, not the client's.
    pub version: String,
    pub proto: u32,
    /// The AirPods it is acting on.
    pub device: Option<String>,
}

// Legacy alias so existing call sites keep compiling.
pub type OkPayload = Response;

impl Response {
    pub fn ok_state(s: DeviceState) -> Self {
        Response::State(s)
    }
    pub fn ok_list(v: Vec<PairedDevice>) -> Self {
        Response::List(v)
    }
    pub fn ok_done() -> Self {
        Response::Done
    }
    pub fn ok_pending(reason: impl Into<String>) -> Self {
        Response::Pending(reason.into())
    }
    pub fn ok_pong() -> Self {
        Response::Pong
    }
    pub fn ok_hello(device: Option<String>) -> Self {
        Response::Hello(Hello {
            version: env!("CARGO_PKG_VERSION").to_string(),
            proto: PROTO,
            device,
        })
    }
    pub fn ok_event(e: Event) -> Self {
        Response::Event(e)
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Response::Err(msg.into())
    }
    pub fn is_err(&self) -> bool {
        matches!(self, Response::Err(_))
    }
}
