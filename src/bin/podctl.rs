use std::process::ExitCode;
use std::str::FromStr;

use podctl::{
    PressSide, Response, WatchRole, client, commands, exitcode,
    ipc::Request,
    model::{ConvAwareness, MicSelection, Mode, PressAction, Profile, SpatialAudio},
};

mod completion;
mod debug;
mod fmt;
mod help;
mod install;
mod meter;
mod standalone;
mod tray_cli;

const PROG: &str = "podctl";

/// How to print an answer: for a machine or for a person, and in the
/// machine case on one line or indented.
#[derive(Clone, Copy)]
struct Output {
    json: bool,
    pretty: bool,
}

fn main() -> ExitCode {
    install_default_sigpipe();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = Output {
        json: args.iter().any(|a| a == "--json"),
        pretty: args.iter().any(|a| a == "--pretty"),
    };
    let code = run(args).unwrap_or_else(|e| {
        if out.json {
            // An error is an envelope too, so one reader handles both.
            match &e.hint {
                Some(hint) => podctl::json::error_with_hint(&e.msg, hint).print(out.pretty),
                None => podctl::json::error(&e.msg).print(out.pretty),
            }
        } else {
            eprintln!("{PROG}: {e}");
            if let Some(hint) = &e.hint {
                eprintln!("hint: {hint}");
            }
        }
        e.exit_code()
    });
    ExitCode::from(code as u8)
}

#[derive(Debug)]
struct CliError {
    msg: String,
    /// Printed as `hint: …` on the next line. One form for all of them;
    /// there used to be three.
    hint: Option<String>,
    code: i32,
}

