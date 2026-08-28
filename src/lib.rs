pub mod aap;
pub mod audio;
pub mod bluez;
pub mod caps;
pub mod exitcode;
pub mod ipc;
pub mod l2cap;
pub mod model;

pub use caps::{Capabilities, Model};
pub use ipc::{OkPayload, PressSide, Request, Response, WatchRole, socket_path};
pub use model::{
    AudioState, Battery, BtState, ConvAwareness, DeviceInfo, DeviceState, EarStatus, Event, InEar,
    MicSelection, Mode, PairedDevice, PodSettings, PressAction, PressCounts, PressKind, Profile,
    Side, SpatialAudio,
};
