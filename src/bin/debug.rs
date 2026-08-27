use std::fmt::Write as _;
use std::process::Command;

use podctl::{audio, bluez};

pub fn run(no_redact: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "=== podctl debug report ===");
    let _ = writeln!(
        out,
        "redacted: {}",
        if no_redact {
            "no (LOCAL ONLY, do not paste)"
        } else {
            "yes"
        }
    );
    let _ = writeln!(out);

    section(&mut out, "podctl");
    let _ = writeln!(out, "  version       {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "  license       MIT OR Apache-2.0");
    let _ = writeln!(
        out,
        "  profile       {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );

    section(&mut out, "system");
    let _ = writeln!(out, "  os            {}", os_release());
    let _ = writeln!(out, "  kernel        {}", uname_release());
    let _ = writeln!(out, "  arch          {}", std::env::consts::ARCH);
    let _ = writeln!(
        out,
        "  desktop       {}",
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "?".into())
    );
    let _ = writeln!(
        out,
        "  session       {}",
        std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "?".into())
    );

    section(&mut out, "tools");
    let _ = writeln!(
        out,
        "  pactl         {}",
        tool_version("pactl", &["--version"])
    );
    let _ = writeln!(
        out,
        "  wpctl         {}",
        tool_version("wpctl", &["--version"])
    );
    let _ = writeln!(
        out,
        "  bluetoothctl  {}",
        tool_version("bluetoothctl", &["--version"])
    );
    let _ = writeln!(out, "  bluez daemon  {}", bluez_daemon_status());

    section(&mut out, "daemon");
    let sock = podctl::socket_path();
    let sock_present = sock.exists();
    let _ = writeln!(
        out,
        "  socket        {}",
        redact_path(&sock.display().to_string(), no_redact)
    );
    let _ = writeln!(out, "  socket exists {}", yesno(sock_present));
    let _ = writeln!(out, "  reachable     {}", yesno(daemon_pong()));
    match daemon_status() {
        Some(st) => {
            let _ = writeln!(out, "  bluez link    {}", yesno(st.connected));
            let _ = writeln!(out, "  aap link      {}", yesno(st.aap_linked));
            let _ = writeln!(out, "  battery data  {}", yesno(st.battery.any_known()));
        }
        None => {
            let _ = writeln!(out, "  bluez link    ?  (no answer from daemon)");
        }
    }

    section(&mut out, "airpods");
    match bluez::primary_airpods() {
        Some(dev) => {
            let _ = writeln!(out, "  model         {}", dev.model().label());
            let _ = writeln!(
                out,
                "  address       {}",
                redact_mac(&dev.address, no_redact)
            );
            let _ = writeln!(out, "  name          {}", redact_name(&dev.name, no_redact));
            let _ = writeln!(out, "  connected     {}", yesno(dev.connected));
            let _ = writeln!(out, "  trusted       {}", yesno(dev.trusted));
            let _ = writeln!(out, "  paired        {}", yesno(dev.paired));
            let _ = writeln!(
                out,
                "  modalias      {}",
                dev.modalias.as_deref().unwrap_or("-")
            );
            let caps = dev.capabilities();
            let _ = writeln!(
                out,
                "  caps          anc:{} trans:{} adaptive:{} conv:{} loud:{}",
                yesno(caps.has_anc),
                yesno(caps.has_transparency),
                yesno(caps.has_adaptive),
                yesno(caps.has_conv_awareness),
                yesno(caps.has_loud_sound_reduction)
            );
            let _ = writeln!(
                out,
                "                spatial:{} press-hold:{} ear-det:{} mic-sel:{}",
                yesno(caps.has_spatial_audio),
                yesno(caps.has_press_and_hold),
                yesno(caps.has_in_ear_detection),
                yesno(caps.has_mic_selection)
            );
        }
        None => {
            let _ = writeln!(out, "  (no paired AirPods detected)");
        }
    }
    let total = bluez::paired_airpods().map(|v| v.len()).unwrap_or(0);
    let _ = writeln!(out, "  paired count  {total}");

    section(&mut out, "audio");
    match audio::primary_sink() {
        Some(s) => {
            let _ = writeln!(out, "  sink          {}", redact_node(&s.name, no_redact));
            let _ = writeln!(out, "  default sink  {}", yesno(s.is_default));
        }
        None => {
            let _ = writeln!(out, "  (no AirPods sink found in PipeWire)");
        }
    }
    if let Some(c) = audio::primary_card() {
        let _ = writeln!(out, "  card          {}", redact_node(&c.name, no_redact));
        let _ = writeln!(out, "  profile       {}", c.active_profile);
        let _ = writeln!(out, "  codec         {}", c.codec.as_deref().unwrap_or("-"));
        let _ = writeln!(out, "  profiles      {}", c.profiles.join(", "));
        let _ = writeln!(
            out,
            "  codecs        {}",
            audio::available_codecs(&c).join(", ")
        );
    }
    let state = audio::snapshot();
    if let Some(v) = state.volume_percent {
        let _ = writeln!(
            out,
            "  volume        {}% (muted: {})",
            v,
            yesno(state.muted)
        );
    }

    section(&mut out, "env");
    let _ = writeln!(
        out,
        "  XDG_RUNTIME_DIR present  {}",
        yesno(std::env::var_os("XDG_RUNTIME_DIR").is_some())
    );
    let _ = writeln!(
        out,
        "  XDG_CONFIG_HOME present  {}",
        yesno(std::env::var_os("XDG_CONFIG_HOME").is_some())
    );
    let _ = writeln!(
        out,
        "  RUST_LOG                 {}",
        std::env::var("RUST_LOG").unwrap_or_else(|_| "(unset)".into())
    );

    out
}

