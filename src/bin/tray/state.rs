use std::time::Instant;

use podctl::caps::Capabilities;
use podctl::model::{AudioState, Battery, Mode, PodSettings};

use crate::icons;

#[derive(Clone, Debug, Default)]
pub struct TrayState {
    /// Whether podctld answered last time we asked. Without it a dead
    /// daemon looked exactly like AirPods that are switched off.
    pub daemon_up: bool,
    pub connected: bool,
    /// Whether the daemon has a pair of AirPods to connect to at all.
    pub paired: bool,
    pub name: Option<String>,
    pub model: Option<String>,
    pub battery: Battery,
    pub settings: PodSettings,
    pub audio: AudioState,
    pub capabilities: Capabilities,
    pub low_battery_threshold: u8,
    /// When the last full status came in.
    pub pulled: Option<Instant>,
}

impl TrayState {
    pub fn new(low_battery_threshold: u8) -> Self {
        Self {
            low_battery_threshold,
            ..Self::default()
        }
    }

    /// Everything that only holds while the link is up.
    pub fn forget_link(&mut self) {
        self.connected = false;
        self.battery = Battery::default();
        self.settings = PodSettings::default();
        self.audio = AudioState::default();
    }

    pub fn icon_name(&self) -> String {
        if icons::theme_path().is_none() {
            // Theme icons only: never battery-*, which the theme draws
            // as the system's PC battery.
            return if self.connected {
                "audio-headphones"
            } else {
                "audio-headphones-symbolic"
            }
            .into();
        }
        if !self.daemon_up || !self.connected {
            return icons::DISCONNECTED.into();
        }
        let b = &self.battery;
        let buds = [(b.left, b.left_charging), (b.right, b.right_charging)];
        let known = buds.iter().filter_map(|&(p, c)| p.map(|p| (p, c)));
        match known.clone().map(|(p, _)| p).min() {
            // Charging only once every bud that reports is: one in the
            // case and one in the ear still runs down.
            Some(lowest) => icons::level(lowest, known.clone().all(|(_, c)| c)),
            None => icons::PLAIN.into(),
        }
    }

    pub fn status(&self) -> &'static str {
        if self.connected
            && [
                (self.battery.left, self.battery.left_charging),
                (self.battery.right, self.battery.right_charging),
            ]
            .into_iter()
            .any(|(pct, charging)| !charging && pct.is_some_and(|p| p < self.low_battery_threshold))
        {
            "NeedsAttention"
        } else {
            "Active"
        }
    }

    pub fn title(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.model.clone())
            .unwrap_or_else(|| "AirPods".into())
    }

    /// The icon shown while the status is `NeedsAttention`. Hosts swap
    /// to it on their own, which is the only way a low level shows up
    /// without the theme turning the headphones into a PC battery.
    pub fn attention_icon_name(&self) -> String {
        if icons::theme_path().is_some() {
            // Our own icon already turns red when low.
            self.icon_name()
        } else {
            "battery-caution-symbolic".into()
        }
    }

    pub fn tooltip(&self) -> (String, String) {
        if !self.daemon_up {
            return ("podctl".into(), "podctld is not running".into());
        }
        let mut lines = Vec::with_capacity(2);
        lines.extend(self.levels());
        if self.connected {
            lines.push(match self.settings.mode {
                Some(m) => format!("{} · Connected", mode_label(m)),
                None => "Connected".into(),
            });
        } else {
            // Levels without a link come from the case announcing
            // itself — the buds are lying in it, not gone.
            lines.push("Not connected".into());
        }
        (self.title(), lines.join("\n"))
    }

    /// Battery levels as one line, or `None` while nothing is known.
    pub fn levels(&self) -> Option<String> {
        let mut parts = Vec::with_capacity(3);
        if let Some(l) = self.battery.left {
            parts.push(format_part("L", l, self.battery.left_charging));
        }
        if let Some(r) = self.battery.right {
            parts.push(format_part("R", r, self.battery.right_charging));
        }
        if let Some(c) = self.battery.case {
            parts.push(format_part("Case", c, self.battery.case_charging));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

pub fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Off => "Off",
        Mode::NoiseCancellation => "Noise Cancellation",
        Mode::Transparency => "Transparency",
        Mode::Adaptive => "Adaptive",
    }
}

fn format_part(label: &str, pct: u8, charging: bool) -> String {
    if charging {
        format!("{label} {pct}% ⚡")
    } else {
        format!("{label} {pct}%")
    }
}
