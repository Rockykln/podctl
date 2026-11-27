use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use crate::caps::{Capabilities, Model};
use crate::model::{BtState, PairedDevice};

pub type Addr = String;

#[derive(Clone, Debug)]
pub struct DeviceSummary {
    pub address: Addr,
    pub name: String,
    pub connected: bool,
    pub trusted: bool,
    pub paired: bool,
    /// `None` on a BlueZ too old to say.
    pub bonded: Option<bool>,
    pub modalias: Option<String>,
    pub icon: Option<String>,
    /// Signal strength, and with it the answer to "was this device
    /// actually heard just now?". BlueZ sets it while a scan runs and
    /// drops it again afterwards, so it has to be read in the same
    /// breath as the rest, not later.
    pub rssi_dbm: Option<i16>,
    /// True for an LE advertisement whose address rotates, which is not
    /// a device one can pair with.
    pub random_address: bool,
}

impl DeviceSummary {
    pub fn is_apple(&self) -> bool {
        self.modalias
            .as_deref()
            .map(|m| m.starts_with("bluetooth:v004C") || m.starts_with("bluetooth:v004c"))
            .unwrap_or(false)
    }

    pub fn is_airpods(&self) -> bool {
        // Apple vendor, and either the headphones icon (covers a yet
        // unknown product code) or a product code we recognise as
        // AirPods. Relying on the icon alone dropped real AirPods when
        // BlueZ reported no/!= "audio-headphones" Icon, which then read
        // as Unknown model → Mode/Conv disabled.
        if self.modalias.is_none() {
            // A paired device that has not connected since the adapter
            // came up can lack its Modalias; the name is all there is.
            return self.name.to_lowercase().contains("airpods");
        }
        self.is_apple()
            && (self.icon.as_deref() == Some("audio-headphones") || self.model() != Model::Unknown)
    }

    pub fn model(&self) -> Model {
        let Some(m) = self.modalias.as_deref() else {
            return Model::Unknown;
        };
        let Some(p_idx) = m.find('p') else {
            return Model::Unknown;
        };
        let tail = &m[p_idx + 1..];
        let code_str = &tail[..tail.len().min(4)];
        u16::from_str_radix(code_str, 16)
            .map(Model::from_code)
            .unwrap_or(Model::Unknown)
    }

    pub fn capabilities(&self) -> Capabilities {
        self.model().capabilities()
    }
}

pub fn paired_airpods() -> anyhow::Result<Vec<DeviceSummary>> {
    let out = bluetoothctl(&["devices", "Paired"])?;
    let mut summaries = Vec::new();
    for addr in device_addresses(&out) {
        if let Ok(s) = info(&addr)
            && s.is_airpods()
        {
            summaries.push(s);
        }
    }
    summaries.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(summaries)
}

pub fn info(addr: &str) -> anyhow::Result<DeviceSummary> {
    let out = bluetoothctl(&["info", addr])?;
    Ok(DeviceSummary {
        address: addr.to_string(),
        name: field(&out, "Name:")
            .map(|n| crate::model::sanitize_display_name(&n))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| addr.to_string()),
        connected: field(&out, "Connected:")
            .map(|v| v == "yes")
            .unwrap_or(false),
        trusted: field(&out, "Trusted:").map(|v| v == "yes").unwrap_or(false),
        paired: field(&out, "Paired:").map(|v| v == "yes").unwrap_or(false),
        bonded: field(&out, "Bonded:").map(|v| v == "yes"),
        modalias: field(&out, "Modalias:"),
        icon: field(&out, "Icon:"),
        rssi_dbm: field(&out, "RSSI:").and_then(|v| v.parse::<i16>().ok()),
        random_address: random_address(&out),
    })
}

/// BlueZ opens its report with the address type: "Device AA:… (public)".
/// Connected AirPods also advertise over LE under an address that
/// rotates, and `pair` offered those as fresh candidates — a pair
/// already paired, under a name the user recognised, that it then could
/// not pair with.
fn random_address(out: &str) -> bool {
    out.lines()
        .next()
        .map(strip_ansi)
        .is_some_and(|l| l.trim_end().ends_with("(random)"))
}

