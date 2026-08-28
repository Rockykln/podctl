use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Duration;

use podctl::{
    PressSide, Response, WatchRole, exitcode,
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
        }
        e.exit_code()
    });
    ExitCode::from(code as u8)
}

#[derive(Debug)]
struct CliError {
    msg: String,
    code: i32,
}

impl CliError {
    fn usage(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            code: exitcode::USAGE,
        }
    }
    fn unavailable(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            code: exitcode::UNAVAILABLE,
        }
    }
    fn oserr(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            code: exitcode::OSERR,
        }
    }
    fn dataerr(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
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

    let verb = match args.first().map(String::as_str) {
        None | Some("help") | Some("-h") | Some("--help") => {
            help::print_top(args.get(1).map(String::as_str));
            return Ok(exitcode::OK);
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
            return Ok(meter::run(&args[1..]));
        }
        Some("install") => {
            if args.iter().any(|a| a == "--help" || a == "-h") {
                help::print_verb("install");
                return Ok(exitcode::OK);
            }
            return Ok(install::install(&args[1..]));
        }
        Some("tray") => {
            return Ok(tray_cli::run(&args[1..]));
        }
        Some("popup") => {
            if args.iter().any(|a| a == "--help" || a == "-h") {
                help::print_verb("popup");
                return Ok(exitcode::OK);
            }
            let resp = try_daemon(&Request::ShowPopup)?.ok_or_else(|| {
                CliError::unavailable("popup needs the daemon (podctld not running)")
            })?;
            return handle_response("popup", resp, json);
        }
        Some("reboot") => {
            if args.iter().any(|a| a == "--help" || a == "-h") {
                help::print_verb("reboot");
                return Ok(exitcode::OK);
            }
            return Ok(reboot());
        }
        Some("uninstall") => {
            if args.iter().any(|a| a == "--help" || a == "-h") {
                help::print_verb("uninstall");
                return Ok(exitcode::OK);
            }
            return Ok(install::uninstall(&args[1..]));
        }
        Some(v) => v.to_string(),
    };

    if args.iter().any(|a| a == "--help" || a == "-h") {
        help::print_verb(&verb);
        return Ok(exitcode::OK);
    }

    if matches!(verb.as_str(), "watch" | "w") {
        return stream_events(json);
    }

    let request = parse_verb(&verb, &args[1..])?;
    let response = match try_daemon(&request)? {
        Some(resp) => resp,
        None => standalone::dispatch(&request),
    };
    handle_response(&verb, response, json)
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

fn parse_verb(verb: &str, rest: &[String]) -> Result<Request, CliError> {
    Ok(match verb {
        "status" | "s" => Request::Status,
        "battery" | "bat" | "b" => Request::Battery,
        "ping" => Request::Ping,

        "mode" | "m" => Request::SetMode {
            mode: parse_arg::<Mode>(rest, verb, "off|anc|transparency|adaptive")?,
        },
        "conv" | "c" => Request::SetConv {
            conv: parse_arg::<ConvAwareness>(rest, verb, "on|off")?,
        },
        "spatial" => Request::SetSpatial {
            mode: parse_arg::<SpatialAudio>(rest, verb, "off|fixed|head-tracked")?,
        },

        "ear" | "ear-detection" => Request::SetEarDetection {
            on: parse_on_off(rest, verb)?,
        },
        "mic" => Request::SetMic {
            mic: parse_arg::<MicSelection>(rest, verb, "auto|left|right")?,
        },
        "loud-reduction" | "loud" => Request::SetLoudReduction {
            on: parse_on_off(rest, verb)?,
        },
        "press" => {
            let side = parse_press_side(rest.first().map(String::as_str))?;
            let action = parse_arg_at::<PressAction>(rest, 1, verb, "mode-cycle|siri|none")?;
            Request::SetPressAction { side, action }
        }
        "tone-on-press" | "tone" => Request::SetToneOnPress {
            on: parse_on_off(rest, verb)?,
        },
        "rename" => Request::Rename {
            name: require_arg(rest, verb, "\"new name\"")?.to_string(),
        },

        "connect" => Request::Connect,
        "disconnect" | "dc" => Request::Disconnect,
        "pair" => Request::Pair,
        "unpair" | "forget" => Request::Unpair,
        "list" | "ls" => Request::List,
        "auto-connect" | "auto" => Request::SetAutoConnect {
            on: parse_on_off(rest, verb)?,
        },

        "volume" | "vol" | "v" => {
            let arg = require_arg(rest, verb, "0..100")?;
            let pct: u8 = arg
                .parse()
                .map_err(|_| CliError::usage(format!("'volume': expected 0..100, got '{arg}'")))?;
            if pct > 100 {
                return Err(CliError::usage(format!(
                    "'volume': expected 0..100, got {pct}"
                )));
            }
            Request::SetVolume { percent: pct }
        }
        "mute" => Request::SetMuted {
            muted: parse_on_off(rest, verb)?,
        },
        "profile" | "p" => Request::SetProfile {
            profile: parse_arg::<Profile>(rest, verb, "high|headset|off")?,
        },
        "codec" => Request::SetCodec {
            codec: require_arg(rest, verb, "sbc|aac|aptx|ldac|…")?.to_string(),
        },
        "default" | "default-sink" => Request::MakeDefaultSink,
        "latency" => {
            let arg = require_arg(rest, verb, "<ms> (negative ok)")?;
            let ms: i32 = arg.parse().map_err(|_| {
                CliError::usage(format!("'latency': expected integer ms, got '{arg}'"))
            })?;
            Request::SetLatencyOffset { ms }
        }

        "one-bud-anc" | "obanc" => Request::SetOneBudAnc {
            on: parse_on_off(rest, verb)?,
        },
        "chime" | "chime-volume" => {
            let arg = require_arg(rest, verb, "0..100")?;
            let level: u8 = arg
                .parse()
                .map_err(|_| CliError::usage(format!("'chime': expected 0..100, got '{arg}'")))?;
            if level > 100 {
                return Err(CliError::usage(format!(
                    "'chime': expected 0..100, got {level}"
                )));
            }
            Request::SetChimeVolume { level }
        }
        "auto-anc" | "anc-strength" => {
            let arg = require_arg(rest, verb, "0..100")?;
            let level: u8 = arg.parse().map_err(|_| {
                CliError::usage(format!("'auto-anc': expected 0..100, got '{arg}'"))
            })?;
            if level > 100 {
                return Err(CliError::usage(format!(
                    "'auto-anc': expected 0..100, got {level}"
                )));
            }
            Request::SetAutoAncLevel { level }
        }

        "watch" | "w" => Request::Watch {
            role: WatchRole::Observer,
        },

        other => {
            return Err(CliError::usage(format!(
                "unknown command '{other}'. run 'podctl help' for the full list."
            )));
        }
    })
}

fn parse_arg<T: FromStr<Err = String>>(
    rest: &[String],
    verb: &str,
    hint: &str,
) -> Result<T, CliError> {
    let raw = require_arg(rest, verb, hint)?;
    T::from_str(raw).map_err(|e| CliError::usage(format!("'{verb}': {e}")))
}

fn parse_arg_at<T: FromStr<Err = String>>(
    rest: &[String],
    idx: usize,
    verb: &str,
    hint: &str,
) -> Result<T, CliError> {
    let raw = rest
        .get(idx)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("'{verb}' needs an argument: {hint}")))?;
    T::from_str(raw).map_err(|e| CliError::usage(format!("'{verb}': {e}")))
}

