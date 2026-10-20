use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Duration;

use podctl::{
    PressSide, Response, WatchRole, commands, exitcode,
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

fn main() -> ExitCode {
    install_default_sigpipe();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let code = run(args).unwrap_or_else(|e| {
        if json {
            let payload = serde_json::json!({
                "kind": "err",
                "data": e.msg.clone(),
            });
            println!("{payload}");
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
    let json = pop_flag(&mut args, "--json");

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
            print_version(json);
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
                return handle_response("debug", resp, json);
            }
            let no_redact = args.iter().any(|a| a == "--no-redact");
            print!("{}", debug::run(no_redact));
            return Ok(exitcode::OK);
        }
        Some("meter") => {
            return Ok(meter::run(&args[1..], json));
        }
        Some("ble") => {
            return ble_switch(args.get(1).map(String::as_str), json);
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
            return handle_response("popup", resp, json);
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
        return stream_events(json);
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
    handle_response(&verb, response, json)
}

/// `podctl ble [on|off]` — the daemon reads the file itself, so this
/// works without it running.
fn ble_switch(arg: Option<&str>, json: bool) -> Result<i32, CliError> {
    let on = match arg {
        None => {
            let cfg = podctl::config::load();
            if json {
                println!("{{\"ble\":{}}}", cfg.ble);
            } else {
                println!("ble  {}", if cfg.ble { "on" } else { "off" });
            }
            return Ok(exitcode::OK);
        }
        Some(raw) => on_off("ble", raw)?,
    };
    podctl::config::set_ble(on)
        .map_err(|e| CliError::unavailable(format!("could not write the settings file: {e}")))?;
    if json {
        println!("{{\"ble\":{on}}}");
    } else {
        println!("ble  {}", if on { "on" } else { "off" });
    }
    Ok(exitcode::OK)
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

/// How long to wait for the daemon's answer. Ten seconds is plenty for
/// everything that only reads cached state, but `pair` scans for twelve
/// of them before it even answers, and BlueZ takes its time over a
/// connect to buds that are still waking up.
fn answer_timeout(req: &Request) -> Duration {
    let secs = match req {
        Request::Pair { .. } => 45,
        Request::Connect | Request::Disconnect | Request::Unpair => 30,
        _ => 10,
    };
    Duration::from_secs(secs)
}

fn try_daemon(req: &Request) -> Result<Option<Response>, CliError> {
    let path = podctl::socket_path();
    let stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(_) => return Ok(None),
    };
    let wait = answer_timeout(req);
    stream
        .set_read_timeout(Some(wait))
        .map_err(|e| CliError::oserr(format!("socket read timeout: {e}")))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| CliError::oserr(format!("socket write timeout: {e}")))?;

    let mut writer = &stream;
    writeln!(
        writer,
        "{}",
        serde_json::to_string(req).expect("request serialises")
    )
    .map_err(|e| CliError::oserr(format!("write to daemon: {e}")))?;
    writer
        .flush()
        .map_err(|e| CliError::oserr(format!("flush: {e}")))?;

    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    // A timeout on this socket surfaces as EAGAIN, which printed as
    // "Resource temporarily unavailable (os error 11)".
    let read = reader.read_line(&mut line).map_err(|e| match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
            CliError::unavailable(format!(
                "no answer from the daemon within {}s — it may still be working on it.",
                wait.as_secs()
            ))
        }
        _ => CliError::oserr(format!("read from daemon: {e}")),
    })?;
    if read == 0 {
        return Err(CliError::unavailable(
            "the daemon closed the connection without answering.",
        ));
    }
    let resp: Response = serde_json::from_str(line.trim())
        .map_err(|e| CliError::dataerr(format!("malformed response: {e}")))?;
    Ok(Some(resp))
}

fn handle_response(verb: &str, resp: Response, json: bool) -> Result<i32, CliError> {
    if json {
        let v = json_view(verb, &resp);
        println!("{v}");
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
        Response::Done => {
            println!("ok");
            Ok(exitcode::OK)
        }
        Response::Pending(reason) => {
            println!("ok (pending: {reason})");
            Ok(exitcode::OK)
        }
        Response::Err(error) => {
            eprintln!("{PROG}: {error}");
            Ok(exitcode::UNAVAILABLE)
        }
    }
}