/// The addresses in a `bluetoothctl devices` listing. BlueZ 5.86 and
/// later print some lines in grey, and an escape code in front of
/// "Device" made those devices vanish: paired AirPods were then missing
/// from `list`, and `pair` did not know them.
fn device_addresses(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|line| {
            let line = strip_ansi(line);
            let mut parts = line.splitn(3, ' ');
            (parts.next() == Some("Device")).then(|| parts.next().map(str::to_string))?
        })
        .collect()
}

fn field(out: &str, key: &str) -> Option<String> {
    for line in out.lines() {
        if let Some(rest) = line.trim_start().strip_prefix(key) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// How long a connect may take. Some controllers page for well over
/// half a minute before BlueZ gives up, longer than the CLI waits for
/// the daemon, so the user saw a timeout instead of the reason.
pub const CONNECT_LIMIT: Duration = Duration::from_secs(20);

pub fn connect(addr: &str) -> anyhow::Result<()> {
    let res = match bluetoothctl_within(&["connect", addr], CONNECT_LIMIT) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => {
            // BlueZ keeps paging after the tool is gone and answers the
            // next try with "busy"; a disconnect calls the attempt off.
            let _ = bluetoothctl(&["disconnect", addr]);
            Err(anyhow::anyhow!(
                "bluetoothctl connect: page-timeout after {}s",
                CONNECT_LIMIT.as_secs()
            ))
        }
        Err(e) => Err(e),
    };
    res.map_err(|e| match why_not_connected(&e.to_string()) {
        Some(plain) => anyhow::anyhow!(plain),
        None => e,
    })
}

/// The reasons BlueZ gives for a failed connect, in words that say what
/// to do. Without the tool's name in them they count as unavailable.
fn why_not_connected(reason: &str) -> Option<&'static str> {
    if reason.contains("page-timeout") || reason.contains("Host is down") {
        Some("the AirPods did not answer: open the case or take them out, and keep them close.")
    } else if reason.contains("connection-busy") || reason.contains("InProgress") {
        Some("Bluetooth is busy with another connection, try again in a moment.")
    } else if reason.contains("NotReady") {
        Some("Bluetooth is off; switch it on first.")
    } else {
        None
    }
}

pub fn disconnect(addr: &str) -> anyhow::Result<()> {
    bluetoothctl(&["disconnect", addr])?;
    Ok(())
}

pub fn set_trusted(addr: &str, on: bool) -> anyhow::Result<()> {
    bluetoothctl(&[if on { "trust" } else { "untrust" }, addr])?;
    Ok(())
}

