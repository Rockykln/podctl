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

#[cfg(test)]
mod tests {
    use super::{ah, parse_addr, resolves};

    const IRK: [u8; 16] = [
        0xec, 0x02, 0x34, 0xa3, 0x57, 0xc8, 0xad, 0x05, 0x34, 0x10, 0x10, 0xa6, 0x0a, 0x39, 0x7d,
        0x9b,
    ];

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
