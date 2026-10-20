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

/// Surplus arguments used to be dropped without a word, so
/// `podctl rename Test AirPods` renamed the buds to "Test".
#[test]
fn surplus_arguments_are_a_usage_error() {
    for args in [
        &["status", "foo"][..],
        &["mode", "anc", "extra"][..],
        &["volume", "50", "60"][..],
        &["rename", "Test", "AirPods"][..],
    ] {
        let out = podctl(args);
        assert_eq!(out.status.code(), Some(64), "{args:?} exit status");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("takes"), "{args:?}: {err}");
    }
}

#[test]
fn an_option_the_verb_does_not_take_is_refused() {
    let out = podctl(&["mode", "--bogus"]);
    assert_eq!(out.status.code(), Some(64));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown option '--bogus'"), "{err}");
}

/// A typo should point at the command that was meant.
#[test]
fn a_typo_gets_a_suggestion() {
    let out = podctl(&["stauts"]);
    assert_eq!(out.status.code(), Some(64));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown command 'stauts'"), "{err}");
    assert!(err.contains("did you mean 'status'"), "{err}");
}

/// `podctl help nope` wrote to stderr and still exited 0.
#[test]
fn help_for_an_unknown_topic_is_a_usage_error() {
    let out = podctl(&["help", "nope"]);
    assert_eq!(out.status.code(), Some(64));
}

/// Every verb in the table has a help page of its own; a missing one
/// would fall through to the "no help for" branch and exit 64.
#[test]
fn every_command_has_a_help_page() {
    for c in podctl::commands::COMMANDS {
        let out = podctl(&["help", c.name]);
        assert_eq!(out.status.code(), Some(0), "help {}", c.name);
        let first = String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        assert!(
            first.starts_with(&format!("podctl {}", c.name)) || c.name == "help",
            "help {} starts with {first:?}",
            c.name
        );
    }
}

/// The overview must name every command, including the ones whose
/// settings are not implemented yet — marked as such.
#[test]
fn the_overview_lists_every_command() {
    let out = podctl(&["help"]);
    let text = String::from_utf8_lossy(&out.stdout);
    for c in podctl::commands::COMMANDS {
        assert!(
            text.contains(c.name),
            "'{}' missing from the overview",
            c.name
        );
    }
    assert_eq!(
        text.matches("(not implemented yet)").count(),
        4,
        "four commands are not implemented; the overview should say so four times"
    );
}
