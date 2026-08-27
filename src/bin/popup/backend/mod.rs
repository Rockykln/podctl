pub mod detect;
pub mod notify;
pub mod wl_layer;
pub mod x11_or;

use std::time::Duration;

use anyhow::Result;

/// A frame the backend uploads: premultiplied BGRA at the size passed to
/// `open`, plus the current bottom gap in pixels (the animation drives
/// this value each tick).
pub struct Frame<'a> {
    pub bgra: &'a [u8],
    pub margin_bottom: i32,
}

pub trait Backend {
    /// How long the bubble is meant to stay up. Only the notification
    /// fallback needs it — the server, not us, retires that one, so it
    /// has to be told the same duration the hold loop uses. Backends
    /// that own their surface ignore it.
    fn set_hold(&mut self, _hold: Duration) {}
    fn open(&mut self, w: u32, h: u32) -> Result<()>;
    fn push_frame(&mut self, f: &Frame) -> Result<()>;
    fn close(&mut self) -> Result<()>;
    fn kind(&self) -> &'static str;
}
