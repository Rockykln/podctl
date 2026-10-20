use serde::{Deserialize, Serialize};

use crate::caps::Capabilities;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Battery {
    pub left: Option<u8>,
    pub right: Option<u8>,
    pub case: Option<u8>,
    pub left_charging: bool,
    pub right_charging: bool,
    pub case_charging: bool,
}

impl Battery {
    /// True once at least one component has reported a level. Used to
    /// tell "the device says nothing yet" from "the device told us".
    pub fn any_known(&self) -> bool {
        self.left.is_some() || self.right.is_some() || self.case.is_some()
    }
}

/// JSON spells these the way the command line takes them, so a value
/// read out of `--json` can be passed straight back in. The old
/// snake_case spellings are still accepted when reading.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Mode {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "anc", alias = "noise_cancellation")]
    NoiseCancellation,
    #[serde(rename = "transparency")]
    Transparency,
    #[serde(rename = "adaptive")]
    Adaptive,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::NoiseCancellation => "anc",
            Mode::Transparency => "transparency",
            Mode::Adaptive => "adaptive",
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "passive" => Ok(Mode::Off),
            "anc" | "nc" | "noise" | "noise-cancellation" => Ok(Mode::NoiseCancellation),
            "transparency" | "tr" | "trans" | "pass" => Ok(Mode::Transparency),
            "adaptive" | "ad" | "adapt" => Ok(Mode::Adaptive),
            _ => Err(format!("unknown mode '{s}'")),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConvAwareness {
    On,
    Off,
}

impl std::str::FromStr for ConvAwareness {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "on" | "1" | "true" | "yes" => Ok(ConvAwareness::On),
            "off" | "0" | "false" | "no" => Ok(ConvAwareness::Off),
            _ => Err(format!("expected 'on' or 'off', got '{s}'")),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Profile {
    #[serde(rename = "high", alias = "high_quality")]
    HighQuality,
    #[serde(rename = "headset")]
    Headset,
    #[serde(rename = "off")]
    Off,
}

impl Profile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Profile::HighQuality => "high",
            Profile::Headset => "headset",
            Profile::Off => "off",
        }
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Profile {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "high" | "a2dp" | "music" | "stereo" => Ok(Profile::HighQuality),
            "headset" | "hsp" | "hfp" | "call" | "mic" => Ok(Profile::Headset),
            "off" | "disabled" | "none" => Ok(Profile::Off),
            _ => Err(format!("unknown profile '{s}'")),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SpatialAudio {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "fixed")]
    Fixed,
    #[serde(rename = "head-tracked", alias = "head_tracked")]
    HeadTracked,
}

impl SpatialAudio {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpatialAudio::Off => "off",
            SpatialAudio::Fixed => "fixed",
            SpatialAudio::HeadTracked => "head-tracked",
        }
    }
}

impl std::fmt::Display for SpatialAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SpatialAudio {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" => Ok(SpatialAudio::Off),
            "fixed" | "stereo" => Ok(SpatialAudio::Fixed),
            "head-tracked" | "head" | "tracked" | "ht" => Ok(SpatialAudio::HeadTracked),
            _ => Err(format!("unknown spatial-audio mode '{s}'")),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MicSelection {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "left", alias = "always_left")]
    AlwaysLeft,
    #[serde(rename = "right", alias = "always_right")]
    AlwaysRight,
}

impl MicSelection {
    pub fn as_str(&self) -> &'static str {
        match self {
            MicSelection::Auto => "auto",
            MicSelection::AlwaysLeft => "left",
            MicSelection::AlwaysRight => "right",
        }
    }
}

impl std::str::FromStr for MicSelection {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(MicSelection::Auto),
            "left" | "l" => Ok(MicSelection::AlwaysLeft),
            "right" | "r" => Ok(MicSelection::AlwaysRight),
            _ => Err(format!("expected 'auto', 'left' or 'right', got '{s}'")),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PressAction {
    #[serde(rename = "mode-cycle", alias = "mode_cycle")]
    ModeCycle,
    #[serde(rename = "siri")]
    Siri,
    #[serde(rename = "none")]
    None,
}

impl PressAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            PressAction::ModeCycle => "mode-cycle",
            PressAction::Siri => "siri",
            PressAction::None => "none",
        }
    }
}