impl CliError {
    fn usage(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            hint: None,
            code: exitcode::USAGE,
        }
    }

    fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    fn unavailable(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            hint: None,
            code: exitcode::UNAVAILABLE,
        }
    }
    fn oserr(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            hint: None,
            code: exitcode::OSERR,
        }
    }
    fn dataerr(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            hint: None,
            code: exitcode::DATAERR,
        }
    }
    fn exit_code(&self) -> i32 {
        self.code
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for CliError {}

fn run(mut args: Vec<String>) -> Result<i32, CliError> {
    let out = Output {
        json: pop_flag(&mut args, "--json"),
        pretty: pop_flag(&mut args, "--pretty"),
    };

    // Asked for before anything is dispatched: `debug --help` used to
    // print a whole diagnostic report, `meter --help` went looking for
    // a sink, and `completion --help` complained about an unknown shell.
    if let Some(verb) = args.first().map(String::as_str)
        && !matches!(verb, "help" | "-h" | "--help")
        && args[1..].iter().any(|a| a == "--help" || a == "-h")
    {
        return Ok(help::print_verb(verb));
    }

    let verb = match args.first().map(String::as_str) {
        None | Some("help") | Some("-h") | Some("--help") => {
            return Ok(help::print_top(args.get(1).map(String::as_str)));
        }
        Some("version") | Some("--version") | Some("-V") => {
            print_version(out);
            return Ok(exitcode::OK);
        }
        Some("completion") => {
            completion::emit(args.get(1).map(String::as_str))
                .map_err(|e| CliError::usage(e.to_string()))?;
            return Ok(exitcode::OK);
        }
        Some("debug") => {
            if args.get(1).map(String::as_str) == Some("emit-case-lid") {
                let open = match args.get(2).map(String::as_str) {
                    Some("open") => true,
                    Some("close") => false,
                    _ => {
                        return Err(CliError::usage(
                            "debug emit-case-lid: expected `open` or `close`",
                        ));
                    }
                };
                let req = Request::DebugEmitCaseLid { open };
                let resp = try_daemon(&req)?.ok_or_else(|| {
                    CliError::unavailable(
                        "debug emit-case-lid needs the daemon (podctld not running)",
                    )
                })?;
                return handle_response("debug", resp, out);
            }
            let no_redact = args.iter().any(|a| a == "--no-redact");
            let report = debug::run(no_redact);
            if out.json {
                // The report is one text block by design — it is meant
                // to be pasted into an issue. `--json` wraps it so a
                // script can collect it alongside everything else.
                podctl::json::Envelope::ok(
                    "debug",
                    serde_json::json!({ "report": report, "redacted": !no_redact }),
                )
                .print(out.pretty);
            } else {
                print!("{report}");
            }
            return Ok(exitcode::OK);
        }
        Some("meter") => {
            return Ok(meter::run(&args[1..], out.json, out.pretty));
        }
        Some("ble") => {
            return ble_switch(args.get(1).map(String::as_str), out);
        }
        Some("install") => {
            return Ok(install::install(&args[1..]));
        }
        Some("tray") => {
            return Ok(tray_cli::run(&args[1..]));
        }
        Some("popup") => {
            let resp = try_daemon(&Request::ShowPopup)?.ok_or_else(|| {
                CliError::unavailable("popup needs the daemon (podctld not running)")
            })?;
            return handle_response("popup", resp, out);
        }
        Some("reboot") => {
            return Ok(reboot());
        }
        Some("uninstall") => {
            return Ok(install::uninstall(&args[1..]));
        }
        Some(v) => v.to_string(),
    };

    if matches!(verb.as_str(), "watch" | "w") {
        return stream_events(out);
    }

    let request = parse_verb(&verb, &args[1..])?;
    // Twelve seconds of silence otherwise, with nothing to say whether
    // anything is happening.
    if let Request::Pair { address: None, .. } = &request {
        eprintln!("{PROG}: looking for AirPods in pairing mode (about 12s)…");
    }
    let response = match try_daemon(&request)? {
        Some(resp) => resp,
        None => standalone::dispatch(&request),
    };
    handle_response(&verb, response, out)
}

/// `podctl ble [on|off]` — the daemon reads the file itself, so this
/// works without it running.
fn ble_switch(arg: Option<&str>, out: Output) -> Result<i32, CliError> {
    let on = match arg {
        None => {
            print_ble(podctl::config::load().ble, out);
            return Ok(exitcode::OK);
        }
        Some(raw) => on_off("ble", raw)?,
    };
    podctl::config::set_ble(on)
        .map_err(|e| CliError::unavailable(format!("could not write the settings file: {e}")))?;
    print_ble(on, out);
    Ok(exitcode::OK)
}

fn print_ble(on: bool, out: Output) {
    if out.json {
        podctl::json::Envelope::ok("ble", serde_json::json!({ "enabled": on })).print(out.pretty);
    } else {
        println!("ble  {}", if on { "on" } else { "off" });
    }
}

fn reboot() -> i32 {
    use std::process::{Command, Stdio};

    let units = ["podctld", "podctl-tray", "podctl-popup"];
    let mut restarted = 0;
    let mut failed = false;
    for u in units {
        let unit = format!("{u}.service");
        let installed = Command::new("systemctl")
            .args(["--user", "cat", &unit])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !installed {
            continue;
        }
        match Command::new("systemctl")
            .args(["--user", "restart", &unit])
            .status()
        {
            Ok(s) if s.success() => {
                println!("{PROG}: restarted {u}");
                restarted += 1;
            }
            _ => {
                eprintln!("{PROG}: reboot: failed to restart {u}");
                failed = true;
            }
        }
    }

    if restarted == 0 {
        eprintln!("{PROG}: reboot: no podctl systemd user services found.");
        eprintln!(
            "hint: install them with 'podctl install' (optionally --with-tray --with-popup)."
        );
        return exitcode::UNAVAILABLE;
    }
    if failed {
        exitcode::SOFTWARE
    } else {
        exitcode::OK
    }
}

fn pop_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let mut found = false;
    args.retain(|a| {
        if a == flag {
            found = true;
            false
        } else {
            true
        }
    });
    found
}