fn section(out: &mut String, name: &str) {
    let _ = writeln!(out);
    let _ = writeln!(out, "[{name}]");
}

fn yesno(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn tool_version(cmd: &str, args: &[&str]) -> String {
    if let Ok(o) = Command::new(cmd).args(args).output()
        && o.status.success()
    {
        let line = String::from_utf8_lossy(&o.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if !line.is_empty() {
            return line;
        }
    }
    match Command::new("which").arg(cmd).output() {
        Ok(o) if o.status.success() => "(installed)".into(),
        _ => "(not installed)".into(),
    }
}

fn os_release() -> String {
    if let Ok(s) = std::fs::read_to_string("/etc/os-release") {
        for line in s.lines() {
            if let Some(v) = line.strip_prefix("PRETTY_NAME=") {
                return v.trim_matches('"').to_string();
            }
        }
    }
    "?".into()
}

fn uname_release() -> String {
    Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "?".into())
}

fn bluez_daemon_status() -> String {
    match Command::new("systemctl")
        .args(["is-active", "bluetooth.service"])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "?".into(),
    }
}

/// One-shot `status` against the daemon. `None` if it isn't answering —
/// the caller already reports reachability separately.
fn daemon_status() -> Option<podctl::DeviceState> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let s = UnixStream::connect(podctl::socket_path()).ok()?;
    // Generous: `status` may nudge the device for a battery dump first.
    let _ = s.set_read_timeout(Some(Duration::from_millis(2500)));
    let mut w = &s;
    writeln!(w, r#"{{"op":"status"}}"#).ok()?;
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line).ok()?;
    match serde_json::from_str::<podctl::Response>(line.trim()).ok()? {
        podctl::Response::State(st) => Some(st),
        _ => None,
    }
}

fn daemon_pong() -> bool {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let Ok(s) = UnixStream::connect(podctl::socket_path()) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    let mut w = &s;
    if writeln!(w, r#"{{"op":"ping"}}"#).is_err() {
        return false;
    }
    let mut line = String::new();
    let mut r = BufReader::new(&s);
    r.read_line(&mut line).is_ok() && line.contains("\"kind\":\"pong\"")
}

// MAC: AA:BB:CC:DD:EE:FF -> AA:BB:CC:**:**:**  (Apple OUI stays, NIC bytes masked)
fn redact_mac(mac: &str, no_redact: bool) -> String {
    if no_redact {
        return mac.to_string();
    }
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return "<mac>".into();
    }
    format!("{}:{}:{}:**:**:**", parts[0], parts[1], parts[2])
}

// Pass through Apple-default names; mask anything custom.
fn redact_name(name: &str, no_redact: bool) -> String {
    if no_redact {
        return name.to_string();
    }
    let allow = [
        "AirPods",
        "AirPods Pro",
        "AirPods Pro 2",
        "AirPods Pro (2nd gen)",
        "AirPods 2",
        "AirPods 3",
        "AirPods 4",
        "AirPods Max",
    ];
    if allow.iter().any(|a| name.eq_ignore_ascii_case(a)) {
        name.to_string()
    } else {
        "<custom name redacted>".into()
    }
}

// bluez_output.AA_BB_CC_DD_EE_FF.1 -> bluez_output.AA_BB_CC_**_**_**.1
fn redact_node(node: &str, no_redact: bool) -> String {
    if no_redact {
        return node.to_string();
    }
    let Some(body) = node
        .strip_prefix("bluez_output.")
        .or_else(|| node.strip_prefix("bluez_card."))
    else {
        return node.to_string();
    };
    let prefix = if node.starts_with("bluez_output.") {
        "bluez_output."
    } else {
        "bluez_card."
    };
    let mut parts = body.splitn(2, '.');
    let mac = parts.next().unwrap_or("");
    let rest = parts.next();
    let mac_parts: Vec<&str> = mac.split('_').collect();
    if mac_parts.len() != 6 {
        return node.to_string();
    }
    let masked = format!(
        "{}_{}_{}_**_**_**",
        mac_parts[0], mac_parts[1], mac_parts[2]
    );
    match rest {
        Some(r) => format!("{prefix}{masked}.{r}"),
        None => format!("{prefix}{masked}"),
    }
}

fn redact_path(p: &str, no_redact: bool) -> String {
    if no_redact {
        return p.to_string();
    }
    if let Ok(home) = std::env::var("HOME")
        && let Some(rest) = p.strip_prefix(&home)
    {
        return format!("~{rest}");
    }
    if let Some(rest) = p.strip_prefix("/run/user/") {
        if let Some(idx) = rest.find('/') {
            return format!("/run/user/<uid>{}", &rest[idx..]);
        }
        return "/run/user/<uid>".into();
    }
    p.to_string()
}
