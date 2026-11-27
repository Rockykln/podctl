//! `podctl bar` — one line per state change, for status bars.

use std::io::Write;
use std::time::Duration;

use podctl::model::{Battery, DeviceState, Event, Mode};
use podctl::{Request, Response, WatchRole, client, exitcode};

/// Retry delays while the daemon is away. A bar is usually started by
/// the session, so it outlives every podctld restart.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Waybar,
    I3blocks,
    Polybar,
    Plain,
}

impl Format {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "waybar" | "json" => Some(Format::Waybar),
            "i3blocks" | "i3" => Some(Format::I3blocks),
            "polybar" => Some(Format::Polybar),
            "plain" | "text" => Some(Format::Plain),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct Opts {
    format: Format,
    once: bool,
    /// Percent below which the line carries the `low` class.
    low: u8,
    /// Shown while the buds are away; empty hides the module in waybar
    /// and prints an empty line elsewhere.
    away: String,
    prefix: String,
    /// Colour for a low level, where the format carries one itself
    /// (i3blocks `color`, polybar `%{F…}`). waybar styles by class.
    low_color: Option<String>,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            format: Format::Waybar,
            once: false,
            low: 20,
            away: String::new(),
            prefix: String::new(),
            low_color: None,
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    let opts = match parse(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("podctl: bar: {e}");
            return exitcode::USAGE;
        }
    };
    if opts.once {
        let state = pull_status();
        let _ = emit(&render(&view(state.as_ref(), &opts), &opts));
        return exitcode::OK;
    }
    serve(&opts)
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--format" | "-f" => {
                let v = it.next().ok_or("--format needs a value")?;
                o.format = Format::parse(v).ok_or_else(|| {
                    format!("unknown format '{v}' (waybar|i3blocks|polybar|plain)")
                })?;
            }
            "--once" => o.once = true,
            "--low" => {
                let v = it.next().ok_or("--low needs a percentage")?;
                o.low = v
                    .parse::<u8>()
                    .map_err(|_| format!("--low wants 0…100, not '{v}'"))?
                    .min(100);
            }
            "--away" => o.away = it.next().cloned().unwrap_or_default(),
            "--low-color" => {
                let v = it.next().ok_or("--low-color needs a colour like #f38ba8")?;
                o.low_color = Some(v.clone());
            }
            "--prefix" => o.prefix = it.next().cloned().unwrap_or_default(),
            other => return Err(format!("unknown option '{other}'")),
        }
    }
    Ok(o)
}

/// Never blocks for long: a bar that hangs leaves a stale line on screen.
fn pull_status() -> Option<DeviceState> {
    let mut c = client::Client::connect()?;
    let req = Request::Status;
    match c.request(&req, client::timeout_for(&req)) {
        Ok(Response::State(s)) => Some(s),
        _ => None,
    }
}

fn serve(opts: &Opts) -> i32 {
    let mut last = String::new();
    let mut delay = RETRY_FIRST;
    loop {
        match session(opts, &mut last) {
            // A stream that stood and then ended is a daemon restart,
            // not a daemon that is gone: start the waiting over.
            Ok(Attached::Yes) => delay = RETRY_FIRST,
            Ok(Attached::No) => {}
            // Nobody is reading our lines any more.
            Err(Gone::Pipe) => return exitcode::OK,
        }
        if show(&render(&view(None, opts), opts), &mut last).is_err() {
            return exitcode::OK;
        }
        std::thread::sleep(delay);
        delay = (delay * 2).min(RETRY_MAX);
    }
}

/// Whether a session got as far as a running event stream.
enum Attached {
    Yes,
    No,
}

/// The bar stopped reading.
enum Gone {
    Pipe,
}

