//! Resolving a Bluetooth resolvable private address (RPA).
//!
//! AirPods advertise under an address that changes whenever the case is
//! opened. Only the identity resolving key (IRK) of that pair tells our
//! advertisements from the next person's.

use crate::aes;

/// The `ah` hash from the Core Specification, Vol 3 Part H 2.2.2:
/// AES-128 over the 24-bit input padded to a block, keyed with the IRK,
/// keeping the low 24 bits.
pub fn ah(irk: &[u8; 16], prand: [u8; 3]) -> [u8; 3] {
    let mut m = [0u8; 16];
    m[13..].copy_from_slice(&prand);
    let e = aes::encrypt_block(irk, &m);
    [e[13], e[14], e[15]]
}

/// Whether `addr` is a resolvable private address belonging to `irk`.
/// `addr` is most-significant byte first, as BlueZ prints it: the top
/// three bytes are `prand`, the bottom three the hash to check.
pub fn resolves(irk: &[u8; 16], addr: [u8; 6]) -> bool {
    // Bits 7..6 of the most significant byte mark an RPA (0b01).
    if addr[0] >> 6 != 0b01 {
        return false;
    }
    let prand = [addr[0], addr[1], addr[2]];
    ah(irk, prand) == [addr[3], addr[4], addr[5]]
}

