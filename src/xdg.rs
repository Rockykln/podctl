//! Where podctl's files live. Each of these was derived again in six
//! places, and `install` wrote the no-daemon marker to `~/.config`
//! while the CLI looked for it under `XDG_CONFIG_HOME` — with a
//! non-default value, the marker had no effect.

use std::path::PathBuf;

/// Falls back to `/tmp` rather than failing: a daemon with no `HOME` is
/// better off writing somewhere than not running.
pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
}

fn dir(var: &str, default: &[&str]) -> PathBuf {
    match std::env::var(var) {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v),
        _ => {
            let mut p = home();
            for part in default {
                p.push(part);
            }
            p
        }
    }
}

pub fn config_home() -> PathBuf {
    dir("XDG_CONFIG_HOME", &[".config"])
}

pub fn data_home() -> PathBuf {
    dir("XDG_DATA_HOME", &[".local", "share"])
}

pub fn state_home() -> PathBuf {
    dir("XDG_STATE_HOME", &[".local", "state"])
}

/// `$XDG_RUNTIME_DIR`, which has no fallback: what goes there must not
/// outlive the session or be seen by other users.
pub fn runtime_home() -> Option<PathBuf> {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
}

/// Whether the session is GNOME, whose panel and tray want their own
/// handling.
pub fn on_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|d| d.split(':').any(|part| part.eq_ignore_ascii_case("gnome")))
}

/// `~/.config/podctl`, where `daemon.toml`, `tray.toml` and
/// `popup.toml` live.
pub fn config_dir() -> PathBuf {
    config_home().join("podctl")
}

/// `~/.local/state/podctl`, for the proximity keys and the ducking
/// note — data podctl wrote itself and can live without.
pub fn state_dir() -> PathBuf {
    state_home().join("podctl")
}

#[cfg(test)]
mod tests {
    use super::dir;
    use std::path::PathBuf;

    /// An empty variable is not a path: `XDG_CONFIG_HOME=` means "not
    /// set" by the spec, and would otherwise put files in `/podctl`.
    #[test]
    fn an_empty_variable_falls_back() {
        let key = "PODCTL_TEST_XDG";
        unsafe { std::env::set_var(key, "") };
        assert!(dir(key, &[".config"]).ends_with(".config"));
        unsafe { std::env::set_var(key, "   ") };
        assert!(dir(key, &[".config"]).ends_with(".config"));
        unsafe { std::env::set_var(key, "/somewhere") };
        assert_eq!(dir(key, &[".config"]), PathBuf::from("/somewhere"));
        unsafe { std::env::remove_var(key) };
        assert!(dir(key, &[".config"]).ends_with(".config"));
    }
}
