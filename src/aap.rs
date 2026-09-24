//! Apple Accessory Protocol over L2CAP PSM 0x1001.
//!
//! Byte layouts are reverse-engineered; markers below point at the bits
//! that still need confirming with a live device capture (btmon).

#![allow(dead_code)]

pub const AAP_PSM: u16 = 0x1001;
pub const FRAME_MAGIC: [u8; 4] = [0x04, 0x00, 0x04, 0x00];

pub mod op {
    pub const HANDSHAKE: u8 = 0x00;
    pub const BATTERY: u8 = 0x04;
    pub const EAR_DETECTION: u8 = 0x06;
    pub const SETTINGS: u8 = 0x09;
    pub const SUBSCRIBE_NOTIF: u8 = 0x0F;
    pub const METADATA: u8 = 0x1d;
    pub const RENAME: u8 = 0x1A;
    /// Also the request for the proximity keys (payload `05 00`).
    pub const REQUEST_INFO: u8 = 0x30;
    pub const PROXIMITY_KEYS: u8 = 0x31;
    pub const FEATURE_FLAGS: u8 = 0x4D;
    pub const CONV_AWARENESS_LVL: u8 = 0x4B;
}

/// IDs that ride inside a `SETTINGS` (0x09) notification.
/// Confirmed for AirPods Pro 2 USB-C against the LibrePods AAP definitions
/// (`docs/control_commands.md`).
pub mod setting {
    pub const MIC_MODE: u8 = 0x01; // 0=auto, 1=right, 2=left
    pub const EAR_DETECTION: u8 = 0x0a; // 1=enabled, 2=disabled
    pub const NOISE_CONTROL: u8 = 0x0d; // 1=Off, 2=NC, 3=Transparency, 4=Adaptive
    pub const ONE_BUD_ANC: u8 = 0x1b; // 1=enabled, 2=disabled
    pub const CHIME_VOLUME: u8 = 0x1f; // 0..100 (?)
    pub const CONV_AWARE: u8 = 0x28; // 1=enabled, 2=disabled
    pub const AUTO_ANC_LEVEL: u8 = 0x2e; // 0..100
}

/// Battery component IDs (in the 0x04 BATTERY notification).
pub mod battery_kind {
    pub const RIGHT: u8 = 0x02;
    pub const LEFT: u8 = 0x04;
    pub const CASE: u8 = 0x08;
}

/// Battery status byte values.
pub mod battery_status {
    pub const UNKNOWN: u8 = 0x00;
    pub const CHARGING: u8 = 0x01;
    pub const DISCHARGING: u8 = 0x02;
    pub const DISCONNECTED: u8 = 0x04;
}

/// Ear-detection pod-status byte values.
pub mod ear_status {
    pub const IN_EAR: u8 = 0x00;
    pub const OUT: u8 = 0x01;
    pub const IN_CASE: u8 = 0x02;
}

pub mod listening_mode {
    pub const OFF: u8 = 0x01;
    pub const NOISE_CANCEL: u8 = 0x02;
    pub const TRANSPARENCY: u8 = 0x03;
    pub const ADAPTIVE: u8 = 0x04;
}

pub mod features {
    pub const SPATIAL_AUDIO_FIXED: u32 = 1 << 0;
    pub const SPATIAL_AUDIO_HEAD_TRACKED: u32 = 1 << 1;
    pub const EAR_DETECTION: u32 = 1 << 2;
    pub const CONV_AWARENESS: u32 = 1 << 3;
    pub const LOUD_SOUND_REDUCTION: u32 = 1 << 4;
    pub const TONE_ON_PRESS: u32 = 1 << 5;
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub opcode: u8,
    pub subop: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(opcode: u8, subop: u8, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            opcode,
            subop,
            payload: payload.into(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(6 + self.payload.len());
        buf.extend_from_slice(&FRAME_MAGIC);
        buf.push(self.opcode);
        buf.push(self.subop);
        buf.extend_from_slice(&self.payload);
        buf
    }

    pub fn parse(buf: &[u8]) -> Option<Self> {
        // Outgoing frames use 04 00 04 00, incoming 04 00 04 00 too in steady
        // state; the handshake reply uses 01 00 04 00. Be lenient on byte 0
        // since the rest of the magic is stable.
        if buf.len() < 6 || buf[1..4] != FRAME_MAGIC[1..4] {
            return None;
        }
        Some(Self {
            opcode: buf[4],
            subop: buf[5],
            payload: buf[6..].to_vec(),
        })
    }
}

/// Decoded payload for a `SETTINGS` (0x09) notification. The setting ID
/// determines what the bytes mean; the rest of the daemon dispatches on it.
#[derive(Clone, Debug)]
pub struct SettingNotice<'a> {
    pub id: u8,
    pub data: &'a [u8],
}