/// Parse `AA:BB:CC:DD:EE:FF` into bytes, most significant first.
pub fn parse_addr(s: &str) -> Option<[u8; 6]> {
    let mut out = [0u8; 6];
    let mut parts = s.split(':');
    for b in &mut out {
        *b = u8::from_str_radix(parts.next()?, 16).ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// The plaintext part of Apple's proximity-pairing advertisement, as it
/// arrives in BlueZ manufacturer data for company 0x004C. Byte meanings
/// were read off captures from AirPods Pro 2 USB-C; the last 16 bytes are
/// encrypted and not touched here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Advert {
    pub model: u16,
    /// The buds stop advertising a moment after the lid closes, so this
    /// is mostly seen on the way there.
    pub lid_closed: bool,
    /// Wraps at 8; a change means the lid moved even if the state reads
    /// the same.
    pub lid_counter: u8,
    pub both_in_case: bool,
    pub primary_left: bool,
    pub left: Pod,
    pub right: Pod,
    pub case: Pod,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pod {
    /// Percent in steps of ten, or `None` while the bud does not report.
    pub battery: Option<u8>,
    pub charging: bool,
    pub in_ear: bool,
}

const APPLE_PROXIMITY: u8 = 0x07;

pub fn parse_advert(data: &[u8]) -> Option<Advert> {
    if data.len() < 11 || data[0] != APPLE_PROXIMITY {
        return None;
    }
    let model = u16::from_le_bytes([data[3], data[4]]);
    let status = data[5];
    let pods = data[6];
    let case_byte = data[7];
    let lid = data[8];

    let primary_left = status & 0x20 != 0;
    // Everything the buds report is ordered primary first, so the two
    // halves swap as soon as the right bud leads.
    let flipped = !primary_left;
    let level = |nibble: u8| (nibble != 15).then_some(nibble * 10);
    let (left_nib, right_nib) = if flipped {
        (pods >> 4, pods & 0x0f)
    } else {
        (pods & 0x0f, pods >> 4)
    };
    let flags = case_byte >> 4;
    let (left_charge, right_charge) = if flipped {
        (flags & 0x02 != 0, flags & 0x01 != 0)
    } else {
        (flags & 0x01 != 0, flags & 0x02 != 0)
    };
    let this_in_case = status & 0x40 != 0;
    let ear_flipped = flipped ^ this_in_case;
    let (left_ear, right_ear) = if ear_flipped {
        (status & 0x08 != 0, status & 0x02 != 0)
    } else {
        (status & 0x02 != 0, status & 0x08 != 0)
    };

    Some(Advert {
        model,
        lid_closed: lid & 0x08 != 0,
        lid_counter: lid & 0x07,
        both_in_case: status & 0x04 != 0,
        primary_left,
        left: Pod {
            battery: level(left_nib),
            charging: left_charge,
            in_ear: left_ear,
        },
        right: Pod {
            battery: level(right_nib),
            charging: right_charge,
            in_ear: right_ear,
        },
        case: Pod {
            battery: level(case_byte & 0x0f),
            charging: flags & 0x04 != 0,
            in_ear: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{ah, parse_addr, resolves};

    const IRK: [u8; 16] = [
        0xec, 0x02, 0x34, 0xa3, 0x57, 0xc8, 0xad, 0x05, 0x34, 0x10, 0x10, 0xa6, 0x0a, 0x39, 0x7d,
        0x9b,
    ];

    fn adv(hex: &str) -> super::Advert {
        let bytes: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        super::parse_advert(&bytes).unwrap()
    }

    /// Captured with both buds in the ears, case out of reach.
    #[test]
    fn reads_both_buds_in_the_ears() {
        let a = adv("07190124200b778f1100055f5740de21491ea5bcb9b3a07f8e950d");
        assert_eq!(a.model, 0x2024);
        assert!(a.left.in_ear && a.right.in_ear);
        assert_eq!((a.left.battery, a.right.battery), (Some(70), Some(70)));
        assert_eq!(a.case.battery, None);
        assert!(!a.both_in_case);
        assert!(!a.lid_closed);
    }

    /// Captured a second after both buds went into the case, lid closing.
    #[test]
    fn reads_the_case_and_the_closing_lid() {
        let a = adv("07190124207577b83a0005be5ec35a37dcf327b8478682685f4bd6");
        assert!(a.both_in_case);
        assert_eq!(a.case.battery, Some(80));
        assert!(a.left.charging && a.right.charging);
        assert!(a.lid_closed);
        assert_eq!(a.lid_counter, 2);
    }

    /// The first advertisement after the lid opened again.
    #[test]
    fn reads_the_lid_opening() {
        let a = adv("07190124201487b8580000d0d43874863e0a2b0897ff63d15ff687");
        assert!(a.lid_closed);
        assert_eq!(a.lid_counter, 0);
        assert_eq!(a.left.battery, Some(80));
    }

    #[test]
    fn ignores_other_apple_advertisements() {
        assert!(super::parse_advert(&[0x10, 0x05, 0x01]).is_none());
        assert!(super::parse_advert(&[]).is_none());
    }

    /// Core Specification Vol 3 Part H, sample data for `ah`.
    #[test]
    fn spec_sample() {
        assert_eq!(ah(&IRK, [0x70, 0x81, 0x94]), [0x0d, 0xfb, 0xaa]);
    }

    #[test]
    fn resolves_its_own_address() {
        let addr = parse_addr("70:81:94:0d:fb:aa").unwrap();
        assert!(resolves(&IRK, addr));
        let mut other = IRK;
        other[0] ^= 1;
        assert!(!resolves(&other, addr));
    }

    #[test]
    fn rejects_addresses_that_are_not_resolvable() {
        // Top bits 0b11 (static random) and 0b00 (non-resolvable).
        assert!(!resolves(&IRK, parse_addr("f0:81:94:0d:fb:aa").unwrap()));
        assert!(!resolves(&IRK, parse_addr("30:81:94:0d:fb:aa").unwrap()));
    }

    #[test]
    fn parse_addr_wants_exactly_six_bytes() {
        assert_eq!(parse_addr("50:35:6f:1b:96:53").unwrap()[0], 0x50);
        assert_eq!(parse_addr("50:35:6F:1B:96:53").unwrap()[5], 0x53);
        assert!(parse_addr("50:35:6f:1b:96").is_none());
        assert!(parse_addr("50:35:6f:1b:96:53:00").is_none());
        assert!(parse_addr("zz:35:6f:1b:96:53").is_none());
    }
}
