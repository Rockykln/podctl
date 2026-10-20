//! Output formatters for the CLI. Plain text — no ANSI colour by default
//! so output stays pipeable into grep/awk.

use podctl::model::{DeviceState, EarStatus, Event, InEar, PairedDevice, PressKind, Profile, Side};

pub fn print_state(verb: &str, s: &DeviceState) {
    let battery = podctl::commands::find(verb).map(|c| c.name) == Some("battery");
    if battery {
        print!("{}", render_battery(s, is_tty()));
    } else {
        print_full(s);
    }
}

/// Width of the label column, so every line starts its value in the
/// same place. The labels are the command names: what you read is what
/// you can type.
const LABEL: usize = 10;

/// Collects the lines, so the layout can be tested without a terminal.
#[derive(Default)]
struct Block {
    out: String,
    /// Bars are for a person looking at a terminal; a pipe gets plain
    /// columns.
    bars: bool,
}

impl Block {
    fn row(&mut self, label: &str, value: &str) {
        if value.is_empty() {
            return;
        }
        self.out.push_str(&format!("  {label:<LABEL$}{value}\n"));
    }

    fn line(&mut self, text: &str) {
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn blank(&mut self) {
        self.out.push('\n');
    }
}

fn print_full(s: &DeviceState) {
    print!("{}", render_full(s, is_tty()));
}

fn render_full(s: &DeviceState, bars: bool) -> String {
    let Some(address) = s.address.clone() else {
        return "no AirPods paired — put them in pairing mode and run 'podctl pair'\n".into();
    };
    let mut b = Block {
        bars,
        ..Default::default()
    };

    // Model first: it is the same every time, so the eye finds the name
    // and the state beside it.
    let mut head = s.capabilities.model.label().to_string();
    if let Some(name) = &s.name {
        head.push_str(&format!("  \"{name}\""));
    }
    let head = format!("{head}  {}", state_line(s));
    b.line(&head);

    b.blank();
    battery_rows(&mut b, s);
    if s.capabilities.has_in_ear_detection {
        let lid = match s.case_lid_open {
            Some(true) => "   case lid open",
            Some(false) => "   case lid closed",
            None => "",
        };
        let value = format!("{}{lid}", format_in_ear(&s.in_ear));
        b.row("in-ear", &value);
    } else if let Some(open) = s.case_lid_open {
        b.row("case lid", if open { "open" } else { "closed" });
    }

    b.blank();
    let listening = listening_line(s);
    b.row("listening", &listening);
    let settings = settings_line(s);
    b.row("settings", &settings);
    for (i, line) in audio_lines(s).iter().enumerate() {
        b.row(if i == 0 { "audio" } else { "" }, line);
    }
    let bt = bluetooth_line(s, &address);
    b.row("bluetooth", &bt);
    let fw = firmware_line(s);
    b.row("firmware", &fw);
    if let Some(color) = &s.info.color {
        b.row("color", color);
    }
    let presses = press_line(s);
    b.row("presses", &presses);
    b.out
}

/// `connected · AAP up · 3s ago`, or what is missing instead.
fn state_line(s: &DeviceState) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(
        if s.connected {
            "connected"
        } else {
            "disconnected"
        }
        .into(),
    );
    if s.connected {
        parts.push(if s.aap_linked {
            "AAP up".into()
        } else {
            "no AAP link".into()
        });
    }
    if s.updated_at > 0 {
        parts.push(format!(
            "{} ago",
            age(now_secs().saturating_sub(s.updated_at))
        ));
    }
    parts.join(" · ")
}

fn listening_line(s: &DeviceState) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(m) = s.settings.mode {
        parts.push(format!("mode {m}"));
    }
    if let Some(c) = s.settings.conv_awareness {
        parts.push(format!("conv {}", on_off(c == podctl::ConvAwareness::On)));
    }
    if let Some(sa) = s.settings.spatial_audio {
        parts.push(format!("spatial {sa}"));
    }
    parts.join(" · ")
}

fn settings_line(s: &DeviceState) -> String {
    let set = &s.settings;
    let mut parts: Vec<String> = Vec::new();
    if let Some(ed) = set.ear_detection {
        parts.push(format!("ear {}", on_off(ed)));
    }
    if let Some(mic) = set.mic_selection {
        parts.push(format!("mic {}", mic.as_str()));
    }
    if let Some(v) = set.one_bud_anc {
        parts.push(format!("one-bud-anc {}", on_off(v)));
    }
    if let Some(v) = set.auto_anc_level {
        parts.push(format!("auto-anc {v}"));
    }
    if let Some(v) = set.chime_volume {
        parts.push(format!("chime {v}"));
    }
    if let Some(lr) = set.loud_sound_reduction {
        parts.push(format!("loud-reduction {}", on_off(lr)));
    }
    if let Some(tone) = set.tone_on_press {
        parts.push(format!("tone-on-press {}", on_off(tone)));
    }
    if let Some(left) = set.press_action_left {
        let right = set.press_action_right.map(|a| a.as_str()).unwrap_or("?");
        parts.push(format!("press L {} / R {right}", left.as_str()));
    }
    parts.join(" · ")
}

