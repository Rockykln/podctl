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

pub fn primary_sink() -> Option<SinkInfo> {
    let default = default_sink_name().ok();
    let out = pactl(&["list", "sinks", "short"]).ok()?;
    for line in out.lines() {
        let mut cols = line.split('\t');
        let id: u32 = cols.next()?.parse().ok()?;
        let name = cols.next()?.to_string();
        if !name.starts_with("bluez_output.") {
            continue;
        }
        let address = extract_address(&name)?;
        let is_default = default.as_deref() == Some(name.as_str());
        return Some(SinkInfo {
            name,
            address,
            id,
            is_default,
        });
    }
    None
}

/// Whether the AirPods sink is actually playing something right now
/// (PipeWire reports `RUNNING`; idle and suspended sinks carry no audio).
pub fn primary_sink_running() -> bool {
    let Ok(out) = pactl(&["list", "sinks", "short"]) else {
        return false;
    };
    out.lines().any(|l| {
        let cols: Vec<&str> = l.split('\t').collect();
        cols.get(1).is_some_and(|n| n.starts_with("bluez_output."))
            && cols.last() == Some(&"RUNNING")
    })
}

pub fn primary_card() -> Option<CardInfo> {
    let sink = primary_sink()?;
    let target = format!("bluez_card.{}", sink.address.replace(':', "_"));
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

pub fn snapshot() -> AudioState {
    let Some(sink) = primary_sink() else {
        return AudioState::default();
    };
    let card = primary_card();
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
