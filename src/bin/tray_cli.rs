use std::path::Path;
use std::process::Command;

use podctl::exitcode;

// Only part of it is needed here; the tray uses the rest.
#[allow(dead_code)]
#[path = "tray/config.rs"]
mod tray_config;

const UNIT: &str = "podctl-tray";

pub fn run(args: &[String]) -> i32 {
    let verb = args.first().map(String::as_str).unwrap_or("status");
    match verb {
        "start" => systemctl(&["enable", "--now", UNIT]),
        "stop" => systemctl(&["disable", "--now", UNIT]),
        "restart" => systemctl(&["restart", UNIT]),
        "status" => status(),
        "help" | "-h" | "--help" => {
            print_help();
            exitcode::OK
        }
        other => {
            eprintln!(
                "podctl: tray: unknown value '{other}' (expected start, stop, status, restart)"
            );
            eprintln!("hint: see 'podctl help tray'");
            exitcode::USAGE
        }
    }
}

fn print_help() {
    println!("usage: podctl tray <start|stop|restart|status>");
    println!();
    println!("  start    enable + start the podctl-tray user service");
    println!("  stop     stop the service and disable it for the next login");
    println!("  restart  restart the service");
    println!("  status   show service state, watcher presence, and config");
}

fn systemctl(args: &[&str]) -> i32 {
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user");
    cmd.args(args);
    match cmd.status() {
        Ok(s) if s.success() => exitcode::OK,
        Ok(_) => exitcode::UNAVAILABLE,
        Err(e) => {
            eprintln!("podctl: tray: systemctl: {e}");
            exitcode::OSERR
        }
    }
}

fn status() -> i32 {
    let installed = unit_installed(UNIT);
    let active = systemctl_is("is-active", UNIT);
    let enabled = systemctl_is("is-enabled", UNIT);
    let watcher = watcher_present();
    let cfg_path = tray_config::path();
    let cfg_exists = cfg_path.exists();
    let cfg_summary = read_config_summary(&cfg_path);

    if installed == Some(false) {
        // `is-active` reports a unit that does not exist as plain
        // "inactive", which reads like a service one `start` away.
        println!("service:    not installed");
        println!("            install it with: podctl install --with-tray");
    } else {
        println!("service:    {}", active.as_deref().unwrap_or("unknown"));
        println!("enabled:    {}", enabled.as_deref().unwrap_or("unknown"));
    }
    println!(
        "watcher:    {}",
        match watcher {
            Some(true) => "present (org.kde.StatusNotifierWatcher on the bus)",
            Some(false) => "missing (GNOME without an extension does not provide one)",
            None => "unknown (dbus-send not found)",
        }
    );
    println!(
        "config:     {}{}",
        cfg_path.display(),
        if cfg_exists {
            ""
        } else {
            "  (missing — defaults apply)"
        }
    );
    if let Some(s) = cfg_summary {
        println!("            {s}");
    }
    if matches!(watcher, Some(false)) {
        println!();
        println!(
            "hint: on GNOME, install the KStatusNotifierItem/AppIndicator extension \
                 (https://extensions.gnome.org/extension/615/appindicator-support/)."
        );
    }
    exitcode::OK
}

/// `None` when systemctl is missing entirely.
fn unit_installed(unit: &str) -> Option<bool> {
    let status = Command::new("systemctl")
        .env("LC_ALL", "C")
        .args(["--user", "cat", unit])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    Some(status.success())
}

fn systemctl_is(verb: &str, unit: &str) -> Option<String> {
    let out = Command::new("systemctl")
        .env("LC_ALL", "C")
        .args(["--user", verb, unit])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn watcher_present() -> Option<bool> {
    let out = Command::new("dbus-send")
        .env("LC_ALL", "C")
        .args([
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.NameHasOwner",
            "string:org.kde.StatusNotifierWatcher",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return Some(false);
    }
    let s = String::from_utf8_lossy(&out.stdout);
    Some(s.contains("boolean true"))
}

fn read_config_summary(path: &Path) -> Option<String> {
    let cfg = tray_config::from_text(&std::fs::read_to_string(path).ok()?);
    Some(format!(
        "left_click={} low_battery_threshold={} notify_threshold={}",
        cfg.left_click.as_str(),
        cfg.low_battery_threshold,
        cfg.notify_threshold,
    ))
}