/// One connected run: full state first, then a line per event. Returns
/// when the stream ends.
fn session(opts: &Opts, last: &mut String) -> Result<Attached, Gone> {
    let Some(mut state) = pull_status() else {
        return Ok(Attached::No);
    };
    show(&render(&view(Some(&state), opts), opts), last)?;

    let Some(mut stream) = client::Client::connect() else {
        return Ok(Attached::No);
    };
    let req = Request::Watch {
        role: WatchRole::Observer,
    };
    match stream.request(&req, client::timeout_for(&req)) {
        Ok(Response::Err(_)) | Err(_) => return Ok(Attached::No),
        Ok(_) => {}
    }
    loop {
        // No deadline: an idle pair of AirPods sends nothing for hours.
        let resp = match stream.next_line(None) {
            Ok(r) => r,
            Err(client::Error::Malformed(_)) => continue,
            Err(_) => return Ok(Attached::Yes),
        };
        match resp {
            // The daemon sends the whole state when the stream opens,
            // when it had to drop events, and whenever something turned
            // up that no event covers. Taking it is the only way a bar
            // recovers from a missed event.
            Response::State(s) => state = s,
            Response::Event(ev) => {
                // Connect and a settings change carry no capabilities,
                // so the model and the mode come from a fresh snapshot.
                let refresh = matches!(ev, Event::Connected { .. } | Event::SettingsChanged);
                apply(&mut state, ev);
                if refresh && let Some(s) = pull_status() {
                    state = s;
                }
            }
            _ => continue,
        }
        show(&render(&view(Some(&state), opts), opts), last)?;
    }
}

fn apply(state: &mut DeviceState, ev: Event) {
    match ev {
        Event::Connected { name, address } => {
            state.connected = true;
            state.name = Some(name);
            state.address = Some(address);
        }
        Event::Disconnected => {
            state.connected = false;
            state.battery = Battery::default();
            state.settings.mode = None;
        }
        Event::Battery(b) => state.battery = b,
        Event::Mode { mode } => state.settings.mode = Some(mode),
        Event::ConvAwareness { conv } => state.settings.conv_awareness = Some(conv),
        Event::InEar(_)
        | Event::CaseLid { .. }
        | Event::Press { .. }
        | Event::SettingsChanged
        | Event::ShowPopup => {}
    }
}

/// Only print what changed: bars redraw on every line they read.
fn show(line: &str, last: &mut String) -> Result<(), Gone> {
    if line == last {
        return Ok(());
    }
    emit(line)?;
    last.clear();
    last.push_str(line);
    Ok(())
}

/// Rust ignores SIGPIPE, so a bar that closed its end shows up here as
/// a write error. Without this, `podctl bar` would outlive every bar it
/// ever fed and hold a daemon connection open for nothing.
fn emit(line: &str) -> Result<(), Gone> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{line}").map_err(|_| Gone::Pipe)?;
    out.flush().map_err(|_| Gone::Pipe)
}

/// What a bar gets to show, independent of its syntax.
#[derive(Debug, PartialEq, Eq)]
struct View {
    text: String,
    /// `connected`, `disconnected`, `unpaired` or `offline`, for bars
    /// that pick an icon by state.
    alt: &'static str,
    classes: Vec<&'static str>,
    percentage: Option<u8>,
    /// One entry per line. waybar wants them joined by `\r`, the other
    /// formats have nowhere to put them.
    tooltip: Vec<String>,
    low: bool,
}

fn view(state: Option<&DeviceState>, opts: &Opts) -> View {
    let Some(s) = state else {
        return View {
            text: opts.away.clone(),
            alt: "offline",
            classes: vec!["offline"],
            percentage: None,
            tooltip: vec!["podctld is not running".into()],
            low: false,
        };
    };
    let name = s
        .name
        .clone()
        .unwrap_or_else(|| s.capabilities.model.label().to_string());
    if s.address.is_none() {
        return View {
            text: opts.away.clone(),
            alt: "unpaired",
            classes: vec!["unpaired"],
            percentage: None,
            tooltip: vec!["no AirPods paired".into()],
            low: false,
        };
    }
    if !s.connected {
        // Levels without a link come from the case announcing itself
        // over LE: the buds are lying in it, not gone.
        let tooltip = match levels(&s.battery) {
            Some(l) => vec![format!("{name} — in case"), l],
            None => vec![format!("{name} — disconnected")],
        };
        return View {
            text: opts.away.clone(),
            alt: "disconnected",
            classes: vec!["disconnected"],
            percentage: None,
            tooltip,
            low: false,
        };
    }
    let lowest = [s.battery.left, s.battery.right]
        .into_iter()
        .flatten()
        .min();
    let charging = s.battery.left_charging || s.battery.right_charging;
    let low = lowest.is_some_and(|p| p < opts.low) && !charging;
    let mut classes = vec!["connected"];
    if charging {
        classes.push("charging");
    }
    if low {
        classes.push("low");
    }
    let text = match lowest {
        Some(p) => format!("{}{p}%", opts.prefix),
        // Connected, but the buds have not reported yet.
        None => format!("{}…", opts.prefix),
    };
    let mut tooltip = vec![match s.settings.mode {
        Some(m) => format!("{name} — {}", mode_label(m)),
        None => name,
    }];
    if let Some(l) = levels(&s.battery) {
        tooltip.push(l);
    }
    View {
        text,
        alt: "connected",
        classes,
        percentage: lowest,
        tooltip,
        low,
    }
}