/// Catch a typo, too many arguments and an option the verb does not
/// take before the request goes anywhere. Every one of these used to
/// pass silently: `podctl rename Test AirPods` renamed to "Test".
fn check_args(verb: &str, rest: &[String]) -> Result<(), CliError> {
    let Some(cmd) = commands::find(verb) else {
        let mut err = CliError::usage(format!("unknown command '{verb}'"));
        err = match commands::suggest(verb) {
            Some(near) => err.with_hint(format!("did you mean '{near}'? see 'podctl help'")),
            None => err.with_hint("'podctl help' lists every command"),
        };
        return Err(err);
    };
    for arg in rest.iter().filter(|a| a.starts_with('-')) {
        let known = cmd.flags.contains(&arg.as_str())
            || commands::GLOBAL_FLAGS.contains(&arg.as_str())
            // A negative latency offset is a value, not an option.
            || arg[1..].chars().all(|c| c.is_ascii_digit());
        if !known {
            return Err(CliError::usage(format!("{verb}: unknown option '{arg}'"))
                .with_hint(format!("see 'podctl help {}'", cmd.name)));
        }
    }
    let positional = rest.iter().filter(|a| !a.starts_with('-')).count();
    if positional > cmd.max_args() {
        let what = match cmd.max_args() {
            0 => format!("{verb} takes no arguments, got {positional}"),
            1 => format!("{verb} takes one argument, got {positional}"),
            n => format!("{verb} takes {n} arguments, got {positional}"),
        };
        let mut err = CliError::usage(what);
        if cmd.name == "rename" {
            err = err.with_hint("quote names with spaces: podctl rename \"Test AirPods\"");
        }
        return Err(err);
    }
    Ok(())
}

fn parse_verb(verb: &str, rest: &[String]) -> Result<Request, CliError> {
    check_args(verb, rest)?;
    Ok(match verb {
        "status" | "s" => Request::Status,
        "battery" | "bat" | "b" => Request::Battery,
        "ping" => Request::Ping,

        "mode" | "m" => Request::SetMode {
            mode: parse_arg::<Mode>(rest, verb)?,
        },
        "conv" | "c" => Request::SetConv {
            conv: parse_arg::<ConvAwareness>(rest, verb)?,
        },
        "spatial" => Request::SetSpatial {
            mode: parse_arg::<SpatialAudio>(rest, verb)?,
        },

        "ear" | "ear-detection" => Request::SetEarDetection {
            on: parse_on_off(rest, verb)?,
        },
        "mic" => Request::SetMic {
            mic: parse_arg::<MicSelection>(rest, verb)?,
        },
        "loud-reduction" | "loud" => Request::SetLoudReduction {
            on: parse_on_off(rest, verb)?,
        },
        "press" => {
            let side = parse_press_side(rest.first().map(String::as_str))?;
            let action = parse_arg_at::<PressAction>(rest, 1, verb)?;
            Request::SetPressAction { side, action }
        }
        "tone-on-press" | "tone" => Request::SetToneOnPress {
            on: parse_on_off(rest, verb)?,
        },
        "rename" => Request::Rename {
            name: require_arg(rest, verb)?.to_string(),
        },

        "connect" => Request::Connect,
        "disconnect" | "dc" => Request::Disconnect,
        "pair" => Request::Pair {
            address: rest
                .iter()
                .find(|a| !a.starts_with('-'))
                .map(|a| a.to_uppercase()),
            yes: rest.iter().any(|a| a == "--yes" || a == "-y"),
        },
        "unpair" | "forget" => Request::Unpair,
        "list" | "ls" => Request::List,
        "auto-connect" | "auto" => Request::SetAutoConnect {
            on: parse_on_off(rest, verb)?,
        },

        "volume" | "vol" | "v" => Request::SetVolume {
            percent: parse_percent(rest, verb)?,
        },
        "mute" => Request::SetMuted {
            muted: parse_on_off(rest, verb)?,
        },
        "profile" | "p" => Request::SetProfile {
            profile: parse_arg::<Profile>(rest, verb)?,
        },
        "codec" => Request::SetCodec {
            codec: require_arg(rest, verb)?.to_string(),
        },
        "default" | "default-sink" => Request::MakeDefaultSink,
        "latency" => {
            let arg = require_arg(rest, verb)?;
            let ms: i32 = arg.parse().map_err(|_| bad_value(verb, 0, arg))?;
            Request::SetLatencyOffset { ms }
        }

        "one-bud-anc" | "obanc" => Request::SetOneBudAnc {
            on: parse_on_off(rest, verb)?,
        },
        "chime" | "chime-volume" => Request::SetChimeVolume {
            level: parse_percent(rest, verb)?,
        },
        "auto-anc" | "anc-strength" => Request::SetAutoAncLevel {
            level: parse_percent(rest, verb)?,
        },

        "watch" | "w" => Request::Watch {
            role: WatchRole::Observer,
        },

        // check_args has already rejected anything not in the table.
        other => return Err(CliError::usage(format!("unknown command '{other}'"))),
    })
}