impl std::str::FromStr for PressAction {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mode-cycle" | "cycle" | "mode" => Ok(PressAction::ModeCycle),
            "siri" => Ok(PressAction::Siri),
            "none" | "noop" | "off" => Ok(PressAction::None),
            _ => Err(format!("unknown press-action '{s}'")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EarStatus {
    #[default]
    Unknown,
    InEar,
    OutOfEar,
    InCase,
}

impl EarStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            EarStatus::Unknown => "—",
            EarStatus::InEar => "in ear",
            EarStatus::OutOfEar => "out",
            EarStatus::InCase => "in case",
        }
    }
}

impl std::fmt::Display for EarStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// AAP reports ear status as primary/secondary (the bud currently driving
/// audio is primary). We cannot recover left/right from the wire alone.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InEar {
    pub primary: EarStatus,
    pub secondary: EarStatus,
}

impl InEar {
    pub fn any_in_ear(&self) -> bool {
        self.primary == EarStatus::InEar || self.secondary == EarStatus::InEar
    }
    pub fn count_in_ear(&self) -> u8 {
        (self.primary == EarStatus::InEar) as u8 + (self.secondary == EarStatus::InEar) as u8
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PressCounts {
    pub left_single: u32,
    pub left_double: u32,
    pub left_triple: u32,
    pub left_long: u32,
    pub right_single: u32,
    pub right_double: u32,
    pub right_triple: u32,
    pub right_long: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceInfo {
    pub firmware_left: Option<String>,
    pub firmware_right: Option<String>,
    pub firmware_case: Option<String>,
    pub serial_left: Option<String>,
    pub serial_right: Option<String>,
    pub serial_case: Option<String>,
    pub color: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AudioState {
    pub volume_percent: Option<u8>,
    pub muted: bool,
    pub profile: Option<Profile>,
    pub codec: Option<String>,
    pub available_codecs: Vec<String>,
    pub is_default_sink: bool,
    pub latency_offset_ms: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BtState {
    pub rssi_dbm: Option<i16>,
    pub paired: bool,
    pub trusted: bool,
    pub auto_connect: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairedDevice {
    pub address: String,
    pub name: String,
    pub model: Option<String>,
    pub connected: bool,
    pub trusted: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PodSettings {
    pub mode: Option<Mode>,
    pub conv_awareness: Option<ConvAwareness>,
    pub spatial_audio: Option<SpatialAudio>,
    pub ear_detection: Option<bool>,
    pub mic_selection: Option<MicSelection>,
    pub loud_sound_reduction: Option<bool>,
    pub press_action_left: Option<PressAction>,
    pub press_action_right: Option<PressAction>,
    pub tone_on_press: Option<bool>,
    pub custom_name: Option<String>,
    pub one_bud_anc: Option<bool>,
    pub chime_volume: Option<u8>,
    pub auto_anc_level: Option<u8>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DeviceState {
    pub connected: bool,
    pub address: Option<String>,
    pub name: Option<String>,
    pub capabilities: Capabilities,
    pub info: DeviceInfo,
    pub battery: Battery,
    /// Whether the AAP/L2CAP channel is open. `connected` only reflects
    /// BlueZ; the two come apart exactly when battery data goes missing,
    /// so the distinction is worth showing.
    #[serde(default)]
    pub aap_linked: bool,
    pub in_ear: InEar,
    pub case_lid_open: Option<bool>,
    pub press_counts: PressCounts,
    pub settings: PodSettings,
    pub audio: AudioState,
    pub bluetooth: BtState,
    pub updated_at: i64,
}

/// Device names come from the device, over the air, and end up in a
/// terminal, a tooltip and a notification. Control characters in one
/// can move the cursor, clear the line or repaint what is already
/// printed, and bidi overrides can make a name read as something else
/// entirely, so neither survives the trip.
pub fn sanitize_display_name(raw: &str) -> String {
    const MAX_CHARS: usize = 64;
    raw.chars()
        .filter(|c| !c.is_control())
        .filter(|c| {
            !matches!(
                c,
                '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
            )
        })
        .take(MAX_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Connected {
        name: String,
        address: String,
    },
    Disconnected,
    Battery(Battery),
    InEar(InEar),
    CaseLid {
        open: bool,
    },
    Mode {
        mode: Mode,
    },
    ConvAwareness {
        conv: ConvAwareness,
    },
    Press {
        side: Side,
        kind: PressKind,
    },
    SettingsChanged,
    /// Ask any running popup to show itself (e.g. `podctl popup`).
    ShowPopup,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PressKind {
    Single,
    Double,
    Triple,
    Long,
}

#[cfg(test)]
mod tests {
    /// What JSON says and what the command line takes have to be the
    /// same word: `podctl mode "$(podctl status --json | jq -r …)"`
    /// used to fail because JSON said "noise_cancellation".
    #[test]
    fn json_spells_values_the_way_the_cli_does() {
        use super::{MicSelection, Mode, PressAction, Profile, SpatialAudio};
        fn same<T: serde::Serialize>(value: T, as_str: &str) {
            let json = serde_json::to_string(&value).expect("serialises");
            assert_eq!(json, format!("\"{as_str}\""));
        }
        for m in [
            Mode::Off,
            Mode::NoiseCancellation,
            Mode::Transparency,
            Mode::Adaptive,
        ] {
            same(m, m.as_str());
            assert_eq!(m.as_str().parse::<Mode>(), Ok(m));
        }
        for p in [Profile::HighQuality, Profile::Headset, Profile::Off] {
            same(p, p.as_str());
            assert_eq!(p.as_str().parse::<Profile>(), Ok(p));
        }
        for s in [
            SpatialAudio::Off,
            SpatialAudio::Fixed,
            SpatialAudio::HeadTracked,
        ] {
            same(s, s.as_str());
            assert_eq!(s.as_str().parse::<SpatialAudio>(), Ok(s));
        }
        for m in [
            MicSelection::Auto,
            MicSelection::AlwaysLeft,
            MicSelection::AlwaysRight,
        ] {
            same(m, m.as_str());
            assert_eq!(m.as_str().parse::<MicSelection>(), Ok(m));
        }
        for a in [PressAction::ModeCycle, PressAction::Siri, PressAction::None] {
            same(a, a.as_str());
        }
    }

    /// Anything written by an older podctl still reads back.
    #[test]
    fn the_old_spellings_still_parse() {
        use super::{MicSelection, Mode, Profile};
        assert_eq!(
            serde_json::from_str::<Mode>("\"noise_cancellation\"").expect("old spelling"),
            Mode::NoiseCancellation
        );
        assert_eq!(
            serde_json::from_str::<Profile>("\"high_quality\"").expect("old spelling"),
            Profile::HighQuality
        );
        assert_eq!(
            serde_json::from_str::<MicSelection>("\"always_left\"").expect("old spelling"),
            MicSelection::AlwaysLeft
        );
    }

    use super::{Battery, ConvAwareness, Event, Mode, sanitize_display_name};

    #[test]
    fn a_name_cannot_steer_the_terminal() {
        // Clear screen, carriage return, a bidi override: all three are
        // things a device may put in its name.
        let got = sanitize_display_name("Air\u{1b}[2Jpods\r\u{202e}dangerous");
        assert_eq!(got, "Air[2Jpodsdangerous");
        let padded = sanitize_display_name("  Test AirPods  ");
        assert_eq!(padded, "Test AirPods");
        assert_eq!(sanitize_display_name("\u{7}\u{0}"), "");
    }

    #[test]
    fn a_name_cannot_fill_the_screen() {
        let long = "A".repeat(200);
        assert_eq!(sanitize_display_name(&long).chars().count(), 64);
    }

    /// Anything that is simply a name stays untouched, emoji included.
    #[test]
    fn ordinary_names_are_left_alone() {
        for name in ["AirPods Pro", "Test AirPods 🎧", "Käthes Hörer"] {
            assert_eq!(sanitize_display_name(name), name);
        }
    }

    #[test]
    fn mode_and_conv_events_carry_a_named_value() {
        let mode = serde_json::to_string(&Event::Mode {
            mode: Mode::Transparency,
        })
        .unwrap();
        assert_eq!(mode, r#"{"event":"mode","mode":"transparency"}"#);
        let conv = serde_json::to_string(&Event::ConvAwareness {
            conv: ConvAwareness::On,
        })
        .unwrap();
        assert_eq!(conv, r#"{"event":"conv_awareness","conv":"on"}"#);
    }

    #[test]
    fn any_known_needs_one_component() {
        assert!(!Battery::default().any_known());
        let case_only = Battery {
            case: Some(47),
            ..Battery::default()
        };
        assert!(case_only.any_known());
        // A charging flag on its own is not a level.
        let flag_only = Battery {
            left_charging: true,
            ..Battery::default()
        };
        assert!(!flag_only.any_known());
    }
}
