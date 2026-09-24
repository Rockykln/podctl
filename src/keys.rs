//! The proximity keys a pair of buds hands out, kept per address.
//!
//! They identify the buds behind their rotating advertisement address.
//! Nothing here leaves the machine, and the file is the user's alone.

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

pub struct Keys {
    pub irk: [u8; 16],
    pub enc: [u8; 16],
}

pub fn path(mac: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let state = std::env::var("XDG_STATE_HOME").unwrap_or_else(|_| format!("{home}/.local/state"));
    PathBuf::from(state)
        .join("podctl")
        .join(format!("keys-{}", mac.replace(':', "")))
}

pub fn load(mac: &str) -> Option<Keys> {
    let text = fs::read_to_string(path(mac)).ok()?;
    let mut irk = None;
    let mut enc = None;
    for line in text.lines() {
        let (what, hex) = line.split_once(' ')?;
        match what {
            "irk" => irk = parse_hex(hex),
            "enc" => enc = parse_hex(hex),
            _ => {}
        }
    }
    Some(Keys {
        irk: irk?,
        enc: enc?,
    })
}

pub fn save(mac: &str, keys: &Keys) -> std::io::Result<()> {
    let p = path(mac);
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&p)?;
    writeln!(f, "irk {}", hex(&keys.irk))?;
    writeln!(f, "enc {}", hex(&keys.enc))?;
    Ok(())
}

fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_hex(s: &str) -> Option<[u8; 16]> {
    let s = s.trim();
    // Length is in bytes: without the ASCII check a damaged file could
    // put a slice boundary inside a character.
    if s.len() != 32 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{Keys, parse_hex, path};

    #[test]
    fn round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("podctl-keys-test-{}", std::process::id()));
        unsafe { std::env::set_var("XDG_STATE_HOME", &dir) };
        let keys = Keys {
            irk: [1u8; 16],
            enc: [2u8; 16],
        };
        super::save("AA:BB:CC:DD:EE:FF", &keys).unwrap();
        let p = path("AA:BB:CC:DD:EE:FF");
        assert!(p.ends_with("podctl/keys-AABBCCDDEEFF"));
        let back = super::load("AA:BB:CC:DD:EE:FF").unwrap();
        assert_eq!(back.irk, keys.irk);
        assert_eq!(back.enc, keys.enc);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_half_written_files() {
        assert!(parse_hex("00112233").is_none());
        assert!(parse_hex("zz112233445566778899aabbccddeeff").is_none());
        // 32 bytes, but not 32 characters.
        assert!(parse_hex("aä11111111111111111111111111111").is_none());
        assert!(parse_hex("00112233445566778899aabbccddeeff").is_some());
    }
}
