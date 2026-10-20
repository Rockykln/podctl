//! The shape of `--json`. Scripts are written against this, so it is
//! pinned here: one envelope, one line, and values spelled the way the
//! command line takes them.

use std::path::PathBuf;
use std::process::{Command, Output};

use podctl::model::{DeviceState, MicSelection, Mode, Profile};

/// A directory that does not exist, so there is no socket to find and
/// the CLI falls back to running in-process. Nothing is created, so
/// nothing is left behind either.
fn without_daemon() -> PathBuf {
    std::env::temp_dir().join(format!("podctl-no-daemon-{}", std::process::id()))
}

fn podctl(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_podctl"))
        .args(args)
        .env("XDG_RUNTIME_DIR", without_daemon())
        .output()
        .expect("run podctl")
}

fn envelope(args: &[&str]) -> serde_json::Value {
    let out = podctl(args);
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next().unwrap_or_default();
    assert!(!first.is_empty(), "{args:?} printed no JSON: {text}");
    serde_json::from_str(first).unwrap_or_else(|e| panic!("{args:?}: {e} in {first}"))
}

/// Every command answers in the same envelope, with the schema version
/// first. There used to be three shapes: `{"kind":…,"data":…}` for some,
/// `{"daemon":…}` for `version`, `{"ble":true}` for `ble`.
#[test]
fn every_command_answers_in_one_envelope() {
    for (args, kind) in [
        (&["version", "--json"][..], "version"),
        (&["ble", "--json"][..], "ble"),
        (&["ping", "--json"][..], "err"),
        (&["status", "--json"][..], "state"),
        (&["battery", "--json"][..], "battery"),
        (&["mode", "foo", "--json"][..], "err"),
    ] {
        let v = envelope(args);
        assert_eq!(v["schema"], 1, "{args:?}");
        assert_eq!(v["kind"], kind, "{args:?}");
        assert_eq!(v["ok"], kind != "err", "{args:?}");
        assert!(v["data"].is_object(), "{args:?}: data should be an object");
    }
}

/// One object per line, so a stream of them is NDJSON and `while read
/// line` works. `status --json` used to be printed over 40 lines.
#[test]
fn json_is_one_line_unless_asked_otherwise() {
    for args in [
        &["version", "--json"][..],
        &["status", "--json"][..],
        &["ble", "--json"][..],
    ] {
        let out = podctl(args);
        let text = String::from_utf8_lossy(&out.stdout);
        assert_eq!(text.trim().lines().count(), 1, "{args:?}: {text}");
    }
    let pretty = podctl(&["version", "--json", "--pretty"]);
    assert!(String::from_utf8_lossy(&pretty.stdout).lines().count() > 1);
}

/// An error goes to stdout as an envelope (stderr stays empty) with the
/// same message a person would see, and the hint with it.
#[test]
fn errors_are_envelopes_too() {
    let out = podctl(&["stauts", "--json"]);
    assert_eq!(out.status.code(), Some(64));
    assert!(out.stderr.is_empty(), "stderr should stay empty for --json");
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("error envelope on stdout");
    assert_eq!(v["ok"], false);
    assert_eq!(v["kind"], "err");
    assert_eq!(v["data"]["message"], "unknown command 'stauts'");
    assert!(
        v["data"]["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("did you mean 'status'"),
        "{v}"
    );
}

/// What comes out of `--json` has to go back in:
/// `podctl mode "$(podctl status --json | jq -r .data.settings.mode)"`
/// used to fail, because JSON said "noise_cancellation".
#[test]
fn values_can_be_fed_back_to_the_cli() {
    let mut state = DeviceState::default();
    state.settings.mode = Some(Mode::NoiseCancellation);
    state.settings.mic_selection = Some(MicSelection::AlwaysLeft);
    state.audio.profile = Some(Profile::HighQuality);
    let json = serde_json::to_value(&state).expect("serialises");
    assert_eq!(json["settings"]["mode"], "anc");
    assert_eq!(json["settings"]["mic_selection"], "left");
    assert_eq!(json["audio"]["profile"], "high");

    // And the CLI takes each of them.
    for (verb, value) in [("mode", "anc"), ("mic", "left"), ("profile", "high")] {
        let out = podctl(&[verb, value, "--json"]);
        let text = String::from_utf8_lossy(&out.stdout);
        let v: serde_json::Value = serde_json::from_str(text.lines().next().unwrap_or("{}"))
            .unwrap_or_else(|e| panic!("{verb} {value}: {e}"));
        // No AirPods and no daemon in a test, so the answer is an error
        // — but never a usage error about the value itself.
        assert_ne!(out.status.code(), Some(64), "{verb} {value}: {text}");
        assert!(v["schema"] == 1, "{verb} {value}: {v}");
    }
}
