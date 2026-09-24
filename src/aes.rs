//! AES-128 on a single block, as specified in FIPS-197.
//!
//! Used for the Bluetooth `ah` hash that resolves an AirPods random
//! address, and for the encrypted tail of the proximity advertisement.
//! Both are 16-byte one-shot operations, so there is no mode of
//! operation here and no attempt at constant-time execution.

const ROUNDS: usize = 10;
const SBOX: [u8; 256] = build_sbox();
const INV_SBOX: [u8; 256] = build_inv_sbox();

/// Encrypt one 16-byte block with a 16-byte key.
pub fn encrypt_block(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    let rk = expand_key(key);
    let mut s = *block;
    add_round_key(&mut s, &rk[0]);
    for round in &rk[1..ROUNDS] {
        sub_bytes(&mut s);
        shift_rows(&mut s);
        mix_columns(&mut s);
        add_round_key(&mut s, round);
    }
    sub_bytes(&mut s);
    shift_rows(&mut s);
    add_round_key(&mut s, &rk[ROUNDS]);
    s
}

/// Decrypt one 16-byte block with a 16-byte key.
pub fn decrypt_block(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    let rk = expand_key(key);
    let mut s = *block;
    add_round_key(&mut s, &rk[ROUNDS]);
    for round in rk[1..ROUNDS].iter().rev() {
        inv_shift_rows(&mut s);
        inv_sub_bytes(&mut s);
        add_round_key(&mut s, round);
        inv_mix_columns(&mut s);
    }
    inv_shift_rows(&mut s);
    inv_sub_bytes(&mut s);
    add_round_key(&mut s, &rk[0]);
    s
}

/// The 11 round keys: the key itself plus one per round.
fn expand_key(key: &[u8; 16]) -> [[u8; 16]; ROUNDS + 1] {
    let mut w = [[0u8; 4]; 4 * (ROUNDS + 1)];
    for (i, word) in w.iter_mut().take(4).enumerate() {
        word.copy_from_slice(&key[4 * i..4 * i + 4]);
    }
    let mut rcon = 1u8;
    for i in 4..w.len() {
        let mut t = w[i - 1];
        if i % 4 == 0 {
            t.rotate_left(1);
            for b in &mut t {
                *b = SBOX[*b as usize];
            }
            t[0] ^= rcon;
            rcon = xtime(rcon);
        }
        for (b, prev) in t.iter_mut().zip(w[i - 4]) {
            *b ^= prev;
        }
        w[i] = t;
    }
    let mut out = [[0u8; 16]; ROUNDS + 1];
    for (r, round) in out.iter_mut().enumerate() {
        for (i, word) in w[4 * r..4 * r + 4].iter().enumerate() {
            round[4 * i..4 * i + 4].copy_from_slice(word);
        }
    }
    out
}

fn add_round_key(s: &mut [u8; 16], rk: &[u8; 16]) {
    for (b, k) in s.iter_mut().zip(rk) {
        *b ^= k;
    }
}

fn sub_bytes(s: &mut [u8; 16]) {
    for b in s.iter_mut() {
        *b = SBOX[*b as usize];
    }
}

fn inv_sub_bytes(s: &mut [u8; 16]) {
    for b in s.iter_mut() {
        *b = INV_SBOX[*b as usize];
    }
}

/// The state is column-major: byte `4*c + r` is row r, column c. Row r
/// rotates left by r.
fn shift_rows(s: &mut [u8; 16]) {
    let t = *s;
    for r in 1..4 {
        for c in 0..4 {
            s[4 * c + r] = t[4 * ((c + r) % 4) + r];
        }
    }
}

fn inv_shift_rows(s: &mut [u8; 16]) {
    let t = *s;
    for r in 1..4 {
        for c in 0..4 {
            s[4 * ((c + r) % 4) + r] = t[4 * c + r];
        }
    }
}

fn mix_columns(s: &mut [u8; 16]) {
    for c in s.as_chunks_mut::<4>().0 {
        let (a0, a1, a2, a3) = (c[0], c[1], c[2], c[3]);
        c[0] = mul(a0, 2) ^ mul(a1, 3) ^ a2 ^ a3;
        c[1] = a0 ^ mul(a1, 2) ^ mul(a2, 3) ^ a3;
        c[2] = a0 ^ a1 ^ mul(a2, 2) ^ mul(a3, 3);
        c[3] = mul(a0, 3) ^ a1 ^ a2 ^ mul(a3, 2);
    }
}