fn levels(b: &Battery) -> Option<String> {
    let mut parts = Vec::with_capacity(3);
    for (label, level, charging) in [
        ("L", b.left, b.left_charging),
        ("R", b.right, b.right_charging),
        ("Case", b.case, b.case_charging),
    ] {
        if let Some(p) = level {
            parts.push(if charging {
                format!("{label} {p}% ⚡")
            } else {
                format!("{label} {p}%")
            });
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Off => "Off",
        Mode::NoiseCancellation => "Noise Cancellation",
        Mode::Transparency => "Transparency",
        Mode::Adaptive => "Adaptive",
    }
}

fn render(v: &View, opts: &Opts) -> String {
    match opts.format {
        Format::Waybar => waybar(v),
        Format::I3blocks => i3blocks(v, opts),
        Format::Polybar => polybar(v, opts),
        Format::Plain => v.text.clone(),
    }
}

/// waybar reads one JSON object per line and knows exactly five keys.
/// Tooltip lines are separated by `\r`, not `\n` (waybar-custom(5)).
fn waybar(v: &View) -> String {
    serde_json::json!({
        "text": v.text,
        "alt": v.alt,
        "class": v.classes,
        "tooltip": v.tooltip.join("\r"),
        "percentage": v.percentage,
    })
    .to_string()
}

/// i3blocks with `format=json` and `interval=persist`: one i3bar block
/// per line. The raw line protocol is ambiguous for persistent commands,
/// so this stays with JSON.
fn i3blocks(v: &View, opts: &Opts) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert("full_text".into(), v.text.clone().into());
    obj.insert("short_text".into(), v.text.clone().into());
    if v.low
        && let Some(c) = &opts.low_color
    {
        obj.insert("color".into(), c.clone().into());
    }
    serde_json::Value::Object(obj).to_string()
}