impl<'a> SettingNotice<'a> {
    pub fn parse(frame: &'a Frame) -> Option<Self> {
        if frame.opcode != op::SETTINGS || frame.payload.is_empty() {
            return None;
        }
        Some(Self {
            id: frame.payload[0],
            data: &frame.payload[1..],
        })
    }
}

/// Ask the buds for their proximity keys: the identity resolving key
/// behind their rotating advertisement address, and the key over the
/// encrypted tail of that advertisement.
pub fn request_keys() -> Frame {
    Frame::new(op::REQUEST_INFO, 0x00, vec![0x05, 0x00])
}

/// The two 16-byte keys from a `PROXIMITY_KEYS` (0x31) reply. They are
/// TLVs after a leading byte: type, 16-bit length, one reserved byte,
/// then the key. Type 1 is the IRK, type 4 the advertisement key.
///
/// The buds hand both out least significant byte first, the reverse of
/// what the Core Specification's `ah` expects; they are turned around
/// here so the rest of the code sees spec order. Verified against 16
/// advertisement addresses captured from this pair.
pub struct ProximityKeys {
    pub irk: [u8; 16],
    pub enc: [u8; 16],
}

pub fn parse_keys(payload: &[u8]) -> Option<ProximityKeys> {
    let mut irk = None;
    let mut enc = None;
    let mut rest = payload.get(1..)?;
    while rest.len() >= 4 {
        let kind = rest[0];
        let len = u16::from_be_bytes([rest[1], rest[2]]) as usize;
        let body = rest.get(4..4 + len)?;
        if len == 16 {
            let mut key: [u8; 16] = body.try_into().ok()?;
            key.reverse();
            match kind {
                1 => irk = Some(key),
                4 => enc = Some(key),
                _ => {}
            }
        }
        rest = &rest[4 + len..];
    }
    Some(ProximityKeys {
        irk: irk?,
        enc: enc?,
    })
}

/// Write to a setting. Format: `04 00 04 00 09 00 [id] [value] 00 00 00`.
/// Used by noise-control / conv-awareness setters.
pub fn write_setting(id: u8, value: u8) -> Frame {
    Frame::new(op::SETTINGS, 0x00, vec![id, value, 0, 0, 0])
}

pub fn set_noise_control(mode: u8) -> Frame {
    write_setting(setting::NOISE_CONTROL, mode)
}

pub fn set_conv_awareness(on: bool) -> Frame {
    write_setting(setting::CONV_AWARE, if on { 0x01 } else { 0x02 })
}

pub fn set_ear_detection(on: bool) -> Frame {
    write_setting(setting::EAR_DETECTION, if on { 0x01 } else { 0x02 })
}

pub fn set_mic_mode(mode: u8) -> Frame {
    // 0=auto, 1=always-right, 2=always-left
    write_setting(setting::MIC_MODE, mode)
}

/// One component of the BATTERY (0x04) notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryComponent {
    pub kind: u8,   // battery_kind::*
    pub level: u8,  // 0..=100, or 0xFF = unknown
    pub status: u8, // battery_status::*
}

