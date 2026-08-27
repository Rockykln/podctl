//! Output formatters for the CLI. Plain text — no ANSI colour by default
//! so output stays pipeable into grep/awk.

use podctl::model::{
    Battery, DeviceState, EarStatus, Event, InEar, MicSelection, PairedDevice, PressKind, Profile,
    Side,
};

pub fn print_state(verb: &str, s: &DeviceState) {
    match verb {
        "battery" | "bat" | "b" => print_battery(&s.battery, s.connected, s.aap_linked),
        _ => print_full(s),
    }
}

fn print_full(s: &DeviceState) {
    if !s.connected {
        println!("disconnected");
        if let Some(addr) = &s.address {
            let suffix = s
                .name
                .as_deref()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            println!("  paired  {addr}{suffix}");
        }
        return;
    }
    let name = s.name.as_deref().unwrap_or("AirPods");
    println!("{}  —  {}", name, s.capabilities.model.label());
    if let Some(addr) = &s.address {
        println!("  address       {addr}");
    }
    if let Some(fw_l) = &s.info.firmware_left {
        let fw_r = s.info.firmware_right.as_deref().unwrap_or("?");
        let fw_c = s.info.firmware_case.as_deref();
        let case_part = fw_c.map(|c| format!(" / case {c}")).unwrap_or_default();
        println!("  firmware      L {fw_l} / R {fw_r}{case_part}");
    }
    if let Some(color) = &s.info.color {
        println!("  color         {color}");
    }

    println!();
    print_battery(&s.battery, s.connected, s.aap_linked);

    if s.capabilities.has_in_ear_detection {
        println!();
        println!("  in-ear        {}", format_in_ear(&s.in_ear));
    }
    if let Some(open) = s.case_lid_open {
        println!("  case lid      {}", if open { "open" } else { "closed" });
    }

    // Bud settings
    println!();
    if let Some(m) = s.settings.mode {
        println!("  mode          {m}");
    }
    if let Some(c) = s.settings.conv_awareness {
        println!("  conv          {}", on_off(c == podctl::ConvAwareness::On));
    }
    if let Some(sa) = s.settings.spatial_audio {
        println!("  spatial       {sa}");
    }
    if let Some(ed) = s.settings.ear_detection {
        println!("  ear-detection {}", on_off(ed));
    }
    if let Some(mic) = s.settings.mic_selection {
        let label = match mic {
            MicSelection::Auto => "auto",
            MicSelection::AlwaysLeft => "left",
            MicSelection::AlwaysRight => "right",
        };
        println!("  mic           {label}");
    }
    if let Some(lr) = s.settings.loud_sound_reduction {
        println!("  loud-reduce   {}", on_off(lr));
    }
    if let Some(pa_l) = s.settings.press_action_left {
        let pa_r = s
            .settings
            .press_action_right
            .map(|a| a.as_str())
            .unwrap_or("?");
        println!("  press         L {} / R {}", pa_l.as_str(), pa_r);
    }
    if let Some(tone) = s.settings.tone_on_press {
        println!("  tone          {}", on_off(tone));
    }
    if let Some(v) = s.settings.one_bud_anc {
        println!("  one-bud-anc   {}", on_off(v));
    }
    if let Some(v) = s.settings.chime_volume {
        println!("  chime         {v}");
    }
    if let Some(v) = s.settings.auto_anc_level {
        println!("  auto-anc      {v}");
    }

    // Audio
    if s.audio != Default::default() {
        println!();
        println!("  audio");
        if let Some(v) = s.audio.volume_percent {
            let muted = if s.audio.muted { "  (muted)" } else { "" };
            println!("    volume      {v}%{muted}");
        }
        if let Some(p) = s.audio.profile {
            let prof = match p {
                Profile::HighQuality => "high (a2dp_sink)",
                Profile::Headset => "headset",
                Profile::Off => "off",
            };
            println!("    profile     {prof}");
        }
        if let Some(c) = &s.audio.codec {
            println!("    codec       {c}");
        }
        if !s.audio.available_codecs.is_empty() {
            println!("    available   {}", s.audio.available_codecs.join(", "));
        }
        if s.audio.is_default_sink {
            println!("    default     yes");
        }
        if s.audio.latency_offset_ms != 0 {
            println!("    latency     {} ms", s.audio.latency_offset_ms);
        }
    }

    // Bluetooth side
    println!();
    println!("  bluetooth");
    if let Some(r) = s.bluetooth.rssi_dbm {
        println!("    rssi        {r} dBm");
    }
    println!("    paired      {}", on_off(s.bluetooth.paired));
    println!("    trusted     {}", on_off(s.bluetooth.trusted));
    println!("    auto-conn   {}", on_off(s.bluetooth.auto_connect));

    // Presses
    let pc = &s.press_counts;
    let any = [
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
    .any(|&v| v > 0);
    if any {
        println!();
        println!("  press counts");
        println!(
            "    L  1×{}  2×{}  3×{}  hold {}",
            pc.left_single, pc.left_double, pc.left_triple, pc.left_long
        );
        println!(
            "    R  1×{}  2×{}  3×{}  hold {}",
            pc.right_single, pc.right_double, pc.right_triple, pc.right_long
        );
    }

    if s.updated_at > 0 {
        let age = now_secs().saturating_sub(s.updated_at);
        println!();
        println!("  updated       {age}s ago");
    }
}

fn print_battery(b: &Battery, connected: bool, aap_linked: bool) {
    if !connected && !b.any_known() {
        println!("battery — no data (disconnected)");
        return;
    }
    print!("battery   ");
    print_cell("L", b.left, b.left_charging);
    print!("   ");
    print_cell("R", b.right, b.right_charging);
    print!("   ");
    print_cell("C", b.case, b.case_charging);
    println!();
    // Empty cells on a live device are ambiguous — say which half is
    // missing rather than leaving the user guessing at three dashes.
    if connected && !b.any_known() {
        if aap_linked {
            println!("          (AAP link up, device has not reported a level yet)");
        } else {
            println!("          (no AAP link — battery needs PSM 0x1001; see 'podctl debug')");
        }
    }
}

fn print_cell(label: &str, val: Option<u8>, charging: bool) {
    match val {
        Some(v) => print!("{label} {:>3}%{}", v, if charging { " ⚡" } else { "  " }),
        None => print!("{label}  ---   "),
    }
}

pub fn print_list(items: &[PairedDevice]) {
    if items.is_empty() {
        println!("no paired AirPods on this adapter.");
        return;
    }
    println!(
        "{:<18}  {:<24}  {:<22}  {:<10}  trusted",
        "address", "name", "model", "state"
    );
    println!("{}", "─".repeat(86));
    for d in items {
        let state = if d.connected { "connected" } else { "offline" };
        let trust = if d.trusted { "yes" } else { "no" };
        let model = d.model.as_deref().unwrap_or("?");
        println!(
            "{:<18}  {:<24}  {:<22}  {:<10}  {}",
            d.address, d.name, model, state, trust
        );
    }
}

pub fn print_event(e: &Event) {
    match e {
        Event::Connected { name, address } => println!("connected     {name}  ({address})"),
        Event::Disconnected => println!("disconnected"),
        Event::Battery(b) => {
            let f = |o: Option<u8>| o.map(|v| format!("{v}%")).unwrap_or_else(|| "—".into());
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
        Event::Mode(m) => println!("mode          {m}"),
        Event::ConvAwareness(c) => {
            println!("conv          {}", on_off(*c == podctl::ConvAwareness::On))
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
/// primary/secondary), but on this user's Pro 2 USB-C the left bud is
/// always primary — so we label accordingly. Set `PODS_NO_LR=1` to
/// fall back to primary/secondary if your unit behaves differently.
fn format_in_ear(in_ear: &InEar) -> String {
    use EarStatus::*;
    let no_lr = std::env::var_os("PODS_NO_LR").is_some();
    let (a_label, b_label) = if no_lr {
        ("primary", "secondary")
    } else {
        ("L", "R")
    };
    match (in_ear.primary, in_ear.secondary) {
        (InEar, InEar) => "both in ear".into(),
        (OutOfEar, OutOfEar) => "both out of ear".into(),
        (InCase, InCase) => "both in case".into(),
        (Unknown, Unknown) => "—".into(),
        (a, b) => format!("{a_label} {a},  {b_label} {b}"),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