fn audio_lines(s: &DeviceState) -> Vec<String> {
    let a = &s.audio;
    if a == &Default::default() {
        return Vec::new();
    }
    let mut first: Vec<String> = Vec::new();
    if let Some(v) = a.volume_percent {
        first.push(format!("{v}%{}", if a.muted { " muted" } else { "" }));
    }
    if let Some(p) = a.profile {
        first.push(
            match p {
                Profile::HighQuality => "high",
                Profile::Headset => "headset",
                Profile::Off => "off",
            }
            .to_string(),
        );
    }
    if let Some(c) = &a.codec {
        first.push(c.clone());
    }
    if a.is_default_sink {
        first.push("[default sink]".into());
    }
    if a.latency_offset_ms != 0 {
        first.push(format!("latency {} ms", a.latency_offset_ms));
    }
    let mut out = Vec::new();
    if !first.is_empty() {
        out.push(first.join("  "));
    }
    if !a.available_codecs.is_empty() {
        out.push(format!("codecs: {}", a.available_codecs.join(", ")));
    }
    out
}

fn bluetooth_line(s: &DeviceState, address: &str) -> String {
    let mut parts = vec![address.to_string()];
    if let Some(r) = s.bluetooth.rssi_dbm {
        parts.push(format!("rssi {r} dBm"));
    }
    if !s.bluetooth.paired {
        parts.push("not paired".into());
    }
    // `trusted` and `auto_connect` are the same BlueZ flag; printing
    // both read as two independent settings.
    parts.push(format!("auto-connect {}", on_off(s.bluetooth.auto_connect)));
    parts.join("  ")
}

fn firmware_line(s: &DeviceState) -> String {
    let info = &s.info;
    let mut parts: Vec<String> = Vec::new();
    if let Some(l) = &info.firmware_left {
        parts.push(format!("L {l}"));
    }
    if let Some(r) = &info.firmware_right {
        parts.push(format!("R {r}"));
    }
    if let Some(c) = &info.firmware_case {
        parts.push(format!("case {c}"));
    }
    parts.join(" · ")
}

fn press_line(s: &DeviceState) -> String {
    let pc = &s.press_counts;
    let total = [
        pc.left_single,
        pc.left_double,
        pc.left_triple,
        pc.left_long,
        pc.right_single,
        pc.right_double,
        pc.right_triple,
        pc.right_long,
    ]
    .iter()
    .sum::<u32>();
    if total == 0 {
        return String::new();
    }
    format!(
        "L 1×{} 2×{} 3×{} hold {}  ·  R 1×{} 2×{} 3×{} hold {}",
        pc.left_single,
        pc.left_double,
        pc.left_triple,
        pc.left_long,
        pc.right_single,
        pc.right_double,
        pc.right_triple,
        pc.right_long
    )
}

/// One line per bud, with a bar on a terminal. Disconnected, the levels
/// are whatever the case last announced over LE, with their age.
fn battery_rows(out: &mut Block, s: &DeviceState) {
    let b = &s.battery;
    if !b.any_known() {
        let why = if !s.connected {
            "no data (disconnected, and the case has not announced itself)"
        } else if s.aap_linked {
            "no data yet (AAP link up, the device has not reported a level)"
        } else {
            "no data (no AAP link — see 'podctl debug')"
        };
        out.row("battery", why);
        return;
    }
    let cells = [
        ("left", b.left, b.left_charging),
        ("right", b.right, b.right_charging),
        ("case", b.case, b.case_charging),
    ];
    for (i, (name, level, charging)) in cells.iter().enumerate() {
        let value = match level {
            Some(v) => {
                let mut cell = format!("{name:<7}{v:>3}%");
                if out.bars {
                    cell.push_str(&format!("  {}", bar(*v)));
                }
                if *charging {
                    cell.push_str("  charging");
                }
                cell
            }
            None => format!("{name:<7} --"),
        };
        out.row(if i == 0 { "battery" } else { "" }, &value);
    }
    if !s.connected {
        if let Some(at) = s.battery_at {
            let seen = format!(
                "over bluetooth LE, {} ago",
                age(now_secs().saturating_sub(at))
            );
            out.row("", &seen);
        }
        out.row("hint", "podctl connect");
    }
}