/// One shape for every value error: `podctl: <verb>: <problem>`, the
/// accepted values named, and the help page as the hint. There used to
/// be three hint styles and two quoting styles.
fn bad_value(verb: &str, index: usize, raw: &str) -> CliError {
    let cmd = commands::find(verb);
    let name = cmd.map(|c| c.name).unwrap_or(verb);
    let expected = expected_label(verb, index)
        .map(|e| format!(" (expected {e})"))
        .unwrap_or_default();
    CliError::usage(format!("{name}: unknown value '{raw}'{expected}"))
        .with_hint(format!("see 'podctl help {name}'"))
}

fn missing_value(verb: &str, index: usize) -> CliError {
    let cmd = commands::find(verb);
    let name = cmd.map(|c| c.name).unwrap_or(verb);
    let what = expected_label(verb, index).unwrap_or_else(|| "a value".into());
    CliError::usage(format!("{name}: needs an argument ({what})"))
        .with_hint(format!("see 'podctl help {name}'"))
}

/// The accepted values of an argument, or its label when it is free
/// text (`0..100`, `ms`).
fn expected_label(verb: &str, index: usize) -> Option<String> {
    let cmd = commands::find(verb)?;
    let values = cmd.values_at(index);
    if !values.is_empty() {
        return Some(values.join(", "));
    }
    cmd.args.get(index).map(|a| a.label.to_string())
}

fn parse_arg<T: FromStr<Err = String>>(rest: &[String], verb: &str) -> Result<T, CliError> {
    parse_arg_at(rest, 0, verb)
}

fn parse_arg_at<T: FromStr<Err = String>>(
    rest: &[String],
    idx: usize,
    verb: &str,
) -> Result<T, CliError> {
    let raw = rest
        .get(idx)
        .map(String::as_str)
        .ok_or_else(|| missing_value(verb, idx))?;
    T::from_str(raw).map_err(|_| bad_value(verb, idx, raw))
}

fn parse_on_off(rest: &[String], verb: &str) -> Result<bool, CliError> {
    let raw = require_arg(rest, verb)?;
    on_off(verb, raw)
}

fn on_off(verb: &str, raw: &str) -> Result<bool, CliError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "on" | "1" | "true" | "yes" => Ok(true),
        "off" | "0" | "false" | "no" => Ok(false),
        _ => Err(bad_value(verb, 0, raw)),
    }
}

fn parse_press_side(raw: Option<&str>) -> Result<PressSide, CliError> {
    let Some(raw) = raw else {
        return Err(missing_value("press", 0));
    };
    match raw.to_ascii_lowercase().as_str() {
        "left" | "l" => Ok(PressSide::Left),
        "right" | "r" => Ok(PressSide::Right),
        other => Err(bad_value("press", 0, other)),
    }
}