/// Decode a BATTERY notification payload (everything after `04 00 04 00 04 00`).
/// Layout: `[count] ([component] 01 [level] [status] 01) × count`.
pub fn parse_battery(payload: &[u8]) -> Vec<BatteryComponent> {
    if payload.is_empty() {
        return Vec::new();
    }
    let declared = payload[0] as usize;
    // Cap by what can actually fit in the rest of the payload (5 bytes
    // per component). Without this, a malformed frame with declared=255
    // would over-allocate even though the loop bails on the bounds check.
    let max_fit = payload.len().saturating_sub(1) / 5;
    let count = declared.min(max_fit);
    let mut out = Vec::with_capacity(count);
    let mut i = 1;
    for _ in 0..count {
        if i + 5 > payload.len() {
            break;
        }
        // payload[i+1] and payload[i+4] are the constant `0x01` framing bytes.
        out.push(BatteryComponent {
            kind: payload[i],
            level: payload[i + 2],
            status: payload[i + 3],
        });
        i += 5;
    }
    out
}

/// Decode an EAR_DETECTION (0x06) notification payload.
/// Layout: `[primary] [secondary]` where primary is the bud currently
/// driving audio. Returns `(primary_status, secondary_status)` raw bytes.
pub fn parse_ear(payload: &[u8]) -> Option<(u8, u8)> {
    if payload.len() < 2 {
        return None;
    }
    Some((payload[0], payload[1]))
}

fn ear_status_from_byte(b: u8) -> crate::model::EarStatus {
    match b {
        0x00 => crate::model::EarStatus::InEar,
        0x01 => crate::model::EarStatus::OutOfEar,
        0x02 => crate::model::EarStatus::InCase,
        _ => crate::model::EarStatus::Unknown,
    }
}

pub fn parse_in_ear(payload: &[u8]) -> Option<crate::model::InEar> {
    if payload.len() < 2 {
        return None;
    }
    Some(crate::model::InEar {
        primary: ear_status_from_byte(payload[0]),
        secondary: ear_status_from_byte(payload[1]),
    })
}

pub fn parse_press(payload: &[u8]) -> Option<(crate::model::Side, crate::model::PressKind)> {
    if payload.len() < 2 {
        return None;
    }
    let side = match payload[0] {
        0 => crate::model::Side::Left,
        1 => crate::model::Side::Right,
        _ => return None,
    };
    let kind = match payload[1] {
        1 => crate::model::PressKind::Single,
        2 => crate::model::PressKind::Double,
        3 => crate::model::PressKind::Triple,
        4 => crate::model::PressKind::Long,
        _ => return None,
    };
    Some((side, kind))
}

pub const APPLE_COMPANY_ID: u16 = 0x004C;
pub const PROXIMITY_PAIR_TYPE: u8 = 0x07;
pub const PROXIMITY_PAIR_LEN: u8 = 0x19;