/// Ten cells, filled to the nearest tenth.
fn bar(pct: u8) -> String {
    let filled = ((u32::from(pct.min(100)) + 5) / 10) as usize;
    format!("[{}{}]", "#".repeat(filled), "-".repeat(10 - filled))
}

fn age(secs: i64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{} min", s / 60),
        s if s < 86_400 => format!("{} h", s / 3600),
        s => format!("{} d", s / 86_400),
    }
}

/// Bars and other decoration only where a person is looking; a pipe
/// gets plain columns.
fn is_tty() -> bool {
    unsafe { libc_isatty(1) != 0 }
}

unsafe extern "C" {
    #[link_name = "isatty"]
    fn libc_isatty(fd: i32) -> i32;
}

/// `podctl battery`: the same rows as `status`, without the rest.
fn render_battery(s: &DeviceState, bars: bool) -> String {
    let mut b = Block {
        bars,
        ..Default::default()
    };
    battery_rows(&mut b, s);
    b.out
}

pub fn print_list(items: &[PairedDevice]) {
    if items.is_empty() {
        println!("no paired AirPods on this adapter.");
        return;
    }
    const NAME: usize = 24;
    const MODEL: usize = 28;
    println!(
        "{:<18}  {:<NAME$}  {:<MODEL$}  {:<10}  auto-connect",
        "address", "name", "model", "state"
    );
    for d in items {
        let state = if d.connected { "connected" } else { "offline" };
        let model = d.model.as_deref().unwrap_or("?");
        println!(
            "{:<18}  {:<NAME$}  {:<MODEL$}  {:<10}  {}",
            d.address,
            clip(&d.name, NAME),
            clip(model, MODEL),
            state,
            on_off(d.trusted)
        );
    }
}

/// Cut to the column, counted in characters.
fn clip(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    text.chars()
        .take(width.saturating_sub(1))
        .collect::<String>()
        + "…"
}

