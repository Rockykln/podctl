//! Promises of the command line that are easy to break from the inside:
//! `--help` prints a page instead of doing the work, and a command that
//! needs the daemon fails when there is none.

use std::path::PathBuf;
use std::process::{Command, Output};

/// An empty directory as `XDG_RUNTIME_DIR`, so the CLI finds no socket
/// and falls back to running in-process.
fn without_daemon() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("podctl-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn podctl(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_podctl"))
        .args(args)
        .env("XDG_RUNTIME_DIR", without_daemon())
        .output()
        .expect("run podctl")
}

/// `debug --help` used to print a full diagnostic report, `meter --help`
/// went looking for a sink and exited 69, and `completion --help`
/// answered "unknown shell '--help'".
#[test]
fn help_prints_a_page_instead_of_running_the_command() {
    for verb in [
        "status",
        "ping",
        "meter",
        "debug",
        "completion",
        "version",
        "ble",
        "watch",
    ] {
        let out = podctl(&[verb, "--help"]);
        let first = String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        assert_eq!(out.status.code(), Some(0), "{verb} --help exit status");
        assert!(
            first.starts_with(&format!("podctl {verb} —")),
            "{verb} --help printed {first:?}"
        );
    }
}

#[test]
fn ping_without_a_daemon_reports_it() {
    let out = podctl(&["ping"]);
    assert_eq!(out.status.code(), Some(69), "sysexits EX_UNAVAILABLE");
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("daemon is not running"), "{text:?}");
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("pong"),
        "answered pong without a daemon"
    );
}