fn parse_on_off(rest: &[String], verb: &str) -> Result<bool, CliError> {
    let raw = require_arg(rest, verb, "on|off")?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "on" | "1" | "true" | "yes" => Ok(true),
        "off" | "0" | "false" | "no" => Ok(false),
        _ => Err(CliError::usage(format!(
            "'{verb}': expected on|off, got '{raw}'"
        ))),
    }
}

fn parse_press_side(raw: Option<&str>) -> Result<PressSide, CliError> {
    match raw.map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("left") | Some("l") => Ok(PressSide::Left),
        Some("right") | Some("r") => Ok(PressSide::Right),
        Some(other) => Err(CliError::usage(format!(
            "'press': side must be 'left' or 'right', got '{other}'"
        ))),
        None => Err(CliError::usage(
            "'press' needs <side> <action>, e.g. 'podctl press left mode-cycle'",
        )),
    }
}

fn require_arg<'a>(rest: &'a [String], verb: &str, hint: &str) -> Result<&'a str, CliError> {
    rest.first()
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("'{verb}' needs an argument: {hint}")))
}

fn try_daemon(req: &Request) -> Result<Option<Response>, CliError> {
    let path = podctl::socket_path();
    let stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(_) => return Ok(None),
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
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
    reader
        .read_line(&mut line)
        .map_err(|e| CliError::oserr(format!("read from daemon: {e}")))?;
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
            "watch needs the daemon (no socket at {}). install with 'podctl setup'.",
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
            println!("{line}");
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
