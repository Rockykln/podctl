use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Auto,
    Wl,
    X11,
    Notify,
}

impl Pick {
    pub fn as_str(self) -> &'static str {
        match self {
            Pick::Auto => "auto",
            Pick::Wl => "wl",
            Pick::X11 => "x11",
            Pick::Notify => "notify",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Pick::Auto),
            "wl" | "wayland" => Some(Pick::Wl),
            "x11" => Some(Pick::X11),
            "notify" => Some(Pick::Notify),
            _ => None,
        }
    }
}

/// `auto` follows the desktop: GNOME's animations switch, KDE's
/// animation speed, the portal's reduced-motion key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionPick {
    Auto,
    Full,
    Reduced,
}

impl MotionPick {
    pub fn as_str(self) -> &'static str {
        match self {
            MotionPick::Auto => "auto",
            MotionPick::Full => "full",
            MotionPick::Reduced => "reduced",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(MotionPick::Auto),
            "full" => Some(MotionPick::Full),
            "reduced" => Some(MotionPick::Reduced),
            _ => None,
        }
    }
}

/// `auto` follows the desktop's light/dark preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePick {
    Auto,
    Dark,
    Light,
}

impl ThemePick {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemePick::Auto => "auto",
            ThemePick::Dark => "dark",
            ThemePick::Light => "light",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(ThemePick::Auto),
            "dark" => Some(ThemePick::Dark),
            "light" => Some(ThemePick::Light),
            _ => None,
        }
    }
}

/// Bounds for the tunables. A `duration_ms = 0` typo would make the
/// bubble flash and vanish; an unbounded one would pin it to the screen
/// with no way to dismiss it.
const DURATION_MIN_MS: u64 = 500;
const DURATION_MAX_MS: u64 = 60_000;

#[derive(Clone, Debug)]
pub struct Config {
    pub enabled: bool,
    pub backend: Pick,
    pub theme: ThemePick,
    pub motion: MotionPick,
    pub duration_ms: u64,
    /// Output (e.g. `eDP-1`) to show the bubble on. `None` leaves it to
    /// the compositor on Wayland; on X11 the monitor under the pointer,
    /// then the RandR primary.
    pub output: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: Pick::Auto,
            theme: ThemePick::Auto,
            motion: MotionPick::Auto,
            // Five seconds was too tight to read the rings and register
            // the mode line; six and a half then outstayed its welcome
            // on a bubble that now opens with the lid.
            duration_ms: 5500,
            output: None,
        }
    }
}

pub fn path() -> PathBuf {
    podctl::xdg::config_dir().join("popup.toml")
}

pub fn load() -> Config {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Config::default();
    };
    from_text(&text)
}

fn from_text(text: &str) -> Config {
    let kv = parse_flat(text);
    let mut cfg = Config::default();
    if let Some(v) = kv.get("enabled").and_then(|s| parse_bool(s)) {
        cfg.enabled = v;
    }
    if let Some(v) = kv.get("backend").and_then(|s| Pick::parse(s)) {
        cfg.backend = v;
    }
    if let Some(v) = kv.get("theme").and_then(|s| ThemePick::parse(s)) {
        cfg.theme = v;
    }
    if let Some(v) = kv.get("motion").and_then(|s| MotionPick::parse(s)) {
        cfg.motion = v;
    }
    if let Some(v) = kv.get("duration_ms").and_then(|s| s.parse::<u64>().ok()) {
        cfg.duration_ms = v.clamp(DURATION_MIN_MS, DURATION_MAX_MS);
    }
    if let Some(v) = kv.get("output")
        && !v.is_empty()
    {
        cfg.output = Some(v.clone());
    }
    cfg
}

/// Total time the bubble occupies the screen: hold plus entry and exit.
pub fn visible_ms(cfg: &Config) -> u64 {
    cfg.duration_ms + (crate::anim::ENTRY_MS + crate::anim::EXIT_MS) as u64
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "true" | "yes" | "1" | "on" => Some(true),
        "false" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
}

fn parse_flat(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for raw in text.lines() {
        let trimmed = raw.split('#').next().unwrap_or("").trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"').trim_matches('\'');
        out.insert(k.trim().to_string(), v.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::default();
        assert!(c.enabled);
        assert_eq!(c.backend, Pick::Auto);
        assert_eq!(c.duration_ms, 5500);
    }

    #[test]
    fn parses_fields() {
        let text = "\
enabled = false
backend = \"x11\"
theme = light
duration_ms = 3000
";
        let kv = parse_flat(text);
        assert_eq!(kv.get("backend").unwrap(), "x11");
        assert_eq!(Pick::parse(kv.get("backend").unwrap()), Some(Pick::X11));
        assert_eq!(parse_bool(kv.get("enabled").unwrap()), Some(false));
        assert_eq!(kv.get("duration_ms").unwrap(), "3000");
    }

    #[test]
    fn clamps_out_of_range_durations() {
        assert_eq!(from_text("duration_ms = 0").duration_ms, DURATION_MIN_MS);
        assert_eq!(
            from_text("duration_ms = 999999").duration_ms,
            DURATION_MAX_MS
        );
        // A negative value isn't a u64 — the field keeps its default.
        assert_eq!(from_text("duration_ms = -1").duration_ms, 5500);
        assert_eq!(from_text("duration_ms = 3000").duration_ms, 3000);
    }

    #[test]
    fn visible_covers_entry_and_exit() {
        let c = Config::default();
        assert_eq!(visible_ms(&c), c.duration_ms + 980);
    }

    #[test]
    fn parses_output() {
        assert_eq!(
            from_text("output = \"eDP-1\"\n").output.as_deref(),
            Some("eDP-1")
        );
        assert_eq!(from_text("output = \"\"\n").output, None);
        assert_eq!(Config::default().output, None);
    }

    #[test]
    fn theme_defaults_to_auto() {
        assert_eq!(Config::default().theme, ThemePick::Auto);
        assert_eq!(from_text("theme = light").theme, ThemePick::Light);
        assert_eq!(from_text("theme = \"dark\"").theme, ThemePick::Dark);
        assert_eq!(from_text("theme = sepia").theme, ThemePick::Auto);
    }

    #[test]
    fn motion_defaults_to_auto() {
        assert_eq!(Config::default().motion, MotionPick::Auto);
        assert_eq!(from_text("motion = reduced").motion, MotionPick::Reduced);
        assert_eq!(from_text("motion = \"full\"").motion, MotionPick::Full);
        assert_eq!(from_text("motion = none").motion, MotionPick::Auto);
    }

    #[test]
    fn backend_alias() {
        assert_eq!(Pick::parse("wayland"), Some(Pick::Wl));
        assert_eq!(Pick::parse("bogus"), None);
    }
}