/// A number in `0..=100`, the only numeric range the CLI takes.
fn parse_percent(rest: &[String], verb: &str) -> Result<u8, CliError> {
    let raw = require_arg(rest, verb)?;
    match raw.parse::<u8>() {
        Ok(v) if v <= 100 => Ok(v),
        _ => Err(bad_value(verb, 0, raw)),
    }
}

fn require_arg<'a>(rest: &'a [String], verb: &str) -> Result<&'a str, CliError> {
    rest.first()
        .map(String::as_str)
        .ok_or_else(|| missing_value(verb, 0))
}

fn try_daemon(req: &Request) -> Result<Option<Response>, CliError> {
    let Some(mut client) = client::Client::connect() else {
        return Ok(None);
    };
    match client.request(req, client::timeout_for(req)) {
        Ok(Response::Err(message)) if message.starts_with("malformed request") => {
            // The daemon could not read what we sent: almost always a
            // daemon of another version, not a bug in the request.
            let mut err = CliError::unavailable(message);
            if let Some(note) = client::version_mismatch() {
                err = err.with_hint(note);
            }
            Err(err)
        }
        Ok(resp) => Ok(Some(resp)),
        Err(e @ client::Error::Timeout(_)) => Err(CliError::unavailable(e.to_string())),
        Err(e @ client::Error::Closed) => Err(CliError::unavailable(e.to_string())),
        Err(client::Error::Malformed(detail)) => {
            let mut err = CliError::dataerr(format!("malformed answer: {detail}"));
            if let Some(note) = client::version_mismatch() {
                err = err.with_hint(note);
            }
            Err(err)
        }
        Err(client::Error::Io(e)) => Err(CliError::oserr(format!("talking to the daemon: {e}"))),
    }
}

fn handle_response(verb: &str, resp: Response, out: Output) -> Result<i32, CliError> {
    if out.json {
        print_json(verb, &resp, out.pretty);
        return Ok(if resp.is_err() {
            exitcode::UNAVAILABLE
        } else {
            exitcode::OK
        });
    }
    match resp {
        Response::State(s) => {
            fmt::print_state(verb, &s);
            Ok(exitcode::OK)
        }
        Response::List(items) => {
            fmt::print_list(&items);
            Ok(exitcode::OK)
        }
        Response::Event(_) => {
            eprintln!("{PROG}: unexpected event payload on single-shot call");
            Ok(exitcode::SOFTWARE)
        }
        Response::Pong => {
            println!("pong");
            Ok(exitcode::OK)
        }
        Response::Hello(hello) => {
            println!("podctld {} (protocol {})", hello.version, hello.proto);
            Ok(exitcode::OK)
        }
        Response::Done => {
            println!("ok");
            Ok(exitcode::OK)
        }
        Response::Pending(reason) => {
            // Not "ok": the thing has not happened yet.
            println!("pending: {reason}");
            Ok(exitcode::OK)
        }
        Response::Err(error) => {
            eprintln!("{PROG}: {error}");
            Ok(exitcode::UNAVAILABLE)
        }
    }
}

/// `--json` for one answer: the same envelope for every command, on one
/// line unless `--pretty` is asked for. There used to be three shapes
/// and a mix of pretty and compact.
fn print_json(verb: &str, resp: &Response, pretty: bool) {
    use podctl::json::{Envelope, error};
    match resp {
        Response::State(state) => {
            if commands::find(verb).map(|c| c.name) == Some("battery") {
                Envelope::ok("battery", &state.battery).print(pretty);
            } else {
                Envelope::ok("state", state).print(pretty);
            }
        }
        Response::List(items) => Envelope::ok("list", items).print(pretty),
        Response::Event(e) => Envelope::ok("event", e).print(pretty),
        Response::Pong => Envelope::ok("pong", serde_json::json!({})).print(pretty),
        Response::Hello(hello) => Envelope::ok("hello", hello).print(pretty),
        Response::Done => Envelope::ok("done", serde_json::json!({})).print(pretty),
        Response::Pending(reason) => {
            Envelope::ok("pending", serde_json::json!({ "reason": reason })).print(pretty)
        }
        Response::Err(message) => error(message).print(pretty),
    }
}