pub fn set_alias(addr: &str, alias: &str) -> anyhow::Result<()> {
    let adapter = first_adapter()
        .ok_or_else(|| anyhow::anyhow!("no bluetooth adapter (/sys/class/bluetooth/hci*) found"))?;
    let dev_path = format!("/org/bluez/{}/dev_{}", adapter, addr.replace(':', "_"));
    // Not even a name we write ourselves gets to carry control
    // characters: it comes back to us through BlueZ later.
    let alias = crate::model::sanitize_display_name(alias);
    let variant = format!("variant:string:{alias}");
    let out = Command::new("dbus-send")
        .env("LC_ALL", "C")
        .args([
            "--system",
            "--print-reply",
            "--dest=org.bluez",
            &dev_path,
            "org.freedesktop.DBus.Properties.Set",
            "string:org.bluez.Device1",
            "string:Alias",
            &variant,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| anyhow::anyhow!("dbus-send not in PATH? ({e})"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("set-alias via D-Bus failed: {}", err.trim());
    }
    Ok(())
}

fn first_adapter() -> Option<String> {
    // Multi-adapter override: `PODCTL_ADAPTER=hci1` pins the adapter
    // regardless of enumeration order. Empty value behaves like unset.
    if let Ok(val) = std::env::var("PODCTL_ADAPTER")
        && !val.trim().is_empty()
    {
        return Some(val.trim().to_string());
    }
    std::fs::read_dir("/sys/class/bluetooth")
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("hci") && !n.contains(':'))
}

pub fn unpair(addr: &str) -> anyhow::Result<()> {
    bluetoothctl(&["remove", addr])?;
    Ok(())
}

/// Pair what the caller named, or what a scan turned up — but never
/// just the first thing in range. Anything in pairing mode within a few
/// metres shows up here, a neighbour's buds included, and pairing also
/// trusts the device.
pub fn pair_choice(address: Option<String>, yes: bool) -> anyhow::Result<()> {
    if let Some(addr) = address {
        if crate::ble::parse_addr(&addr).is_none() {
            anyhow::bail!("'{addr}' is not a Bluetooth address (AA:BB:CC:DD:EE:FF)");
        }
        return pair(&addr);
    }
    let found = discover(12)?;
    let (known, mut fresh): (Vec<DeviceSummary>, Vec<DeviceSummary>) = found
        .into_iter()
        .filter(|d| !d.random_address)
        .partition(|d| d.paired);
    fresh.sort_by(|a, b| a.name.cmp(&b.name));
    match fresh.as_slice() {
        // `bluetoothctl devices` lists everything BlueZ remembers, so a
        // paired set is in there whether or not it is anywhere near.
        // Saying "already paired" about a pair in another room hides the
        // only thing that happened: the scan found nothing new.
        [] => match heard_among(&known) {
            Some(d) => anyhow::bail!(
                "{} ({}) are already paired — use 'podctl connect'",
                d.name,
                d.address
            ),
            None => anyhow::bail!("{NOTHING_IN_PAIRING_MODE}"),
        },
        [one] if yes => pair(&one.address),
        several => {
            let list = several
                .iter()
                .map(|d| format!("  {}  {}", d.address, d.name))
                .collect::<Vec<_>>()
                .join("\n");
            let hint = if several.len() == 1 {
                "pair it with 'podctl pair --yes', or name the address"
            } else {
                "name the address of the one you want"
            };
            anyhow::bail!("{CHOOSE}\n{list}\n{hint}")
        }
    }
}

/// Which paired set to name when the scan turned up nothing new: only
/// one the scan actually heard. Everything BlueZ remembers is in the
/// listing, so without this the message was about whichever pair came
/// first — one left at the office included.
fn heard_among(known: &[DeviceSummary]) -> Option<&DeviceSummary> {
    known.iter().find(|d| d.rssi_dbm.is_some())
}

/// Starts the message when `pair` found candidates and needs to be told
/// which: a question back, not a failure.
const CHOOSE: &str = "in pairing mode right now:";

/// Not every pair has a button any more — the AirPods Pro 3 do not — so
/// the text says what to do without promising one.
const NOTHING_IN_PAIRING_MODE: &str = "no AirPods in pairing mode nearby — open the case next to this \
machine. Older models need the button on the back held until the LED blinks white; newer ones are put \
into pairing mode from the phone they belong to.";

pub fn discover(secs: u32) -> anyhow::Result<Vec<DeviceSummary>> {
    let res = bluetoothctl(&["--timeout", &secs.to_string(), "scan", "on"]);
    if res.is_err() {
        bluetoothctl(&["scan", "on"])?;
        std::thread::sleep(std::time::Duration::from_secs(secs as u64));
        let _ = bluetoothctl(&["scan", "off"]);
    }
    let out = bluetoothctl(&["devices"])?;
    let mut found = Vec::new();
    for addr in device_addresses(&out) {
        if let Ok(s) = info(&addr)
            && s.is_airpods()
        {
            found.push(s);
        }
    }
    Ok(found)
}

/// How long BlueZ gets to finish a pairing. It has to stay under the
/// wait a client allows for `pair` (`client::timeout_for`), scan time
/// included, or the CLI gives up on an answer the daemon is still owed.
pub const PAIR_LIMIT: Duration = Duration::from_secs(25);

pub fn pair(addr: &str) -> anyhow::Result<()> {
    // Debian ships the adapter not pairable. BlueZ then pairs anyway but
    // keeps no key, and the pairing is gone after the next disconnect.
    let was_pairable = adapter_pairable();
    if was_pairable == Some(false) {
        bluetoothctl(&["pairable", "on"])?;
    }
    let paired = bluetoothctl_within(&["pair", addr], PAIR_LIMIT);
    if was_pairable == Some(false) {
        let _ = bluetoothctl(&["pairable", "off"]);
    }
    if paired.as_ref().is_ok_and(|done| done.is_none()) {
        // A pairing left hanging in BlueZ makes it answer InProgress to
        // every later attempt, this program's and every other one's, so
        // take it back before saying so.
        let _ = bluetoothctl(&["cancel-pairing", addr]);
        anyhow::bail!(
            "pairing did not finish within {}s and was called off — make sure the AirPods are in pairing mode, then try again",
            PAIR_LIMIT.as_secs()
        );
    }
    paired?;
    let _ = bluetoothctl(&["trust", addr]);
    if info(addr).is_ok_and(|d| d.bonded == Some(false)) {
        anyhow::bail!(
            "paired, but BlueZ kept no key, so the pairing would be lost on the next disconnect — run 'podctl unpair' and pair again"
        );
    }
    Ok(())
}

fn adapter_pairable() -> Option<bool> {
    let out = bluetoothctl(&["show"]).ok()?;
    field(&out, "Pairable:").map(|v| v == "yes")
}

pub fn to_paired_device(s: &DeviceSummary) -> PairedDevice {
    PairedDevice {
        address: s.address.clone(),
        name: s.name.clone(),
        model: Some(s.model().label().into()),
        connected: s.connected,
        trusted: s.trusted,
    }
}

pub fn bt_state(s: &DeviceSummary) -> BtState {
    BtState {
        rssi_dbm: s.rssi_dbm,
        paired: s.paired,
        trusted: s.trusted,
        auto_connect: s.trusted,
    }
}

fn bluetoothctl_command(args: &[&str]) -> Command {
    tracing::debug!(?args, "bluetoothctl");
    // Force C locale so booleans like "yes"/"no" and field labels
    // ("Connected:", "Modalias:", …) are stable across distros even if a
    // future bluez version picks up a gettext catalog for them.
    let mut cmd = Command::new("bluetoothctl");
    cmd.env("LC_ALL", "C")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn bluetoothctl(args: &[&str]) -> anyhow::Result<String> {
    let out = bluetoothctl_command(args)
        .output()
        .map_err(|e| anyhow::anyhow!("bluetoothctl not in PATH? ({e})"))?;
    checked(args, out)
}

/// `None` when the limit ran out; the tool is killed then.
fn bluetoothctl_within(args: &[&str], limit: Duration) -> anyhow::Result<Option<String>> {
    let out = output_within(&mut bluetoothctl_command(args), limit)
        .map_err(|e| anyhow::anyhow!("bluetoothctl not in PATH? ({e})"))?;
    out.map(|out| checked(args, out)).transpose()
}

/// `Command::output` with a deadline. The pipes are read only at the
/// end, which holds for the few lines bluetoothctl prints.
fn output_within(cmd: &mut Command, limit: Duration) -> std::io::Result<Option<Output>> {
    let mut child = cmd.spawn()?;
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut s) = child.stdout.take() {
        s.read_to_end(&mut stdout)?;
    }
    if let Some(mut s) = child.stderr.take() {
        s.read_to_end(&mut stderr)?;
    }
    Ok(Some(Output {
        status,
        stdout,
        stderr,
    }))
}

fn checked(args: &[&str], out: Output) -> anyhow::Result<String> {
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let reason = failure_reason(err.trim(), &stdout);
        anyhow::bail!("bluetoothctl {:?} failed: {}", args, reason);
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `bluetoothctl` prints why a command failed on stdout ("Failed to
/// connect: org.bluez.Error…"), between the property changes it echoes.
fn failure_reason(stderr: &str, stdout: &str) -> String {
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    let lines = || stdout.lines().map(strip_ansi).map(|l| l.trim().to_string());
    lines()
        .rfind(|l| l.contains("Failed") || l.contains("org.bluez.Error"))
        .or_else(|| lines().rfind(|l| !l.is_empty() && !is_hex_dump(l)))
        .unwrap_or_else(|| "no reason given".into())
}

/// bluetoothctl prints raw packet dumps among its messages. Without this
/// a failed pair once told the user
/// "failed: 03 01 00 01 0a 01 28 00 4a 13".
fn is_hex_dump(line: &str) -> bool {
    line.split_whitespace()
        .take_while(|w| w.len() == 2 && w.chars().all(|c| c.is_ascii_hexdigit()))
        .count()
        >= 4
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Which kind of failure a bluetooth error is. `bluetoothctl` or
/// `dbus-send` missing or failing is a backend problem; a device that
/// did not answer may answer later. Every message from this module that
/// comes from one of the two names it.
pub fn error_code_for(message: &str) -> crate::ipc::ErrorCode {
    if message.starts_with(CHOOSE) {
        crate::ipc::ErrorCode::Usage
    } else if message.contains("bluetoothctl") || message.contains("dbus-send") {
        crate::ipc::ErrorCode::Backend
    } else {
        crate::ipc::ErrorCode::Unavailable
    }
}

pub fn have_bluetoothctl() -> bool {
    Command::new("bluetoothctl")
        .env("LC_ALL", "C")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Address of the AirPods to act on, for the audio helpers that need
/// one to tell their sink from every other Bluetooth device's.
pub fn primary_airpods_address() -> Option<String> {
    primary_airpods().map(|d| d.address)
}

pub fn primary_airpods() -> Option<DeviceSummary> {
    let all = paired_airpods().ok()?;
    all.iter()
        .find(|d| d.connected)
        .cloned()
        .or_else(|| all.first().cloned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_rotating_le_address_is_no_candidate() {
        // What BlueZ prints for the LE advertisement of a pair that is
        // already paired over the classic link.
        let out = "Device C0:BB:CC:DD:EE:03 (random)\n\tName: Test AirPods\n\tPaired: no\n";
        assert!(super::random_address(out));
        assert!(!super::random_address(
            "Device AA:BB:CC:DD:EE:01 (public)\n\tName: Test AirPods\n"
        ));
        assert!(!super::random_address(""));
    }

    #[test]
    fn only_a_pair_the_scan_heard_is_named() {
        let mut far = device("Test AirPods", None);
        far.rssi_dbm = None;
        let mut near = device("Other AirPods", None);
        near.address = "AA:BB:CC:DD:EE:02".into();
        near.rssi_dbm = Some(-52);
        // The one in the drawer at work must not be the answer to
        // "nothing new in pairing mode".
        assert!(super::heard_among(std::slice::from_ref(&far)).is_none());
        assert_eq!(
            super::heard_among(&[far, near]).map(|d| d.name.as_str()),
            Some("Other AirPods")
        );
    }

    #[test]
    fn a_packet_dump_is_not_a_failure_reason() {
        // This exact line once reached a user as the reason a pairing
        // failed, raw bytes and all.
        let dump = "03 01 00 01 0a 01 28 00 4a 13                    ......(.J.";
        assert!(super::is_hex_dump(dump));
        assert!(!super::is_hex_dump(
            "Failed to pair: org.bluez.Error.AuthenticationRejected"
        ));
        assert_eq!(
            super::failure_reason("", &format!("Attempting to pair\n{dump}\n")),
            "Attempting to pair"
        );
    }

    #[test]
    fn a_real_reason_beats_the_last_line() {
        let out = "Attempting to pair\nFailed to pair: org.bluez.Error.AuthenticationRejected\n03 01 00 01 0a\n";
        assert_eq!(
            super::failure_reason("", out),
            "Failed to pair: org.bluez.Error.AuthenticationRejected"
        );
        // stderr still wins when there is any.
        assert_eq!(
            super::failure_reason("no such device", out),
            "no such device"
        );
    }

    #[test]
    fn a_pairing_answers_before_the_client_gives_up() {
        // The CLI waits `timeout_for` for an answer; BlueZ gets less
        // than that, scan time included, so the daemon is never left
        // working on a request nobody is listening for any more.
        let waits = crate::client::timeout_for(&crate::Request::Pair {
            address: None,
            yes: false,
        });
        let scan = std::time::Duration::from_secs(12);
        assert!(
            super::PAIR_LIMIT + scan < waits,
            "pair limit {:?} plus the scan must stay under the client's {:?}",
            super::PAIR_LIMIT,
            waits
        );
    }

    #[test]
    fn a_tool_that_hangs_is_cut_off() {
        use std::process::Command;
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut slow = Command::new("sleep");
        slow.arg("5");
        let out = super::output_within(&mut slow, Duration::from_millis(200)).expect("spawn");
        assert!(out.is_none());
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        let mut quick = Command::new("echo");
        quick.arg("hi").stdout(std::process::Stdio::piped());
        let out = super::output_within(&mut quick, Duration::from_secs(5)).expect("spawn");
        assert_eq!(out.expect("finished").stdout, b"hi\n");
        let timeout = format!(
            "bluetoothctl connect: page-timeout after {}s",
            super::CONNECT_LIMIT.as_secs()
        );
        assert!(super::why_not_connected(&timeout).is_some());
    }

    fn device(name: &str, modalias: Option<&str>) -> super::DeviceSummary {
        super::DeviceSummary {
            address: "AA:BB:CC:DD:EE:01".into(),
            name: name.into(),
            connected: false,
            trusted: true,
            paired: true,
            bonded: Some(true),
            modalias: modalias.map(Into::into),
            icon: None,
            rssi_dbm: None,
            random_address: false,
        }
    }

    #[test]
    fn airpods_without_a_modalias_are_known_by_name() {
        assert!(device("Test AirPods", None).is_airpods());
        assert!(!device("Some Speaker", None).is_airpods());
        assert!(device("Buds", Some("bluetooth:v004Cp2024d0000")).is_airpods());
        assert!(!device("Test AirPods", Some("bluetooth:v0075p0100d0000")).is_airpods());
    }

    #[test]
    fn a_choice_to_make_is_a_usage_error() {
        let msg = format!("{}\n  AA:BB:CC:DD:EE:01  Test AirPods", super::CHOOSE);
        assert_eq!(super::error_code_for(&msg), crate::ipc::ErrorCode::Usage);
        assert_eq!(
            super::error_code_for("no new AirPods spotted"),
            crate::ipc::ErrorCode::Unavailable
        );
    }

    #[test]
    fn grey_lines_of_the_device_list_count_too() {
        let out = "Device AA:BB:CC:DD:EE:01 AirPods Pro\n\
                   \x1b[1;30mDevice AA:BB:CC:DD:EE:02 Test AirPods\x1b[0m\n\
                   [NEW] Controller 11:22:33:44:55:66 host1\n";
        assert_eq!(
            super::device_addresses(out),
            ["AA:BB:CC:DD:EE:01", "AA:BB:CC:DD:EE:02"]
        );
    }

    #[test]
    fn a_failed_connect_says_what_to_do() {
        let timeout = "bluetoothctl [\"connect\", \"AA:BB:CC:DD:EE:01\"] failed: \
                       Failed to connect: org.bluez.Error.Failed br-connection-page-timeout";
        let plain = super::why_not_connected(timeout).expect("known reason");
        assert!(plain.contains("did not answer"), "{plain}");
        assert_eq!(
            super::error_code_for(plain),
            crate::ipc::ErrorCode::Unavailable
        );
        assert!(super::why_not_connected("org.bluez.Error.Failed br-connection-busy").is_some());
        assert!(super::why_not_connected("org.bluez.Error.NotReady").is_some());
        assert_eq!(
            super::why_not_connected("org.bluez.Error.Failed something new"),
            None
        );
    }

    #[test]
    fn the_reason_for_a_failure_is_found_on_stdout() {
        let out = "Attempting to connect to AA:BB:CC:DD:EE:01\n\
                   [\x1b[0;93mCHG\x1b[0m] Device 11:22:33:44:55:66 RSSI: 0xffffffac (-84)\n\
                   Failed to connect: org.bluez.Error.Failed br-connection-busy\n";
        assert_eq!(
            super::failure_reason("", out),
            "Failed to connect: org.bluez.Error.Failed br-connection-busy"
        );
        assert_eq!(super::failure_reason("bad", out), "bad");
        assert_eq!(super::failure_reason("", ""), "no reason given");
    }

    /// A typo must not reach `bluetoothctl pair`, and nothing gets
    /// paired without being named or confirmed.
    #[test]
    fn an_address_has_to_look_like_one() {
        let err = super::pair_choice(Some("nonsense".into()), false)
            .expect_err("not an address")
            .to_string();
        assert!(err.contains("not a Bluetooth address"), "{err}");
    }
}
