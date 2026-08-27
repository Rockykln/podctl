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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Off,
    NoiseCancellation,
    Transparency,
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
#[serde(rename_all = "snake_case")]
pub enum Profile {
    HighQuality,
    Headset,
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
#[serde(rename_all = "snake_case")]
pub enum SpatialAudio {
    Off,
    Fixed,
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
#[serde(rename_all = "snake_case")]
pub enum MicSelection {
    Auto,
    AlwaysLeft,
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
#[serde(rename_all = "snake_case")]
pub enum PressAction {
    ModeCycle,
    Siri,
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
    Mode(Mode),
    ConvAwareness(ConvAwareness),
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
    use super::Battery;

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
