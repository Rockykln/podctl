use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use podctl::{
    Response, audio, bluez,
    caps::Capabilities,
    ipc::Request,
    model::{DeviceState, PairedDevice, Profile},
};

static BANNER_SHOWN: AtomicBool = AtomicBool::new(false);

pub fn dispatch(req: &Request) -> Response {
    show_banner_once();
    match req {
        Request::Status => status(),
        Request::Battery => status(),
        Request::Ping => Response::ok_pong(),

        Request::SetVolume { percent } => audio_volume(*percent),
        Request::SetMuted { muted } => audio_mute(*muted),
        Request::SetProfile { profile } => audio_profile(*profile),
        Request::SetCodec { codec } => audio_codec(codec),
        Request::MakeDefaultSink => audio_default(),
        Request::SetLatencyOffset { ms } => audio_latency(*ms),

        Request::Connect => bt_connect(),
        Request::Disconnect => bt_disconnect(),
        Request::Pair => bt_pair(),
        Request::Unpair => bt_unpair(),
        Request::List => bt_list(),
        Request::SetAutoConnect { on } => bt_auto(*on),
        Request::Rename { name } => bt_rename(name),

        Request::SetMode { .. }
        | Request::SetConv { .. }
        | Request::SetSpatial { .. }
        | Request::SetEarDetection { .. }
        | Request::SetMic { .. }
        | Request::SetLoudReduction { .. }
        | Request::SetPressAction { .. }
        | Request::SetToneOnPress { .. }
        | Request::SetOneBudAnc { .. }
        | Request::SetChimeVolume { .. }
        | Request::SetAutoAncLevel { .. } => Response::err(
            "this setting changes a value on the AirPods themselves — needs the daemon. \
             run 'podctl setup' to install.",
        ),

        Request::Watch { .. } => {
            Response::err("watch needs the daemon — install it with 'podctl setup'.")
        }

        Request::DebugEmitCaseLid { .. } => {
            Response::err("debug emit-case-lid needs the daemon (it broadcasts on the event bus).")
        }

        Request::ShowPopup => {
            Response::err("popup needs the daemon (it broadcasts on the event bus).")
        }
    }
}

fn show_banner_once() {
    if BANNER_SHOWN.swap(true, Ordering::Relaxed) {
        return;
    }
    if std::env::var_os("NO_BANNER").is_some() {
        return;
    }
    if no_daemon_marker().map(|p| p.exists()).unwrap_or(false) {
        return;
    }
    if !stderr_is_tty() {
        return;
    }

    let _ = writeln!(
        std::io::stderr(),
        "note: running without daemon — some commands disabled, no live state. \
         install with 'podctl setup'."
    );
}

fn stderr_is_tty() -> bool {
    unsafe { libc_isatty(2) != 0 }
}

unsafe extern "C" {
    #[link_name = "isatty"]
    fn libc_isatty(fd: i32) -> i32;
}

fn no_daemon_marker() -> Option<std::path::PathBuf> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("podctl").join("no-daemon"))
}

fn status() -> Response {
    let mut state = DeviceState::default();
    if let Some(dev) = bluez::primary_airpods() {
        state.address = Some(dev.address.clone());
        state.name = Some(dev.name.clone());
        state.connected = dev.connected;
        state.capabilities = dev.capabilities();
        state.bluetooth = bluez::bt_state(&dev);
    } else {
        state.capabilities = Capabilities::default();
    }
    state.audio = audio::snapshot();
    state.updated_at = now_secs();
    Response::ok_state(state)
}

fn audio_volume(pct: u8) -> Response {
    let Some(sink) = audio::primary_sink() else {
        return Response::err("no AirPods audio sink found — connect them first.");
    };
    match audio::set_volume(&sink, pct) {
        Ok(()) => Response::ok_done(),
        Err(e) => Response::err(format!("{e}")),
    }
}

