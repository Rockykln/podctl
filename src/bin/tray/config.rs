use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftClick {
    Popup,
    ModeCycle,
    ToggleAncTr,
    Menu,
}

impl LeftClick {
    pub fn as_str(self) -> &'static str {
        match self {
            LeftClick::Popup => "popup",
            LeftClick::ModeCycle => "mode-cycle",
            LeftClick::ToggleAncTr => "toggle-anc-tr",
            LeftClick::Menu => "menu",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "popup" => Some(LeftClick::Popup),
            "mode-cycle" => Some(LeftClick::ModeCycle),
            "toggle-anc-tr" | "toggle" => Some(LeftClick::ToggleAncTr),
            "menu" => Some(LeftClick::Menu),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub left_click: LeftClick,
    pub low_battery_threshold: u8,
    pub notify_threshold: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            left_click: LeftClick::Popup,
            low_battery_threshold: 20,
            notify_threshold: 10,
        }
    }
}

pub fn path() -> PathBuf {
    podctl::xdg::config_dir().join("tray.toml")
}

pub fn load() -> Config {
    std::fs::read_to_string(path())
        .map(|text| from_text(&text))
        .unwrap_or_default()
}

/// `low_battery_threshold` is the first warning and the icon's
/// attention state, `notify_threshold` the critical one below it.
pub fn from_text(text: &str) -> Config {
    let kv = parse_flat(text);
    let mut cfg = Config::default();
    if let Some(v) = kv.get("left_click").and_then(|s| LeftClick::parse(s)) {
        cfg.left_click = v;
    }
    if let Some(v) = kv
        .get("low_battery_threshold")
        .and_then(|s| s.parse::<u8>().ok())
    {
        cfg.low_battery_threshold = v.min(100);
    }
    if let Some(v) = kv
        .get("notify_threshold")
        .and_then(|s| s.parse::<u8>().ok())
    {
        cfg.notify_threshold = v.min(cfg.low_battery_threshold);
    }
    cfg
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
    fn defaults_when_missing() {
        let kv: HashMap<String, String> = HashMap::new();
        let _ = kv;
        let c = Config::default();
        assert_eq!(c.left_click, LeftClick::Popup);
        assert_eq!(c.low_battery_threshold, 20);
        assert_eq!(c.notify_threshold, 10);
    }

    #[test]
    fn parses_each_field() {
        let text = "\
left_click = \"toggle-anc-tr\"
low_battery_threshold = 25
notify_threshold = 5
# comment
";
        let kv = parse_flat(text);
        assert_eq!(kv.get("left_click").unwrap(), "toggle-anc-tr");
        assert_eq!(kv.get("low_battery_threshold").unwrap(), "25");
        assert_eq!(kv.get("notify_threshold").unwrap(), "5");
    }

    #[test]
    fn levels_stay_in_order() {
        let c = from_text("low_battery_threshold = 150\nnotify_threshold = 120");
        assert_eq!((c.low_battery_threshold, c.notify_threshold), (100, 100));
        let c = from_text("low_battery_threshold = 15\nnotify_threshold = 30");
        assert_eq!((c.low_battery_threshold, c.notify_threshold), (15, 15));
    }

    #[test]
    fn skips_garbage() {
        let text = "no equals\n# comment only\n\nkey=value";
        let kv = parse_flat(text);
        assert_eq!(kv.len(), 1);
        assert_eq!(kv.get("key").unwrap(), "value");
    }

    #[test]
    fn left_click_alias() {
        assert_eq!(LeftClick::parse("toggle"), Some(LeftClick::ToggleAncTr));
        assert_eq!(LeftClick::parse("popup"), Some(LeftClick::Popup));
        assert_eq!(LeftClick::parse("bogus"), None);
    }
}
