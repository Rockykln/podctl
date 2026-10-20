use std::process::{Command, Stdio};

use crate::model::{AudioState, Profile};

#[derive(Clone, Debug)]
pub struct SinkInfo {
    pub name: String,
    pub address: String,
    pub id: u32,
    pub is_default: bool,
}

#[derive(Clone, Debug)]
pub struct CardInfo {
    pub name: String,
    pub address: String,
    pub active_profile: String,
    pub profiles: Vec<String>,
    pub codec: Option<String>,
}

/// PipeWire and PulseAudio name a device's sink and card after its
/// address: `bluez_output.AA_BB_CC_DD_EE_01.1` (the suffix differs
/// between WirePlumber versions), `bluez_card.AA_BB_CC_DD_EE_01`.
fn bluez_prefix(kind: &str, addr: &str) -> String {
    format!("{kind}.{}.", addr.replace(':', "_"))
}

/// The sink of the device at `addr`, or `None` when it has none — it is
/// disconnected, or its card sits in the `off` profile.
pub fn sink_for(addr: &str) -> Option<SinkInfo> {
    let default = default_sink_name().ok();
    let out = pactl(&["list", "sinks", "short"]).ok()?;
    pick_sink(&out, addr, default.as_deref())
}

fn pick_sink(out: &str, addr: &str, default: Option<&str>) -> Option<SinkInfo> {
    let want = bluez_prefix("bluez_output", addr);
    for line in out.lines() {
        let mut cols = line.split('\t');
        let Some(id) = cols.next().and_then(|c| c.parse::<u32>().ok()) else {
            continue;
        };
        let name = cols.next()?.to_string();
        if !name.starts_with(&want) {
            continue;
        }
        let is_default = default == Some(name.as_str());
        return Some(SinkInfo {
            name,
            address: addr.to_string(),
            id,
            is_default,
        });
    }
    None
}

/// Whether that device's sink is actually playing something right now
/// (PipeWire reports `RUNNING`; idle and suspended sinks carry no audio).
pub fn sink_running_for(addr: &str) -> bool {
    let Ok(out) = pactl(&["list", "sinks", "short"]) else {
        return false;
    };
    sink_is_running(&out, addr)
}

fn sink_is_running(out: &str, addr: &str) -> bool {
    let want = bluez_prefix("bluez_output", addr);
    out.lines().any(|l| {
        let cols: Vec<&str> = l.split('\t').collect();
        cols.get(1).is_some_and(|n| n.starts_with(&want)) && cols.last() == Some(&"RUNNING")
    })
}

/// The card of the device at `addr`. Looked up by name rather than
/// through its sink: in the `off` profile there is no sink, and deriving
/// one from the other left `profile`, `codec` and `latency` with
/// "no AirPods card found" and no way back.
pub fn card_for(addr: &str) -> Option<CardInfo> {
    let target = format!("bluez_card.{}", addr.replace(':', "_"));
    let out = pactl(&["list", "cards"]).ok()?;
    parse_card_block(&out, &target)
}