fn print_version(out: Output) {
    // The daemon's own version, not ours: running a new CLI against an
    // old daemon is the first thing to check when a setter fails.
    let daemon = client::hello();
    let (state, version) = match &daemon {
        None => ("not-running", None),
        Some(None) => ("running", None),
        Some(Some(h)) => ("running", Some(h)),
    };
    if out.json {
        podctl::json::Envelope::ok(
            "version",
            serde_json::json!({
                "name":           env!("CARGO_PKG_NAME"),
                "version":        env!("CARGO_PKG_VERSION"),
                "license":        "MIT OR Apache-2.0",
                "daemon":         state,
                "daemon_version": version.map(|h| h.version.clone()),
                "daemon_proto":   version.map(|h| h.proto),
                "proto":          podctl::ipc::PROTO,
            }),
        )
        .print(out.pretty);
        return;
    }
    println!(
        "podctl {} (protocol {})",
        env!("CARGO_PKG_VERSION"),
        podctl::ipc::PROTO
    );
    println!("  license  MIT OR Apache-2.0");
    match version {
        Some(h) => println!("  daemon   {} (protocol {})", h.version, h.proto),
        None if daemon.is_some() => {
            println!("  daemon   running, version unknown (older than the handshake)")
        }
        None => println!("  daemon   not running (standalone mode)"),
    }
    if let Some(note) = client::version_mismatch() {
        eprintln!("{PROG}: {note}");
    }
}

fn stream_events(out: Output) -> Result<i32, CliError> {
    let path = podctl::socket_path();
    let mut client = client::Client::connect().ok_or_else(|| {
        CliError::unavailable(format!(
            "watch needs the daemon (no socket at {}). install it with 'podctl install'.",
            path.display()
        ))
    })?;
    // The subscription is a normal request; everything after it is the
    // stream, which this reads line by line without a deadline.
    let req = Request::Watch {
        role: WatchRole::Observer,
    };
    match client.request(&req, client::timeout_for(&req)) {
        Ok(Response::Err(message)) => return Err(CliError::unavailable(message)),
        Ok(_) => {}
        Err(e) => return Err(CliError::unavailable(e.to_string())),
    }
    if !out.json {
        eprintln!("{PROG}: watching for events (Ctrl-C to stop)…");
    }
    loop {
        // No deadline: a stream may be quiet for hours.
        let resp = match client.next_line(None) {
            Ok(resp) => resp,
            // The daemon went away, which ends the stream rather than
            // being an error of its own.
            Err(client::Error::Closed) => return Ok(exitcode::OK),
            Err(client::Error::Malformed(detail)) => {
                eprintln!("{PROG}: malformed event: {detail}");
                continue;
            }
            Err(e) => return Err(CliError::oserr(e.to_string())),
        };
        if out.json {
            print_json("watch", &resp, out.pretty);
            continue;
        }
        match resp {
            Response::Event(e) => fmt::print_event(&e),
            Response::Err(error) => {
                eprintln!("{PROG}: {error}");
                return Ok(exitcode::UNAVAILABLE);
            }
            _ => {}
        }
    }
}

// Restore default SIGPIPE so `podctl watch | head` exits cleanly.
fn install_default_sigpipe() {
    unsafe {
        let _ = libc_signal(SIGPIPE, SIG_DFL);
    }
}

const SIGPIPE: i32 = 13;
const SIG_DFL: usize = 0;
unsafe extern "C" {
    #[link_name = "signal"]
    fn libc_signal(sig: i32, handler: usize) -> usize;
}