fn json_view(verb: &str, resp: &Response) -> String {
    if let Response::State(s) = resp {
        let projected = match verb {
            "battery" | "bat" | "b" => Some(serde_json::json!({
                "kind": "battery",
                "data": &s.battery,
            })),
            _ => None,
        };
        if let Some(v) = projected {
            return serde_json::to_string_pretty(&v).unwrap_or_else(|_| "null".to_string());
        }
    }
    serde_json::to_string_pretty(resp).unwrap_or_else(|_| "null".to_string())
}

fn print_version(json: bool) {
    let mut running = false;
    if let Ok(stream) = UnixStream::connect(podctl::socket_path()) {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let mut w = &stream;
        if writeln!(
            w,
            "{}",
            serde_json::to_string(&Request::Ping).unwrap_or_default()
        )
        .is_ok()
        {
            let mut line = String::new();
            let mut r = BufReader::new(&stream);
            if r.read_line(&mut line).is_ok() {
                running = matches!(
                    serde_json::from_str::<Response>(line.trim()),
                    Ok(Response::Pong)
                );
            }
        }
    }
    if json {
        let v = serde_json::json!({
            "name":    env!("CARGO_PKG_NAME"),
            "version": env!("CARGO_PKG_VERSION"),
            "license": "MIT OR Apache-2.0",
            "daemon":  if running { "running" } else { "not_running" },
        });
        println!("{v}");
        return;
    }
    println!("podctl {}", env!("CARGO_PKG_VERSION"));
    println!("  license  MIT OR Apache-2.0");
    println!(
        "  daemon   {}",
        if running {
            "running"
        } else {
            "not running (standalone mode)"
        }
    );
}

fn stream_events(json: bool) -> Result<i32, CliError> {
    let path = podctl::socket_path();
    let stream = UnixStream::connect(&path).map_err(|_| {
        CliError::unavailable(format!(
            "watch needs the daemon (no socket at {}). install it with 'podctl install'.",
            path.display()
        ))
    })?;
    let mut writer = &stream;
    writeln!(
        writer,
        "{}",
        serde_json::to_string(&Request::Watch {
            role: WatchRole::Observer,
        })
        .expect("serialise")
    )
    .map_err(|e| CliError::oserr(format!("write: {e}")))?;
    writer
        .flush()
        .map_err(|e| CliError::oserr(format!("flush: {e}")))?;

    let reader = BufReader::new(&stream);
    for line in reader.lines() {
        let line = line.map_err(|e| CliError::oserr(format!("read: {e}")))?;
        if json {
            // The subscription ack is protocol, not an event.
            if !matches!(serde_json::from_str::<Response>(&line), Ok(Response::Done)) {
                println!("{line}");
            }
            continue;
        }
        match serde_json::from_str::<Response>(&line) {
            Ok(Response::Event(e)) => fmt::print_event(&e),
            Ok(Response::Done) => {
                eprintln!("{PROG}: watching for events (Ctrl-C to stop)…");
            }
            Ok(Response::Err(error)) => {
                eprintln!("{PROG}: {error}");
                return Ok(exitcode::UNAVAILABLE);
            }
            Ok(_) => {}
            Err(e) => eprintln!("{PROG}: malformed event: {e}"),
        }
    }
    Ok(exitcode::OK)
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

#[cfg(test)]
mod tests {
    use super::answer_timeout;
    use podctl::Request;

    /// `pair` scans for twelve seconds before it answers at all, so the
    /// ten-second wait every other verb gets ran out every single time.
    #[test]
    fn slow_verbs_wait_longer() {
        let pair = answer_timeout(&Request::Pair {
            address: None,
            yes: false,
        });
        assert!(pair.as_secs() > 12, "{pair:?}");
        assert!(answer_timeout(&Request::Connect) > answer_timeout(&Request::Status));
    }
}
