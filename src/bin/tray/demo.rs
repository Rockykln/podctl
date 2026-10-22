//! `podctl-tray --demo`: made-up states in turn, for looking at the icon
//! and the menu without a daemon or AirPods.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use podctl::caps::Model;
use podctl::model::{Battery, Mode, Profile};
use zbus::Connection;

use crate::sni::SharedState;
use crate::state::TrayState;

/// How long each state stays up.
const STEP: Duration = Duration::from_secs(4);

static ON: AtomicBool = AtomicBool::new(false);

/// Nothing is sent to podctld while the demo runs.
pub fn active() -> bool {
    ON.load(Ordering::Relaxed)
}

/// `only` holds one state, named as in [`states`], instead of going round.
pub fn spawn(state: SharedState, conn: Connection, only: Option<String>) {
    ON.store(true, Ordering::Relaxed);
    tokio::spawn(async move {
        let threshold = state.read().await.low_battery_threshold;
        let mut all = states(threshold);
        if let Some(name) = &only {
            all.retain(|(what, _)| what == name);
            if all.is_empty() {
                let names: Vec<_> = states(threshold).into_iter().map(|(w, _)| w).collect();
                tracing::warn!("demo: no state {name:?}; there are {names:?}");
                return;
            }
        }
        loop {
            for (what, s) in all.clone() {
                tracing::info!("demo: {what}");
                *state.write().await = s;
                crate::watch::notify_changed(&state, &conn, crate::notify::get()).await;
                if only.is_some() {
                    return;
                }
                tokio::time::sleep(STEP).await;
            }
        }
    });
}

fn states(threshold: u8) -> Vec<(&'static str, TrayState)> {
    let pro = |l, r, c| {
        let mut s = TrayState::new(threshold);
        s.daemon_up = true;
        s.connected = true;
        s.paired = true;
        s.name = Some("Demo AirPods".into());
        s.model = Some(Model::Pro2UsbC.label().into());
        s.capabilities = Model::Pro2UsbC.capabilities();
        s.battery = Battery {
            left: Some(l),
            right: Some(r),
            case: Some(c),
            ..Battery::default()
        };
        s.settings.mode = Some(Mode::NoiseCancellation);
        s.audio.volume_percent = Some(50);
        s.audio.profile = Some(Profile::HighQuality);
        s.audio.is_default_sink = true;
        s
    };
    let mut charging = pro(55, 57, 80);
    charging.battery.left_charging = true;
    charging.battery.right_charging = true;
    charging.battery.case_charging = true;
    let mut plain = pro(70, 68, 40);
    plain.model = Some(Model::Gen3.label().into());
    plain.capabilities = Model::Gen3.capabilities();
    plain.settings.mode = None;
    let mut max = pro(62, 62, 0);
    max.model = Some(Model::MaxUsbC.label().into());
    max.capabilities = Model::MaxUsbC.capabilities();
    max.battery.right = None;
    max.battery.case = None;
    let mut away = pro(80, 75, 40);
    away.connected = false;
    let mut down = TrayState::new(threshold);
    down.daemon_up = false;
    vec![
        ("full", pro(100, 98, 90)),
        ("half", pro(62, 58, 40)),
        ("charging", charging),
        ("low", pro(18, 34, 40)),
        ("critical", pro(8, 30, 40)),
        ("no-anc", plain),
        ("max", max),
        ("away", away),
        ("no-daemon", down),
    ]
}
