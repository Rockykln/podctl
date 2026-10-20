//! Truncated, flipped and random bytes through every parser: a panic in
//! one of them takes the daemon down. The random sequence is fixed, so a
//! failure is reproducible.

use podctl::aap::{self, Frame};
use podctl::ble;

/// The shapes a device sends. The battery payload is the documented
/// example; the other three are made up to match the layout.
const BATTERY: &[u8] = &[
    0x03, 0x02, 0x01, 0x64, 0x02, 0x01, 0x04, 0x01, 0x63, 0x01, 0x01, 0x08, 0x01, 0x11, 0x02, 0x01,
];
const SETTINGS_FRAME: &[u8] = &[0x04, 0x00, 0x04, 0x00, 0x09, 0x00, 0x0d, 0x02, 0, 0, 0];
const KEYS: &[u8] = &[
    0x04, 0x01, 0x00, 0x10, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb,
    0xcc, 0xdd, 0xee, 0xff, 0x00, 0x04, 0x00, 0x10, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
    0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
];
const ADVERT: &[u8] = &[
    0x07, 0x19, 0x01, 0x20, 0x20, 0x2b, 0x99, 0x12, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00,
];

/// Everything that takes raw bytes, so a new parser is one line away.
fn feed_all(bytes: &[u8]) {
    let _ = Frame::parse(bytes);
    let _ = aap::parse_battery(bytes);
    let _ = aap::parse_keys(bytes);
    let _ = aap::parse_ear(bytes);
    let _ = aap::parse_in_ear(bytes);
    let _ = aap::parse_press(bytes);
    let _ = aap::parse_proximity_pair(bytes);
    let _ = ble::parse_advert(bytes);
    if let Some(frame) = Frame::parse(bytes) {
        let _ = aap::SettingNotice::parse(&frame);
    }
}

/// Cut short at every possible place: that is a link dropping
/// mid-frame.
#[test]
fn a_truncated_frame_is_not_a_panic() {
    for sample in [BATTERY, SETTINGS_FRAME, KEYS, ADVERT] {
        for len in 0..=sample.len() {
            feed_all(&sample[..len]);
        }
    }
}

/// One bit at a time — this is how a length field comes to claim more
/// than there is.
#[test]
fn a_flipped_bit_is_not_a_panic() {
    for sample in [BATTERY, SETTINGS_FRAME, KEYS, ADVERT] {
        for byte in 0..sample.len() {
            for bit in 0..8 {
                let mut data = sample.to_vec();
                data[byte] ^= 1 << bit;
                feed_all(&data);
            }
        }
    }
}

/// Random bytes, in the lengths the parsers branch on.
#[test]
fn random_bytes_are_not_a_panic() {
    let mut rng = Xorshift::new(0x5eed_1234_abcd_0001);
    for _ in 0..4000 {
        let len = (rng.next() % 72) as usize;
        let data: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();
        feed_all(&data);
    }
}

/// A battery percentage no display can show, or more components than
/// the payload holds, must not come out of here.
#[test]
fn a_parsed_value_stays_inside_its_range() {
    let mut rng = Xorshift::new(0xc0ff_ee00_1234_5678);
    for _ in 0..4000 {
        let len = (rng.next() % 72) as usize;
        let data: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();

        let comps = aap::parse_battery(&data);
        let claimed = data.first().copied().unwrap_or(0) as usize;
        assert!(comps.len() <= claimed, "more components than announced");
        assert!(comps.len() * 5 <= data.len(), "{comps:?} from {len} bytes");

        if let Some(advert) = ble::parse_advert(&data) {
            for pod in [advert.left, advert.right, advert.case] {
                if let Some(level) = pod.battery {
                    assert!(level <= 100 && level % 10 == 0, "battery {level}");
                }
            }
        }
    }
}

/// A frame that parses encodes back to the same fields — not to the
/// same bytes: the handshake reply has its own magic.
#[test]
fn a_parsed_frame_encodes_back_to_the_same_fields() {
    let mut rng = Xorshift::new(0x00dd_ba11_0000_7777);
    for _ in 0..2000 {
        let len = (rng.next() % 24) as usize;
        let data: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();
        let Some(frame) = Frame::parse(&data) else {
            continue;
        };
        let again = Frame::parse(&frame.encode()).expect("its own bytes parse");
        assert_eq!(again.opcode, frame.opcode);
        assert_eq!(again.subop, frame.subop);
        assert_eq!(again.payload, frame.payload);
    }
}

/// The sequence only has to be the same on every run.
struct Xorshift(u64);

impl Xorshift {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}