/// polybar with `tail = true` takes one line of text per update and
/// allows its own format tags.
fn polybar(v: &View, opts: &Opts) -> String {
    match (&opts.low_color, v.low) {
        (Some(c), true) => format!("%{{F{c}}}{}%{{F-}}", v.text),
        _ => v.text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use podctl::caps::Model;

    fn connected(left: Option<u8>, right: Option<u8>) -> DeviceState {
        let mut s = DeviceState {
            connected: true,
            address: Some("AA:BB:CC:DD:EE:01".into()),
            name: Some("Test AirPods".into()),
            ..DeviceState::default()
        };
        s.capabilities = Model::Pro2UsbC.capabilities();
        s.battery.left = left;
        s.battery.right = right;
        s.settings.mode = Some(Mode::Transparency);
        s
    }

    #[test]
    fn the_text_is_the_emptier_bud() {
        let v = view(Some(&connected(Some(84), Some(78))), &Opts::default());
        assert_eq!(v.text, "78%");
        assert_eq!(v.percentage, Some(78));
        assert_eq!(v.alt, "connected");
        assert_eq!(v.classes, vec!["connected"]);
        assert_eq!(v.tooltip[0], "Test AirPods — Transparency");
        assert_eq!(v.tooltip[1], "L 84% · R 78%");
    }

    #[test]
    fn a_low_bud_is_marked_unless_it_is_charging() {
        let v = view(Some(&connected(Some(12), Some(80))), &Opts::default());
        assert!(v.classes.contains(&"low"));
        let mut s = connected(Some(12), Some(80));
        s.battery.left_charging = true;
        let v = view(Some(&s), &Opts::default());
        assert!(v.classes.contains(&"charging"));
        assert!(!v.classes.contains(&"low"), "charging is not a warning");
    }

    #[test]
    fn a_connected_pair_without_levels_says_so() {
        let v = view(Some(&connected(None, None)), &Opts::default());
        assert_eq!(v.text, "…");
        assert_eq!(v.percentage, None);
    }

    #[test]
    fn levels_without_a_link_read_as_in_case() {
        let mut s = connected(Some(50), Some(50));
        s.connected = false;
        s.battery.case = Some(90);
        let v = view(Some(&s), &Opts::default());
        assert_eq!(v.alt, "disconnected");
        assert!(v.tooltip[0].contains("in case"));
        assert!(v.tooltip[1].contains("Case 90%"));
    }

    #[test]
    fn no_daemon_and_no_pair_are_told_apart() {
        let off = view(None, &Opts::default());
        assert_eq!(off.alt, "offline");
        assert_eq!(off.tooltip, vec!["podctld is not running"]);
        assert_eq!(off.classes, vec!["offline"]);
        let mut s = connected(Some(50), Some(50));
        s.address = None;
        assert_eq!(view(Some(&s), &Opts::default()).alt, "unpaired");
    }

    #[test]
    fn away_text_and_prefix_are_the_user_s() {
        let opts = Opts {
            away: "off".into(),
            prefix: " ".into(),
            ..Opts::default()
        };
        let mut s = connected(Some(70), Some(70));
        s.connected = false;
        assert_eq!(view(Some(&s), &opts).text, "off");
        assert_eq!(
            view(Some(&connected(Some(70), Some(70))), &opts).text,
            " 70%"
        );
    }

    #[test]
    fn waybar_lines_are_one_json_object() {
        let opts = Opts::default();
        let line = render(&view(Some(&connected(Some(84), Some(78))), &opts), &opts);
        assert!(!line.contains('\n'), "a bar reads one object per line");
        let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
        assert_eq!(parsed["text"], "78%");
        assert_eq!(parsed["percentage"], 78);
        assert_eq!(parsed["class"][0], "connected");
        // waybar splits tooltip lines on a carriage return.
        assert_eq!(
            parsed["tooltip"],
            "Test AirPods — Transparency\rL 84% · R 78%"
        );
    }

    #[test]
    fn i3blocks_lines_are_an_i3bar_block() {
        let opts = Opts {
            format: Format::I3blocks,
            low_color: Some("#f38ba8".into()),
            ..Opts::default()
        };
        let fine = render(&view(Some(&connected(Some(84), Some(78))), &opts), &opts);
        let parsed: serde_json::Value = serde_json::from_str(&fine).expect("valid JSON");
        assert_eq!(parsed["full_text"], "78%");
        assert_eq!(parsed["short_text"], "78%");
        assert!(parsed.get("color").is_none(), "no colour above the limit");
        let low = render(&view(Some(&connected(Some(7), Some(78))), &opts), &opts);
        let parsed: serde_json::Value = serde_json::from_str(&low).expect("valid JSON");
        assert_eq!(parsed["color"], "#f38ba8");
    }

    #[test]
    fn plain_is_the_text_alone_and_polybar_can_colour_it() {
        let plain = Opts {
            format: Format::Plain,
            ..Opts::default()
        };
        assert_eq!(
            render(&view(Some(&connected(Some(84), Some(78))), &plain), &plain),
            "78%"
        );
        let poly = Opts {
            format: Format::Polybar,
            low_color: Some("#f38ba8".into()),
            ..Opts::default()
        };
        assert_eq!(
            render(&view(Some(&connected(Some(84), Some(78))), &poly), &poly),
            "78%"
        );
        assert_eq!(
            render(&view(Some(&connected(Some(7), Some(78))), &poly), &poly),
            "%{F#f38ba8}7%%{F-}"
        );
    }

    #[test]
    fn unchanged_state_prints_nothing_twice() {
        let mut last = String::new();
        let opts = Opts::default();
        let v = view(Some(&connected(Some(84), Some(78))), &opts);
        let line = render(&v, &opts);
        assert!(show(&line, &mut last).is_ok());
        assert_eq!(last, line);
        // Same line again: `show` leaves `last` alone, which is what the
        // caller checks.
        assert!(show(&line, &mut last).is_ok());
        assert_eq!(last, line);
    }

    #[test]
    fn formats_and_options_are_parsed() {
        let o = parse(&[
            "--format".into(),
            "i3blocks".into(),
            "--low".into(),
            "5".into(),
        ])
        .unwrap();
        assert_eq!(o.format, Format::I3blocks);
        assert_eq!(o.low, 5);
        assert!(parse(&["--format".into(), "nope".into()]).is_err());
        assert!(parse(&["--nope".into()]).is_err());
        assert!(parse(&["--once".into()]).unwrap().once);
    }

    #[test]
    fn a_disconnect_clears_what_the_link_carried() {
        let mut s = connected(Some(84), Some(78));
        apply(&mut s, Event::Disconnected);
        assert!(!s.connected);
        assert!(!s.battery.any_known());
        assert_eq!(s.settings.mode, None);
    }
}