pub fn parse_proximity_pair(data: &[u8]) -> Option<u16> {
    if data.len() < 8 {
        return None;
    }
    if data[0] != PROXIMITY_PAIR_TYPE || data[1] != PROXIMITY_PAIR_LEN {
        return None;
    }
    Some(u16::from_le_bytes([data[3], data[4]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EarStatus;

    #[test]
    fn frame_roundtrip() {
        let f = Frame::new(
            op::SETTINGS,
            0x00,
            vec![setting::NOISE_CONTROL, 0x02, 0, 0, 0],
        );
        let bytes = f.encode();
        assert_eq!(
            bytes,
            vec![
                0x04,
                0x00,
                0x04,
                0x00,
                0x09,
                0x00,
                setting::NOISE_CONTROL,
                0x02,
                0,
                0,
                0
            ]
        );
        let parsed = Frame::parse(&bytes).expect("parse roundtrip");
        assert_eq!(parsed.opcode, 0x09);
        assert_eq!(parsed.subop, 0x00);
        assert_eq!(parsed.payload, vec![setting::NOISE_CONTROL, 0x02, 0, 0, 0]);
    }

    #[test]
    fn frame_parse_accepts_handshake_reply_magic() {
        // Handshake response uses `01 00 04 00` as the first 4 bytes;
        // parser should still pick out opcode 0x00.
        let bytes = b"\x01\x00\x04\x00\x00\x00\x01\x00\x03\x00";
        let parsed = Frame::parse(bytes).expect("lenient on first magic byte");
        assert_eq!(parsed.opcode, 0x00);
    }

    #[test]
    fn frame_parse_rejects_garbage() {
        assert!(Frame::parse(b"").is_none());
        assert!(Frame::parse(b"\xff\xff\xff").is_none());
        // Wrong second magic word: 04 00 [05 00] — fails.
        assert!(Frame::parse(b"\x04\x00\x05\x00\x00\x00").is_none());
    }

    #[test]
    fn battery_decodes_example_from_doc() {
        // Example from LibrePods AAP Definitions:
        //   03 02 01 64 02 01 04 01 63 01 01 08 01 11 02 01
        // = L 100% discharging, R 99% charging, Case 17% discharging.
        let payload: &[u8] = &[
            0x03, 0x02, 0x01, 0x64, 0x02, 0x01, 0x04, 0x01, 0x63, 0x01, 0x01, 0x08, 0x01, 0x11,
            0x02, 0x01,
        ];
        let comps = parse_battery(payload);
        assert_eq!(comps.len(), 3);
        assert_eq!(comps[0].kind, battery_kind::RIGHT);
        assert_eq!(comps[0].level, 100);
        assert_eq!(comps[0].status, battery_status::DISCHARGING);
        assert_eq!(comps[1].kind, battery_kind::LEFT);
        assert_eq!(comps[1].level, 99);
        assert_eq!(comps[1].status, battery_status::CHARGING);
        assert_eq!(comps[2].kind, battery_kind::CASE);
        assert_eq!(comps[2].level, 17);
        assert_eq!(comps[2].status, battery_status::DISCHARGING);
    }

    #[test]
    fn battery_handles_short_payloads() {
        assert!(parse_battery(&[]).is_empty());
        // Claims 3 components but only carries one full one.
        assert_eq!(parse_battery(&[3, 2, 1, 50, 2, 1]).len(), 1);
    }

    #[test]
    fn in_ear_maps_all_three_statuses() {
        // primary=in-ear, secondary=in-case
        let in_ear = parse_in_ear(&[0x00, 0x02]).unwrap();
        assert_eq!(in_ear.primary, EarStatus::InEar);
        assert_eq!(in_ear.secondary, EarStatus::InCase);

        // primary=out, secondary=in-ear
        let in_ear = parse_in_ear(&[0x01, 0x00]).unwrap();
        assert_eq!(in_ear.primary, EarStatus::OutOfEar);
        assert_eq!(in_ear.secondary, EarStatus::InEar);

        // anything else falls back to Unknown
        let in_ear = parse_in_ear(&[0xFF, 0x00]).unwrap();
        assert_eq!(in_ear.primary, EarStatus::Unknown);
        assert_eq!(in_ear.secondary, EarStatus::InEar);

        // Too short → None
        assert!(parse_in_ear(&[0x00]).is_none());
    }

    #[test]
    fn proximity_pair_extracts_model_code() {
        // 07 19 <flags> <model_lo> <model_hi> ...
        // Model 0x2024 = AirPods Pro 2 USB-C
        let data: &[u8] = &[0x07, 0x19, 0x01, 0x24, 0x20, 0x00, 0x00, 0x00];
        assert_eq!(parse_proximity_pair(data), Some(0x2024));

        // Wrong type byte: rejected.
        let data: &[u8] = &[0x08, 0x19, 0x01, 0x24, 0x20, 0x00, 0x00, 0x00];
        assert_eq!(parse_proximity_pair(data), None);
    }

    #[test]
    fn settings_frame_decodes_id_and_payload() {
        // 04 00 04 00 09 00 0d 03 00 00 00  → noise-control = Transparency
        let bytes = b"\x04\x00\x04\x00\x09\x00\x0d\x03\x00\x00\x00";
        let frame = Frame::parse(bytes).unwrap();
        let notice = SettingNotice::parse(&frame).unwrap();
        assert_eq!(notice.id, setting::NOISE_CONTROL);
        assert_eq!(notice.data, &[0x03, 0x00, 0x00, 0x00]);
    }
}
