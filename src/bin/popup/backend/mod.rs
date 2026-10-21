pub mod detect;
pub mod notify;
pub mod wl_layer;
pub mod x11_or;

use std::time::Duration;

use anyhow::Result;

/// A frame the backend uploads: premultiplied BGRA of `w`×`h` device
/// pixels (the size passed to `open` times `scale`), plus the current
/// bottom gap in logical pixels (the animation drives this value each
/// tick).
pub struct Frame<'a> {
    pub bgra: &'a [u8],
    pub w: u32,
    pub h: u32,
    pub margin_bottom: i32,
}

/// Cheap hash over a frame's pixels, so a backend can tell an unchanged
/// picture from a new one.
pub fn fingerprint(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub trait Backend {
    /// How long the bubble is meant to stay up. Only the notification
    /// fallback needs it — the server, not us, retires that one, so it
    /// has to be told the same duration the hold loop uses. Backends
    /// that own their surface ignore it.
    fn set_hold(&mut self, _hold: Duration) {}
    /// What the card says, in words. The notification fallback shows
    /// text — a server that scales the card down to an icon leaves the
    /// battery levels unreadable otherwise. Backends that draw the card
    /// themselves ignore it.
    fn set_labels(&mut self, _title: &str, _body: &str) {}
    /// `w`×`h` is the logical size.
    fn open(&mut self, w: u32, h: u32) -> Result<()>;
    /// Device pixels per logical pixel where the bubble shows. It may
    /// change after `open` once the server says where that is.
    fn scale(&self) -> f32 {
        1.0
    }
    fn push_frame(&mut self, f: &Frame) -> Result<()>;
    fn close(&mut self) -> Result<()>;
    fn kind(&self) -> &'static str;
}
