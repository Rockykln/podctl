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
    // Different versions of podctl and podctld is the usual cause of
    // "the setter does nothing" after an update without a restart.
    match podctl::client::hello() {
        Some(Some(h)) => {
            let _ = writeln!(out, "  podctld       {} (protocol {})", h.version, h.proto);
        }
        Some(None) => {
            let _ = writeln!(out, "  podctld       ?  (older than the handshake)");
        }
        None => {}
    }
    if let Some(note) = podctl::client::version_mismatch() {
        let _ = writeln!(out, "  mismatch      {note}");
    }
    let mut key_stored = None;
    let daemon_state = daemon_status();
    match &daemon_state {
        Some(st) => {
            let _ = writeln!(out, "  bluez link    {}", yesno(st.connected));
            let _ = writeln!(out, "  aap link      {}", yesno(st.aap_linked));
            let _ = writeln!(out, "  battery data  {}", yesno(st.battery.any_known()));
            key_stored = st
                .address
                .as_deref()
                .map(|mac| podctl::keys::path(mac).exists());
        }
        None => {
            let _ = writeln!(out, "  bluez link    ?  (no answer from daemon)");
        }
    }
    let _ = writeln!(out, "  ble listening {}", yesno(podctl::config::load().ble));
    let _ = writeln!(
        out,
        "  proximity key {}",
        match key_stored {
            Some(true) => "stored",
            Some(false) => "not yet asked for",
            None => "?",
        }
    );

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
            let numbers = daemon_state
                .as_ref()
                .map(|st| st.info.model_numbers.join(" · "))
                .filter(|n| !n.is_empty());
            let _ = writeln!(out, "  model no.     {}", numbers.as_deref().unwrap_or("?"));
            // AAP layouts depend on the firmware, so every bug starts here.
            let fw = daemon_state.as_ref().map(|st| &st.info);
            let _ = writeln!(
                out,
                "  firmware      {}",
                match fw {
                    Some(i) if i.firmware_left.is_some() || i.firmware_right.is_some() => format!(
                        "L {} · R {} · case {}",
                        i.firmware_left.as_deref().unwrap_or("?"),
                        i.firmware_right.as_deref().unwrap_or("?"),
                        i.firmware_case.as_deref().unwrap_or("?")
                    ),
                    _ => "?  (connect them and ask the daemon)".into(),
                }
            );
            // The modalias is a guess from the model code; the daemon
            // has the settings dump.
            let (caps, caps_from) = match &daemon_state {
                Some(st) => (st.capabilities, "daemon"),
                None => (dev.capabilities(), "modalias"),
            };
            let _ = writeln!(
                out,
                "  caps ({caps_from:<8}) anc:{} trans:{} adaptive:{} conv:{} loud:{}",
                yesno(caps.has_anc),
                yesno(caps.has_transparency),
                yesno(caps.has_adaptive),
                yesno(caps.has_conv_awareness),
                yesno(caps.has_loud_sound_reduction)
            );
            let _ = writeln!(
                out,
                "                spatial:{} press-hold:{} press-vol:{} tone:{}",
                yesno(caps.has_spatial_audio),
                yesno(caps.has_press_and_hold),
                yesno(caps.has_press_volume),
                yesno(caps.has_tone_on_press)
            );
            let _ = writeln!(
                out,
                "                ear-det:{} ear-det-setting:{} mic-sel:{}",
                yesno(caps.has_in_ear_detection),
                yesno(caps.has_ear_detection_setting),
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
    match airpods_sink() {
        Some(s) => {
            let _ = writeln!(out, "  sink          {}", redact_node(&s.name, no_redact));
            let _ = writeln!(out, "  default sink  {}", yesno(s.is_default));
        }
        None => {
            let _ = writeln!(out, "  (no AirPods sink found in PipeWire)");
        }
    }
    if let Some(c) = airpods_card() {
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
    let state = airpods_audio_state();
    if let Some(v) = state.volume_percent {
        let _ = writeln!(
            out,
            "  volume        {}% (muted: {})",
            v,
            yesno(state.muted)
        );
    }

    // Tray and popup run as systemd *user* units wanted by
    // graphical-session.target. Two things silently break them on
    // compositors that don't integrate with systemd: the target never
    // activates (so they don't autostart at login), and the user manager
    // never learns WAYLAND_DISPLAY / DISPLAY (so the popup falls back to
    // plain notifications instead of the layer-shell bubble).
    section(&mut out, "desktop services");
    let _ = writeln!(
        out,
        "  graphical-session       {}",
        unit_state("graphical-session.target")
    );
    for unit in ["podctld", "podctl-tray", "podctl-popup"] {
        let _ = writeln!(
            out,
            "  {unit:<22}  {}",
            unit_state(&format!("{unit}.service"))
        );
    }
    if let Some(state) = gnome_extension_state() {
        let _ = writeln!(out, "  gnome extension         {state}");
    }
    for var in ["WAYLAND_DISPLAY", "DISPLAY"] {
        let here = std::env::var_os(var).is_some();
        let systemd = systemd_env_has(var);
        // Unreachable is not missing: under sudo, over ssh without
        // lingering and in containers the manager cannot be asked, and
        // the report used to blame the session for it.
        let (shown, note) = match systemd {
            None => ("?", "  (user manager not reachable)"),
            Some(true) => ("yes", ""),
            Some(false) if here => ("no", "  <- set here but NOT in the systemd user env"),
            Some(false) => ("no", ""),
        };
        let _ = writeln!(
            out,
            "  {var:<22}  shell {} / systemd {shown}{note}",
            yesno(here)
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
    for var in ["PODCTL_LOG", "RUST_LOG"] {
        let _ = writeln!(
            out,
            "  {var:<22}   {}",
            std::env::var(var).unwrap_or_else(|_| "(unset)".into())
        );
    }

    section(&mut out, "logs");
    let _ = writeln!(out, "  {JOURNAL_CMD} -n {LOG_LINES}");
    match journal_tail(LOG_LINES) {
        Ok(lines) if lines.is_empty() => {
            let _ = writeln!(out, "  (nothing in the journal for these units)");
        }
        Ok(lines) => {
            let literals = local_literals(daemon_state.as_ref());
            for line in lines {
                let _ = writeln!(out, "  {}", redact_text(&line, &literals, no_redact));
            }
        }
        Err(why) => {
            let _ = writeln!(out, "  (no journal: {why})");
        }
    }

    out
}

const LOG_LINES: usize = 40;
const JOURNAL_CMD: &str = "journalctl --user -u podctld -u podctl-tray -u podctl-popup";

/// The last lines of the three services, or why there are none.
fn journal_tail(n: usize) -> Result<Vec<String>, String> {
    let mut cmd = Command::new("journalctl");
    cmd.args(["--user", "--no-pager", "-o", "short-iso", "-n"])
        .arg(n.to_string());
    for unit in ["podctld", "podctl-tray", "podctl-popup"] {
        cmd.args(["-u", unit]);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("journalctl not available ({e})"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            "journalctl refused".into()
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// Strings that exist only on this machine. A journal line carries the
/// hostname, and the daemon logs serial numbers at debug level.
fn local_literals(state: Option<&podctl::DeviceState>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(h) = host_name() {
        out.push(h);
    }
    for key in ["USER", "LOGNAME"] {
        if let Ok(v) = std::env::var(key) {
            out.push(v);
        }
    }
    if let Some(st) = state {
        let info = &st.info;
        for serial in [&info.serial_left, &info.serial_right, &info.serial_case]
            .into_iter()
            .flatten()
        {
            out.push(serial.clone());
        }
        // Only a name the user chose; an Apple default says nothing.
        if let Some(name) = &st.name
            && redact_name(name, false) != *name
        {
            out.push(name.clone());
        }
    }
    out.retain(|l| l.chars().count() >= 3);
    out.sort();
    out.dedup();
    // Longest first, so a name that contains another is not cut in half.
    out.sort_by_key(|l| std::cmp::Reverse(l.len()));
    out
}

fn host_name() -> Option<String> {
    let out = Command::new("uname").arg("-n").output().ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// A journal line is free text, which the field-by-field redaction above
/// does not reach into.
fn redact_text(line: &str, literals: &[String], no_redact: bool) -> String {
    if no_redact {
        return line.to_string();
    }
    let mut s = mask_macs(line);
    if let Ok(home) = std::env::var("HOME")
        && home.len() > 1
    {
        s = s.replace(&home, "~");
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR")
        && dir.starts_with("/run/user/")
    {
        s = s.replace(&dir, "/run/user/<uid>");
    }
    for lit in literals {
        s = s.replace(lit.as_str(), "<redacted>");
    }
    s
}

/// `AA:BB:CC:DD:EE:FF` and the `_` spelling PipeWire uses, anywhere in a
/// line. The OUI stays: it only says "Apple".
fn mask_macs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        if let Some(sep) = mac_prefix(rest) {
            out.push_str(&rest[..8]);
            for _ in 0..3 {
                out.push(sep);
                out.push_str("**");
            }
            rest = &rest[17..];
        } else {
            let ch = rest.chars().next().expect("not empty");
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

/// `Some(separator)` when `s` starts with six hex pairs.
fn mac_prefix(s: &str) -> Option<char> {
    let b = s.as_bytes();
    if b.len() < 17 {
        return None;
    }
    let sep = b[2];
    if sep != b':' && sep != b'_' {
        return None;
    }
    for group in 0..6 {
        let at = group * 3;
        if !b[at].is_ascii_hexdigit() || !b[at + 1].is_ascii_hexdigit() {
            return None;
        }
        if group < 5 && b[at + 2] != sep {
            return None;
        }
    }
    Some(sep as char)
}

/// Only on GNOME: whether the extension the popup draws through is
/// installed and running.
fn gnome_extension_state() -> Option<String> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if !desktop.split(':').any(|d| d.eq_ignore_ascii_case("gnome")) {
        return None;
    }
    let installed = podctl::xdg::gnome_extension_dirs("podctl@rockykln.com")
        .iter()
        .any(|d| d.join("metadata.json").exists());
    let running = Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.NameHasOwner",
            "string:com.rockykln.podctl1.Shell",
        ])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("boolean true"));
    Some(match (installed, running) {
        (_, Some(true)) => "running".into(),
        (true, _) => "installed, not running (log out and back in once; if it stays off: podctl install --gnome-extension)".into(),
        (false, _) => "not installed (podctl install --gnome-extension; until then the popup falls back to X11 or notifications)".into(),
    })
}

fn section(out: &mut String, name: &str) {
    let _ = writeln!(out);
    let _ = writeln!(out, "[{name}]");
}

/// The sink, card and audio state of the paired AirPods; another
/// Bluetooth device's would say nothing about them.
fn airpods_sink() -> Option<audio::SinkInfo> {
    audio::sink_for(&bluez::primary_airpods_address()?)
}

fn airpods_card() -> Option<audio::CardInfo> {
    audio::card_for(&bluez::primary_airpods_address()?)
}

fn airpods_audio_state() -> podctl::model::AudioState {
    match bluez::primary_airpods_address() {
        Some(addr) => audio::snapshot(&addr),
        None => Default::default(),
    }
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

/// `active` / `inactive` / `not installed` for a systemd user unit.
fn unit_state(unit: &str) -> String {
    let Ok(out) = Command::new("systemctl")
        .args(["--user", "is-active", unit])
        .output()
    else {
        return "? (no systemctl)".into();
    };
    let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if state.is_empty() {
        return "?".into();
    }
    // `is-active` says "inactive" both for a stopped unit and for one
    // that was never installed; `cat` tells the two apart.
    if state != "active" {
        let installed = Command::new("systemctl")
            .args(["--user", "cat", unit])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !installed {
            return "not installed".into();
        }
    }
    state
}

/// Whether the systemd *user manager* carries `var`. The session sets
/// this via `import-environment` / `dbus-update-activation-environment`;
/// without it the tray and popup start blind.
/// `None` when the manager did not answer at all.
fn systemd_env_has(var: &str) -> Option<bool> {
    let out = Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.split('=').next() == Some(var)),
    )
}

/// One-shot `status` against the daemon. `None` if it isn't answering —
/// the caller already reports reachability separately.
fn daemon_status() -> Option<podctl::DeviceState> {
    let mut client = podctl::client::Client::connect()?;
    // Generous: `status` may nudge the device for a battery dump first.
    let wait = std::time::Duration::from_millis(2500);
    match client.request(&podctl::Request::Status, wait).ok()? {
        podctl::Response::State(st) => Some(st),
        _ => None,
    }
}

fn daemon_pong() -> bool {
    let Some(mut client) = podctl::client::Client::connect() else {
        return false;
    };
    let wait = std::time::Duration::from_millis(500);
    matches!(
        client.request(&podctl::Request::Ping, wait),
        Ok(podctl::Response::Pong)
    )
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

#[cfg(test)]
mod tests {
    use super::{mask_macs, redact_text};

    /// Both spellings, in the middle of a sentence, and the OUI kept.
    #[test]
    fn a_mac_in_a_log_line_is_masked() {
        assert_eq!(
            mask_macs("link to AA:BB:CC:DD:EE:01 up"),
            "link to AA:BB:CC:**:**:** up"
        );
        assert_eq!(
            mask_macs("sink bluez_output.AA_BB_CC_DD_EE_01.1"),
            "sink bluez_output.AA_BB_CC_**_**_**.1"
        );
    }

    /// Words that look like one but are not.
    #[test]
    fn other_text_is_left_alone() {
        for line in [
            "12:34:56 started",
            "time 15:16:55 ok",
            "AA:BB:CC:DD:EE up",
            "codec aac, 44100 Hz · peak -12.5 dBFS",
        ] {
            assert_eq!(mask_macs(line), line, "{line}");
        }
    }

    /// The hostname and a custom device name are what identify a person
    /// in a journal line pasted into an issue.
    #[test]
    fn local_strings_are_replaced() {
        let literals = vec!["Test AirPods".to_string(), "host1".to_string()];
        assert_eq!(
            redact_text("host1 podctld[7]: Test AirPods linked", &literals, false),
            "<redacted> podctld[7]: <redacted> linked"
        );
    }

    #[test]
    fn no_redact_keeps_everything() {
        let literals = vec!["host1".to_string()];
        let line = "host1 podctld[7]: AA:BB:CC:DD:EE:01";
        assert_eq!(redact_text(line, &literals, true), line);
    }
}