pub fn print_event(e: &Event) {
    match e {
        Event::Connected { name, address } => println!("connected     {name}  ({address})"),
        Event::Disconnected => println!("disconnected"),
        Event::Battery(b) => {
            let f = |o: Option<u8>| o.map(|v| format!("{v}%")).unwrap_or_else(|| "--".into());
            println!(
                "battery       L {}  R {}  C {}",
                f(b.left),
                f(b.right),
                f(b.case)
            );
        }
        Event::InEar(in_ear) => {
            println!("in-ear        {}", format_in_ear(in_ear));
        }
        Event::CaseLid { open } => {
            println!("case lid      {}", if *open { "open" } else { "closed" });
        }
        Event::Mode { mode } => println!("mode          {mode}"),
        Event::ConvAwareness { conv } => {
            println!(
                "conv          {}",
                on_off(*conv == podctl::ConvAwareness::On)
            )
        }
        Event::Press { side, kind } => {
            let s = match side {
                Side::Left => "L",
                Side::Right => "R",
            };
            let k = match kind {
                PressKind::Single => "single",
                PressKind::Double => "double",
                PressKind::Triple => "triple",
                PressKind::Long => "long",
            };
            println!("press         {s} {k}");
        }
        Event::SettingsChanged => println!("settings      changed (re-run 'podctl status')"),
        Event::ShowPopup => {}
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

/// Render in-ear state. We don't know L/R from the wire (AAP gives
/// primary/secondary), but on a Pro 2 USB-C the left bud is always
/// primary — so we label accordingly. Set `PODCTL_NO_LR=1` to fall back
/// to primary/secondary if your unit behaves differently; `PODS_NO_LR`
/// is the old name and still works.
fn format_in_ear(in_ear: &InEar) -> String {
    use EarStatus::*;
    let no_lr =
        std::env::var_os("PODCTL_NO_LR").is_some() || std::env::var_os("PODS_NO_LR").is_some();
    let (a_label, b_label) = if no_lr {
        ("primary", "secondary")
    } else {
        ("L", "R")
    };
    match (in_ear.primary, in_ear.secondary) {
        (InEar, InEar) => "both in ear".into(),
        (OutOfEar, OutOfEar) => "both out of ear".into(),
        (InCase, InCase) => "both in case".into(),
        (Unknown, Unknown) => "--".into(),
        (a, b) => format!("{a_label} {a},  {b_label} {b}"),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{age, bar, render_full};
    use podctl::caps::Model;
    use podctl::model::{DeviceState, EarStatus, InEar, MicSelection, Mode, Profile};

    fn sample() -> DeviceState {
        let mut s = DeviceState {
            connected: true,
            address: Some("AA:BB:CC:DD:EE:01".into()),
            name: Some("My AirPods".into()),
            aap_linked: true,
            ..DeviceState::default()
        };
        s.capabilities = Model::Pro2UsbC.capabilities();
        s.info.firmware_left = Some("7A305".into());
        s.info.firmware_right = Some("7A305".into());
        s.info.firmware_case = Some("7A305".into());
        s.battery.left = Some(80);
        s.battery.left_charging = true;
        s.battery.right = Some(75);
        s.battery.case = Some(40);
        s.in_ear = InEar {
            primary: EarStatus::InEar,
            secondary: EarStatus::InEar,
        };
        s.case_lid_open = Some(false);
        s.settings.mode = Some(Mode::NoiseCancellation);
        s.settings.conv_awareness = Some(podctl::ConvAwareness::On);
        s.settings.ear_detection = Some(true);
        s.settings.mic_selection = Some(MicSelection::Auto);
        s.settings.one_bud_anc = Some(false);
        s.settings.auto_anc_level = Some(50);
        s.settings.chime_volume = Some(100);
        s.audio.volume_percent = Some(60);
        s.audio.profile = Some(Profile::HighQuality);
        s.audio.codec = Some("aac".into());
        s.audio.available_codecs = vec!["sbc".into(), "aac".into()];
        s.audio.is_default_sink = true;
        s.bluetooth.paired = true;
        s.bluetooth.trusted = true;
        s.bluetooth.auto_connect = true;
        s.bluetooth.rssi_dbm = Some(-52);
        s
    }

    /// The whole layout at once. Columns line up, the labels are the
    /// command names, and nothing has a trailing space.
    #[test]
    fn a_connected_device_reads_like_this() {
        let mut s = sample();
        s.updated_at = super::now_secs();
        let text = render_full(&s, false);
        let want = "\
AirPods Pro (2nd gen, USB-C)  \"My AirPods\"  connected · AAP up · 0s ago

  battery   left    80%  charging
            right   75%
            case    40%
  in-ear    both in ear   case lid closed

  listening mode anc · conv on
  settings  ear on · mic auto · one-bud-anc off · auto-anc 50 · chime 100
  audio     60%  high  aac  [default sink]
            codecs: sbc, aac
  bluetooth AA:BB:CC:DD:EE:01  rssi -52 dBm  auto-connect on
  firmware  L 7A305 · R 7A305 · case 7A305
";
        assert_eq!(text, want, "\n--- got ---\n{text}");
        for line in text.lines() {
            assert_eq!(line, line.trim_end(), "trailing space: {line:?}");
        }
        // One flag, one line: `trusted` is the same BlueZ bit.
        assert!(!text.contains("trusted"), "{text}");
    }

    /// Disconnected, the levels the case announced over LE are shown
    /// with their age.
    #[test]
    fn disconnected_still_shows_what_the_case_said() {
        let mut s = sample();
        s.connected = false;
        s.aap_linked = false;
        s.battery_at = Some(super::now_secs() - 150);
        let text = render_full(&s, false);
        assert!(text.contains("disconnected"), "{text}");
        assert!(text.contains("left    80%"), "{text}");
        assert!(text.contains("over bluetooth LE, 2 min ago"), "{text}");
        assert!(text.contains("hint      podctl connect"), "{text}");
    }

    #[test]
    fn nothing_paired_says_what_to_do() {
        let s = DeviceState::default();
        let text = render_full(&s, false);
        assert_eq!(
            text,
            "no AirPods paired — put them in pairing mode and run 'podctl pair'\n"
        );
    }

    /// A terminal gets bars, a pipe does not — and the bar is ten cells
    /// wide whatever the level.
    #[test]
    fn bars_are_for_terminals() {
        let s = sample();
        assert!(render_full(&s, true).contains("[########--]"));
        // `[default sink]` has brackets of its own; it is the bar that
        // must be absent.
        assert!(!render_full(&s, false).contains("[###"));
        assert_eq!(bar(0), "[----------]");
        assert_eq!(bar(5), "[#---------]");
        assert_eq!(bar(100), "[##########]");
        assert_eq!(bar(255), "[##########]");
    }

    #[test]
    fn the_list_keeps_its_columns() {
        use super::clip;
        assert_eq!(clip("AirPods", 10), "AirPods");
        assert_eq!(clip("0123456789", 10), "0123456789");
        assert_eq!(clip("01234567890", 10), "012345678…");
        // Counted in characters: a name of umlauts is not cut mid-byte.
        assert_eq!(clip("äöüäöüäöüäöü", 4), "äöü…");
    }

    #[test]
    fn ages_read_as_words() {
        assert_eq!(age(0), "0s");
        assert_eq!(age(59), "59s");
        assert_eq!(age(150), "2 min");
        assert_eq!(age(7200), "2 h");
        assert_eq!(age(200_000), "2 d");
    }
}