fn inv_mix_columns(s: &mut [u8; 16]) {
    for c in s.as_chunks_mut::<4>().0 {
        let (a0, a1, a2, a3) = (c[0], c[1], c[2], c[3]);
        c[0] = mul(a0, 14) ^ mul(a1, 11) ^ mul(a2, 13) ^ mul(a3, 9);
        c[1] = mul(a0, 9) ^ mul(a1, 14) ^ mul(a2, 11) ^ mul(a3, 13);
        c[2] = mul(a0, 13) ^ mul(a1, 9) ^ mul(a2, 14) ^ mul(a3, 11);
        c[3] = mul(a0, 11) ^ mul(a1, 13) ^ mul(a2, 9) ^ mul(a3, 14);
    }
}

/// Multiply by x in GF(2^8), reducing modulo the AES polynomial 0x11b.
const fn xtime(b: u8) -> u8 {
    (b << 1) ^ if b & 0x80 != 0 { 0x1b } else { 0 }
}

/// Russian-peasant multiplication in GF(2^8).
const fn mul(mut a: u8, mut b: u8) -> u8 {
    let mut out = 0u8;
    while b != 0 {
        if b & 1 != 0 {
            out ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    out
}

/// S-box: multiplicative inverse in GF(2^8) followed by the affine map
/// from FIPS-197 §5.1.1. Built at compile time instead of pasted as a
/// 256-byte table, so the definition is visible.
const fn build_sbox() -> [u8; 256] {
    let mut sbox = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        let inv = gf_inverse(i as u8);
        sbox[i] = inv ^ rotl(inv, 1) ^ rotl(inv, 2) ^ rotl(inv, 3) ^ rotl(inv, 4) ^ 0x63;
        i += 1;
    }
    sbox
}

const fn build_inv_sbox() -> [u8; 256] {
    let sbox = build_sbox();
    let mut inv = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        inv[sbox[i] as usize] = i as u8;
        i += 1;
    }
    inv
}

const fn rotl(b: u8, n: u32) -> u8 {
    b.rotate_left(n)
}

/// Inverse in GF(2^8), with 0 mapped to 0. Brute force: 255 multiplies
/// once at compile time beats carrying log tables around.
const fn gf_inverse(a: u8) -> u8 {
    if a == 0 {
        return 0;
    }
    let mut i = 1u16;
    while i < 256 {
        if mul(a, i as u8) == 1 {
            return i as u8;
        }
        i += 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::{decrypt_block, encrypt_block};

    fn hex(s: &str) -> [u8; 16] {
        let mut out = [0u8; 16];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex");
        }
        out
    }

    /// FIPS-197 appendix B.
    #[test]
    fn fips197_worked_example() {
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let pt = hex("3243f6a8885a308d313198a2e0370734");
        assert_eq!(
            encrypt_block(&key, &pt),
            hex("3925841d02dc09fbdc118597196a0b32")
        );
    }

    /// FIPS-197 appendix C.1 (AES-128).
    #[test]
    fn fips197_c1_round_trips() {
        let key = hex("000102030405060708090a0b0c0d0e0f");
        let pt = hex("00112233445566778899aabbccddeeff");
        let ct = hex("69c4e0d86a7b0430d8cdb78070b4c55a");
        assert_eq!(encrypt_block(&key, &pt), ct);
        assert_eq!(decrypt_block(&key, &ct), pt);
    }

    #[test]
    fn all_zero_key_and_block() {
        let z = [0u8; 16];
        assert_eq!(
            encrypt_block(&z, &z),
            hex("66e94bd4ef8a2c3b884cfa59ca342b2e")
        );
        assert_eq!(decrypt_block(&z, &encrypt_block(&z, &z)), z);
    }

    #[test]
    fn sbox_is_a_permutation() {
        let mut seen = [false; 256];
        for b in super::SBOX {
            assert!(!seen[b as usize], "duplicate 0x{b:02x}");
            seen[b as usize] = true;
        }
        for (i, b) in super::SBOX.iter().enumerate() {
            assert_eq!(super::INV_SBOX[*b as usize] as usize, i);
        }
    }
}