fn audio_mute(muted: bool) -> Response {
    let Some(sink) = audio::primary_sink() else {
        return Response::err("no AirPods audio sink found.");
    };
    match audio::set_muted(&sink, muted) {
        Ok(()) => Response::ok_done(),
        Err(e) => Response::err(format!("{e}")),
    }
}

fn audio_profile(profile: Profile) -> Response {
    let Some(card) = audio::primary_card() else {
        return Response::err("no AirPods card found.");
    };
    match audio::set_profile(&card, profile) {
        Ok(()) => Response::ok_done(),
        Err(e) => Response::err(format!("{e}")),
    }
}

fn audio_codec(codec: &str) -> Response {
    let Some(card) = audio::primary_card() else {
        return Response::err("no AirPods card found.");
    };
    match audio::set_codec(&card, codec) {
        Ok(()) => Response::ok_done(),
        Err(e) => Response::err(format!("{e}")),
    }
}

fn audio_default() -> Response {
    let Some(sink) = audio::primary_sink() else {
        return Response::err("no AirPods audio sink found.");
    };
    match audio::set_default(&sink) {
        Ok(()) => Response::ok_done(),
        Err(e) => Response::err(format!("{e}")),
    }
}

fn audio_latency(ms: i32) -> Response {
    let Some(card) = audio::primary_card() else {
        return Response::err("no AirPods card found.");
    };
    match audio::set_latency_offset(&card, ms) {
        Ok(()) => Response::ok_done(),
        Err(e) => Response::err(format!("{e}")),
    }
}

fn bt_connect() -> Response {
    let Some(dev) = bluez::primary_airpods() else {
        return Response::err("no AirPods paired — run 'podctl pair' first.");
    };
    match bluez::connect(&dev.address) {
        Ok(()) => Response::ok_done(),
        Err(e) => Response::err(format!("{e}")),
    }
}

fn bt_disconnect() -> Response {
    let Some(dev) = bluez::primary_airpods() else {
        return Response::err("no AirPods paired.");
    };
    match bluez::disconnect(&dev.address) {
        Ok(()) => Response::ok_done(),
        Err(e) => Response::err(format!("{e}")),
    }
}

fn bt_pair() -> Response {
    let found = match bluez::discover(12) {
        Ok(v) => v,
        Err(e) => return Response::err(format!("scan failed: {e}")),
    };
    let target = match found.into_iter().find(|d| !d.paired) {
        Some(d) => d,
        None => {
            return Response::err(
                "no new AirPods spotted — make sure the case is open and the LED blinks white.",
            );
        }
    };
    match bluez::pair(&target.address) {
        Ok(()) => Response::ok_done(),
        Err(e) => Response::err(format!("{e}")),
    }
}

fn bt_unpair() -> Response {
    let Some(dev) = bluez::primary_airpods() else {
        return Response::err("nothing to unpair.");
    };
    match bluez::unpair(&dev.address) {
        Ok(()) => Response::ok_done(),
        Err(e) => Response::err(format!("{e}")),
    }
}

fn bt_list() -> Response {
    let items = bluez::paired_airpods()
        .unwrap_or_default()
        .iter()
        .map(bluez::to_paired_device)
        .collect::<Vec<PairedDevice>>();
    Response::ok_list(items)
}

fn bt_auto(on: bool) -> Response {
    let Some(dev) = bluez::primary_airpods() else {
        return Response::err("no AirPods paired.");
    };
    match bluez::set_trusted(&dev.address, on) {
        Ok(()) => Response::ok_done(),
        Err(e) => Response::err(format!("{e}")),
    }
}

fn bt_rename(name: &str) -> Response {
    if name.trim().is_empty() {
        return Response::err("name cannot be empty");
    }
    if name.len() > 64 {
        return Response::err("name too long (max 64 bytes UTF-8)");
    }
    let Some(dev) = bluez::primary_airpods() else {
        return Response::err("no AirPods paired.");
    };
    match bluez::set_alias(&dev.address, name) {
        Ok(()) => Response::ok_done(),
        Err(e) => Response::err(format!("{e}")),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
