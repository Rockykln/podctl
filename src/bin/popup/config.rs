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

/// Bounds for the tunables. A `duration_ms = 0` typo would make the
/// bubble flash and vanish; an unbounded one would pin it to the screen
/// with no way to dismiss it.
const DURATION_MIN_MS: u64 = 500;
const DURATION_MAX_MS: u64 = 60_000;
const ANIM_MAX_MS: u32 = 2_000;

#[derive(Clone, Debug)]
pub struct Config {
    pub enabled: bool,
    pub backend: Pick,
    pub theme: String,
    pub duration_ms: u64,
    pub anim_ms: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: Pick::Auto,
            theme: "dark".into(),
            // Five seconds was too tight to read the rings and register
            // the mode line, especially when the bubble is triggered by
            // a lid open you weren't looking at.
            duration_ms: 6500,
            anim_ms: 200,
        }
    }
}

pub fn path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let xdg = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    PathBuf::from(xdg).join("podctl").join("popup.toml")
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
    if let Some(v) = kv.get("theme")
        && (v == "dark" || v == "light")
    {
        cfg.theme = v.clone();
    }
    if let Some(v) = kv.get("duration_ms").and_then(|s| s.parse::<u64>().ok()) {
        cfg.duration_ms = v.clamp(DURATION_MIN_MS, DURATION_MAX_MS);
    }
    if let Some(v) = kv.get("anim_ms").and_then(|s| s.parse::<u32>().ok()) {
        cfg.anim_ms = v.min(ANIM_MAX_MS);
    }
    cfg
}

/// Total time the bubble occupies the screen: hold plus both slides.
pub fn visible_ms(cfg: &Config) -> u64 {
    cfg.duration_ms + 2 * cfg.anim_ms as u64
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
        assert_eq!(c.duration_ms, 6500);
        assert_eq!(c.anim_ms, 200);
    }

    #[test]
    fn parses_fields() {
        let text = "\
enabled = false
backend = \"x11\"
theme = light
duration_ms = 3000
anim_ms = 150
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
        assert_eq!(from_text("anim_ms = 99999").anim_ms, ANIM_MAX_MS);
        // A negative value isn't a u64 — the field keeps its default.
        assert_eq!(from_text("duration_ms = -1").duration_ms, 6500);
        assert_eq!(from_text("duration_ms = 3000").duration_ms, 3000);
    }

    #[test]
    fn visible_covers_both_slides() {
        let c = Config::default();
        assert_eq!(visible_ms(&c), c.duration_ms + 2 * c.anim_ms as u64);
    }

    #[test]
    fn backend_alias() {
        assert_eq!(Pick::parse("wayland"), Some(Pick::Wl));
        assert_eq!(Pick::parse("bogus"), None);
    }
}
