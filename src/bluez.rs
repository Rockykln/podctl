use std::process::{Command, Stdio};

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
    pub modalias: Option<String>,
    pub icon: Option<String>,
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
    for line in out.lines() {
        let mut parts = line.splitn(3, ' ');
        if parts.next().unwrap_or("") != "Device" {
            continue;
        }
        let addr = match parts.next() {
            Some(a) => a.to_string(),
            None => continue,
        };
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
        modalias: field(&out, "Modalias:"),
        icon: field(&out, "Icon:"),
    })
}

pub fn rssi(addr: &str) -> Option<i16> {
    let out = bluetoothctl(&["info", addr]).ok()?;
    field(&out, "RSSI:").and_then(|v| v.parse::<i16>().ok())
}

fn field(out: &str, key: &str) -> Option<String> {
    for line in out.lines() {
        if let Some(rest) = line.trim_start().strip_prefix(key) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

pub fn connect(addr: &str) -> anyhow::Result<()> {
    bluetoothctl(&["connect", addr])?;
    Ok(())
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

pub fn discover(secs: u32) -> anyhow::Result<Vec<DeviceSummary>> {
    let res = bluetoothctl(&["--timeout", &secs.to_string(), "scan", "on"]);
    if res.is_err() {
        bluetoothctl(&["scan", "on"])?;
        std::thread::sleep(std::time::Duration::from_secs(secs as u64));
        let _ = bluetoothctl(&["scan", "off"]);
    }
    let out = bluetoothctl(&["devices"])?;
    let mut found = Vec::new();
    for line in out.lines() {
        let mut parts = line.splitn(3, ' ');
        if parts.next() != Some("Device") {
            continue;
        }
        let addr = match parts.next() {
            Some(a) => a,
            None => continue,
        };
        if let Ok(s) = info(addr)
            && s.is_airpods()
        {
            found.push(s);
        }
    }
    Ok(found)
}

pub fn pair(addr: &str) -> anyhow::Result<()> {
    bluetoothctl(&["pair", addr])?;
    let _ = bluetoothctl(&["trust", addr]);
    Ok(())
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
        rssi_dbm: rssi(&s.address),
        paired: s.paired,
        trusted: s.trusted,
        auto_connect: s.trusted,
    }
}

fn bluetoothctl(args: &[&str]) -> anyhow::Result<String> {
    // Force C locale so booleans like "yes"/"no" and field labels
    // ("Connected:", "Modalias:", …) are stable across distros even if a
    // future bluez version picks up a gettext catalog for them.
    let out = Command::new("bluetoothctl")
        .env("LC_ALL", "C")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| anyhow::anyhow!("bluetoothctl not in PATH? ({e})"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("bluetoothctl {:?} failed: {}", args, err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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
