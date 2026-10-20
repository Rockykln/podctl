//! The daemon's settings file, `~/.config/podctl/daemon.toml`.

use std::io::Write;
use std::path::PathBuf;

use crate::model::{MicSelection, PodSettings};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Listen for the case's Bluetooth LE advertisement while the buds
    /// are disconnected.
    pub ble: bool,
    /// Pause on a bud leaving the ear. The buds have a switch for this
    /// too, but the Pro 2 leave it out of their settings dump, so this
    /// file is what podctl goes by.
    pub ear_detection: bool,
    /// Which bud's microphone to use. Also absent from the dump.
    pub mic: MicSelection,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ble: true,
            ear_detection: true,
            mic: MicSelection::Auto,
        }
    }
}

impl Config {
    /// The settings podctl answers for itself. The rest of `PodSettings`
    /// stays unknown until the device reports it.
    pub fn pod_settings(&self) -> PodSettings {
        PodSettings {
            ear_detection: Some(self.ear_detection),
            mic_selection: Some(self.mic),
            ..PodSettings::default()
        }
    }
}

pub fn path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let xdg = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    PathBuf::from(xdg).join("podctl").join("daemon.toml")
}

/// Read fresh every time it is needed: the file is small, and a change
/// then takes effect without restarting the daemon.
pub fn load() -> Config {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Config::default();
    };
    from_text(&text)
}

fn from_text(text: &str) -> Config {
    let mut cfg = Config::default();
    for raw in text.lines() {
        let trimmed = raw.split('#').next().unwrap_or("").trim();
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"').trim_matches('\'');
        match k.trim() {
            "ble" => {
                if let Some(on) = parse_bool(v) {
                    cfg.ble = on;
                }
            }
            "ear_detection" => {
                if let Some(on) = parse_bool(v) {
                    cfg.ear_detection = on;
                }
            }
            "mic" => {
                if let Ok(mic) = v.parse() {
                    cfg.mic = mic;
                }
            }
            _ => {}
        }
    }
    cfg
}

pub fn set_ble(on: bool) -> std::io::Result<()> {
    set("ble", &on.to_string())
}

pub fn set_ear_detection(on: bool) -> std::io::Result<()> {
    set("ear_detection", &on.to_string())
}

pub fn set_mic(mic: MicSelection) -> std::io::Result<()> {
    set("mic", mic.as_str())
}

/// Write one key, keeping whatever else the file holds.
fn set(key: &str, value: &str) -> std::io::Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let old = std::fs::read_to_string(&p).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in old.lines() {
        let line_key = line
            .split('#')
            .next()
            .unwrap_or("")
            .split('=')
            .next()
            .unwrap_or("")
            .trim();
        if line_key == key {
            lines.push(format!("{key} = {value}"));
            replaced = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if !replaced {
        lines.push(format!("{key} = {value}"));
    }
    let mut f = std::fs::File::create(&p)?;
    for line in lines {
        writeln!(f, "{line}")?;
    }
    Ok(())
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "true" | "yes" | "1" | "on" => Some(true),
        "false" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, from_text};
    use crate::model::MicSelection;

    #[test]
    fn listens_unless_told_otherwise() {
        assert!(Config::default().ble);
        assert!(from_text("").ble);
        assert!(from_text("ble = maybe").ble);
        assert!(from_text("other = false").ble);
    }

    #[test]
    fn reads_the_switch() {
        assert!(!from_text("ble = false").ble);
        assert!(!from_text("  ble=off  # no scanning").ble);
        assert!(!from_text("ble = \"no\"").ble);
        assert!(from_text("ble = on").ble);
    }

    /// The buds don't report these two, so the file has to answer — and
    /// an answer the file doesn't give is "on" and "auto", never unknown.
    #[test]
    fn ear_detection_and_mic_default_to_on_and_auto() {
        let cfg = from_text("");
        assert!(cfg.ear_detection);
        assert_eq!(cfg.mic, MicSelection::Auto);
        let s = cfg.pod_settings();
        assert_eq!(s.ear_detection, Some(true));
        assert_eq!(s.mic_selection, Some(MicSelection::Auto));
    }

    #[test]
    fn reads_ear_detection_and_mic() {
        let cfg = from_text("ear_detection = false\nmic = left\nble = false\n");
        assert!(!cfg.ear_detection);
        assert_eq!(cfg.mic, MicSelection::AlwaysLeft);
        assert!(!cfg.ble);
        assert_eq!(from_text("mic = nonsense").mic, MicSelection::Auto);
        assert!(from_text("ear_detection = nonsense").ear_detection);
    }
}