fn parse_card_block(out: &str, target: &str) -> Option<CardInfo> {
    let mut current: Vec<String> = Vec::new();
    let mut blocks: Vec<Vec<String>> = Vec::new();
    for line in out.lines() {
        if line.starts_with("Card #") && !current.is_empty() {
            blocks.push(std::mem::take(&mut current));
        }
        current.push(line.to_string());
    }
    if !current.is_empty() {
        blocks.push(current);
    }

    for block in blocks {
        let name_line = block.iter().find(|l| l.trim_start().starts_with("Name:"))?;
        let name = name_line
            .trim_start()
            .trim_start_matches("Name:")
            .trim()
            .to_string();
        if name != target {
            continue;
        }
        let address = extract_address(&name)?;
        let active_profile = block
            .iter()
            .find(|l| l.trim_start().starts_with("Active Profile:"))
            .map(|l| {
                l.trim_start()
                    .trim_start_matches("Active Profile:")
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();
        let mut profiles = Vec::new();
        let mut in_profiles = false;
        for line in &block {
            if line.trim_start().starts_with("Profiles:") {
                in_profiles = true;
                continue;
            }
            if in_profiles {
                if !line.starts_with("\t\t") && !line.starts_with("        ") {
                    break;
                }
                if let Some(idx) = line.find(':') {
                    let n = line[..idx].trim().to_string();
                    if !n.is_empty() {
                        profiles.push(n);
                    }
                }
            }
        }
        let codec = extract_codec(&active_profile);
        return Some(CardInfo {
            name,
            address,
            active_profile,
            profiles,
            codec,
        });
    }
    None
}

fn extract_address(name: &str) -> Option<String> {
    let body = name
        .strip_prefix("bluez_output.")
        .or_else(|| name.strip_prefix("bluez_card."))?;
    let mac = body.split('.').next()?;
    let with_colons = mac.replace('_', ":");
    if with_colons.len() == 17 && with_colons.chars().filter(|c| *c == ':').count() == 5 {
        Some(with_colons)
    } else {
        None
    }
}

fn extract_codec(profile: &str) -> Option<String> {
    if profile == "a2dp-sink" {
        return Some("aac".into());
    }
    profile.strip_prefix("a2dp-sink-").map(|s| s.to_string())
}

pub fn default_sink_name() -> anyhow::Result<String> {
    Ok(pactl(&["get-default-sink"])?.trim().to_string())
}

pub fn set_volume(sink: &SinkInfo, percent: u8) -> anyhow::Result<()> {
    pactl(&["set-sink-volume", &sink.name, &format!("{percent}%")])?;
    Ok(())
}

/// Volume in tenths of a percent, for fades finer than whole steps.
/// Written as a raw PulseAudio value (65536 = 100 %) so no decimal
/// separator is involved.
pub fn set_volume_tenths(sink: &SinkInfo, tenths: u16) -> anyhow::Result<()> {
    let raw = u32::from(tenths) * 65536 / 1000;
    pactl(&["set-sink-volume", &sink.name, &raw.to_string()])?;
    Ok(())
}

pub fn set_muted(sink: &SinkInfo, muted: bool) -> anyhow::Result<()> {
    pactl(&["set-sink-mute", &sink.name, if muted { "1" } else { "0" }])?;
    Ok(())
}

pub fn set_default(sink: &SinkInfo) -> anyhow::Result<()> {
    pactl(&["set-default-sink", &sink.name])?;
    Ok(())
}

pub fn set_profile(card: &CardInfo, kind: Profile) -> anyhow::Result<()> {
    let target = pick_profile(card, kind)?;
    pactl(&["set-card-profile", &card.name, &target])?;
    Ok(())
}

fn pick_profile(card: &CardInfo, kind: Profile) -> anyhow::Result<String> {
    let prefs: &[&str] = match kind {
        Profile::HighQuality => &[
            "a2dp-sink",
            "a2dp-sink-aac",
            "a2dp-sink-sbc_xq",
            "a2dp-sink-sbc",
        ],
        Profile::Headset => &[
            "headset-head-unit",
            "headset-head-unit-msbc",
            "headset-head-unit-cvsd",
        ],
        Profile::Off => &["off"],
    };
    for pref in prefs {
        if card.profiles.iter().any(|p| p == pref) {
            return Ok((*pref).to_string());
        }
    }
    anyhow::bail!(
        "no matching profile for '{}' (available: {})",
        kind.as_str(),
        card.profiles.join(", ")
    )
}

pub fn set_codec(card: &CardInfo, codec: &str) -> anyhow::Result<()> {
    let codec = codec.trim().to_ascii_lowercase();
    let target = if codec == "aac" {
        "a2dp-sink".to_string()
    } else {
        format!("a2dp-sink-{codec}")
    };
    if !card.profiles.contains(&target) {
        anyhow::bail!(
            "codec '{codec}' not available (try one of: {})",
            available_codecs(card).join(", ")
        );
    }
    pactl(&["set-card-profile", &card.name, &target])?;
    Ok(())
}

pub fn available_codecs(card: &CardInfo) -> Vec<String> {
    let mut out: Vec<String> = card
        .profiles
        .iter()
        .filter_map(|p| {
            if p == "a2dp-sink" {
                Some("aac".to_string())
            } else {
                p.strip_prefix("a2dp-sink-").map(|c| c.to_string())
            }
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

pub fn set_latency_offset(card: &CardInfo, ms: i32) -> anyhow::Result<()> {
    let usec = (ms as i64).saturating_mul(1_000).to_string();
    let _ = pactl(&[
        "set-port-latency-offset",
        &card.name,
        "headphone-output",
        &usec,
    ]);
    let _ = pactl(&[
        "set-port-latency-offset",
        &card.name,
        "headphone-hf-output",
        &usec,
    ]);
    Ok(())
}

pub fn snapshot(addr: &str) -> AudioState {
    let Some(sink) = sink_for(addr) else {
        return AudioState::default();
    };
    let card = card_for(addr);
    let volume_percent = parse_sink_volume_percent(&sink.name);
    let muted = parse_sink_mute(&sink.name);
    let (profile, codec, available) = match &card {
        Some(c) => (
            logical_profile(&c.active_profile),
            c.codec.clone(),
            available_codecs(c),
        ),
        None => (None, None, Vec::new()),
    };
    AudioState {
        volume_percent,
        muted,
        profile,
        codec,
        available_codecs: available,
        is_default_sink: sink.is_default,
        latency_offset_ms: 0,
    }
}

fn logical_profile(active: &str) -> Option<Profile> {
    if active.is_empty() || active == "off" {
        Some(Profile::Off)
    } else if active.starts_with("a2dp-sink") {
        Some(Profile::HighQuality)
    } else if active.starts_with("headset-head-unit") {
        Some(Profile::Headset)
    } else {
        None
    }
}

fn parse_sink_volume_percent(name: &str) -> Option<u8> {
    let out = pactl(&["get-sink-volume", name]).ok()?;
    let pct = out.split('%').next()?.rsplit([' ', '/']).next()?.trim();
    pct.parse::<u8>().ok().map(|v| v.min(150))
}

fn parse_sink_mute(name: &str) -> bool {
    pactl(&["get-sink-mute", name])
        .map(|s| s.contains("yes"))
        .unwrap_or(false)
}

fn pactl(args: &[&str]) -> anyhow::Result<String> {
    // pactl's gettext-translated booleans ("Mute: yes" → "Mute: ja" on a
    // German system) and translated mute/profile labels break the parser
    // on non-C locales. Force C across the board.
    let out = Command::new("pactl")
        .env("LC_ALL", "C")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| anyhow::anyhow!("pactl not in PATH? ({e})"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("pactl {:?} failed: {}", args, err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A command that writes raw s16le samples of everything playing to
/// `sink` on stdout. `pw-record` captures the sink directly; PipeWire's
/// pulse `.monitor` source hands `parec` silence for Bluetooth sinks, so
/// that route is only the fallback on a plain PulseAudio box.
pub fn capture_cmd(sink: &str, rate: u32, channels: u32) -> Option<Command> {
    if have("pw-record") {
        let mut c = Command::new("pw-record");
        c.args(["--target", sink])
            .args(["-P", "{ stream.capture.sink=true }"])
            .args(["--format", "s16"])
            .args(["--rate", &rate.to_string()])
            .args(["--channels", &channels.to_string()])
            .arg("-");
        return Some(c);
    }
    if have("parec") {
        let mut c = Command::new("parec");
        c.arg(format!("--device={sink}.monitor"))
            .arg(format!("--rate={rate}"))
            .arg(format!("--channels={channels}"))
            .args(["--format=s16le", "--raw"]);
        return Some(c);
    }
    None
}

fn have(bin: &str) -> bool {
    Command::new(bin)
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn have_pactl() -> bool {
    Command::new("pactl")
        .env("LC_ALL", "C")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{pick_sink, sink_is_running};

    /// A Bluetooth speaker and the AirPods, in that order — which is
    /// what made taking the first `bluez_output.*` send the volume,
    /// the profile and the ducking to the speaker.
    const SINKS: &str = "\
42\tbluez_output.FF_EE_DD_CC_BB_AA.1\tPipeWire\ts16le 2ch 44100Hz\tRUNNING
43\tbluez_output.AA_BB_CC_DD_EE_01.1\tPipeWire\ts16le 2ch 44100Hz\tIDLE
44\talsa_output.pci-0000_00_1f.3.analog-stereo\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED";

    #[test]
    fn picks_the_sink_of_one_device() {
        let s = pick_sink(SINKS, "AA:BB:CC:DD:EE:01", None).expect("AirPods sink");
        assert_eq!(s.name, "bluez_output.AA_BB_CC_DD_EE_01.1");
        assert_eq!(s.id, 43);
        assert!(!s.is_default);
        assert!(pick_sink(SINKS, "11:22:33:44:55:66", None).is_none());
    }

    #[test]
    fn marks_the_default_sink() {
        let default = Some("bluez_output.AA_BB_CC_DD_EE_01.1");
        let s = pick_sink(SINKS, "AA:BB:CC:DD:EE:01", default).expect("AirPods sink");
        assert!(s.is_default);
        let other = pick_sink(SINKS, "FF:EE:DD:CC:BB:AA", default).expect("speaker sink");
        assert!(!other.is_default);
    }

    /// The speaker is the one playing here; the ducking used to measure
    /// and lower whatever it found first.
    #[test]
    fn running_is_per_device() {
        assert!(sink_is_running(SINKS, "FF:EE:DD:CC:BB:AA"));
        assert!(!sink_is_running(SINKS, "AA:BB:CC:DD:EE:01"));
    }

    /// WirePlumber has named the sink `.a2dp-sink` and `.1` over the
    /// years; neither suffix may be matched on.
    #[test]
    fn suffix_does_not_matter() {
        let older = "7\tbluez_output.AA_BB_CC_DD_EE_01.a2dp-sink\tPipeWire\ts16le\tIDLE";
        let s = pick_sink(older, "AA:BB:CC:DD:EE:01", None).expect("AirPods sink");
        assert_eq!(s.id, 7);
    }
}
