//! The daemon's settings file, `~/.config/podctl/daemon.toml`.

use std::io::Write;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Listen for the case's Bluetooth LE advertisement while the buds
    /// are disconnected.
    pub ble: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { ble: true }
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
        if k.trim() == "ble"
            && let Some(on) = parse_bool(v)
        {
            cfg.ble = on;
        }
    }
    cfg
}

/// Write the `ble` line, keeping whatever else the file holds.
pub fn set_ble(on: bool) -> std::io::Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let old = std::fs::read_to_string(&p).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in old.lines() {
        let key = line
            .split('#')
            .next()
            .unwrap_or("")
            .split('=')
            .next()
            .unwrap_or("")
            .trim();
        if key == "ble" {
            lines.push(format!("ble = {on}"));
            replaced = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if !replaced {
        lines.push(format!("ble = {on}"));
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
}
