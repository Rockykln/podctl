use anyhow::{Context, Result, anyhow, bail};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape::{ConnectionExt as _, SK as ShapeKind, SO as ShapeOp};
use x11rb::protocol::xproto::{
    AtomEnum, ColormapAlloc, ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, Gcontext,
    ImageFormat, ImageOrder, Pixmap, PropMode, Rectangle, StackMode, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use tracing::debug;

use super::{Backend, Frame, Input, fingerprint};
use crate::render::{CLOSE_HIT_BOX, INPUT_RECT};

pub struct X11Or {
    win: Option<Win>,
}

struct Win {
    conn: RustConnection,
    window: Window,
    pixmap: Pixmap,
    gc: Gcontext,
    w: u16,
    h: u16,
    /// From `Xft.dpi`; X has no scaling of its own, so everything here
    /// is in device pixels.
    scale: f32,
    mon: Rect,
    mapped: bool,
    /// Servers that want the other byte order need the pixels swapped.
    swap_bytes: bool,
    /// Without a compositing manager the alpha channel is ignored, so the
    /// card's transparent surroundings would paint as a black rectangle.
    /// The window is then clipped to the opaque part instead.
    shape_to_card: bool,
    /// Fingerprint of the picture currently in the pixmap.
    painted: Option<u64>,
    /// Input-only child over the close button; it carries the hand cursor.
    close: Window,
    over_close: bool,
    inputs: Vec<Input>,
}

impl X11Or {
    pub fn new() -> Self {
        X11Or { win: None }
    }
}

impl Backend for X11Or {
    fn kind(&self) -> &'static str {
        "x11_or"
    }

    fn open(&mut self, w: u32, h: u32) -> Result<()> {
        let (conn, screen_num) = x11rb::connect(None).context("connect to X server")?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let want = super::super::config::load().output;
        let mon = pick_monitor(&conn, root, want.as_deref()).unwrap_or(Rect {
            x: 0,
            y: 0,
            w: screen.width_in_pixels as i32,
            h: screen.height_in_pixels as i32,
        });

        let (depth, visual) =
            argb_visual(screen).ok_or_else(|| anyhow!("no 32-bit ARGB visual on this screen"))?;
        let swap_bytes = conn.setup().image_byte_order == ImageOrder::MSB_FIRST;
        let scale = resources(&conn, root)
            .as_deref()
            .and_then(xft_dpi)
            .map_or(1.0, |dpi| (dpi / 96.0).clamp(1.0, 4.0));
        let (w, h) = crate::render::buf_px(w, h, scale);
        let shape_to_card = !has_compositor(&conn, screen_num);
        debug!(
            swap_bytes,
            composited = !shape_to_card,
            monitor = ?(mon.x, mon.y, mon.w, mon.h),
            scale,
            "x11 backend open"
        );

        let cmap = conn.generate_id()?;
        conn.create_colormap(ColormapAlloc::NONE, cmap, root, visual)?;

        let window = conn.generate_id()?;
        let aux = CreateWindowAux::new()
            .override_redirect(1)
            .background_pixel(0)
            .border_pixel(0)
            .colormap(cmap)
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::BUTTON_PRESS
                    | EventMask::ENTER_WINDOW
                    | EventMask::LEAVE_WINDOW,
            );
        conn.create_window(
            depth,
            window,
            root,
            0,
            0,
            w as u16,
            h as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &aux,
        )?;

        set_notification_type(&conn, window)?;
        take_input_on_card(&conn, window, scale);
        let close = close_hotspot(&conn, window, scale)?;

        let pixmap = conn.generate_id()?;
        conn.create_pixmap(depth, pixmap, window, w as u16, h as u16)?;
        let gc = conn.generate_id()?;
        conn.create_gc(gc, pixmap, &CreateGCAux::new())?;
        conn.flush()?;

        self.win = Some(Win {
            conn,
            window,
            pixmap,
            gc,
            w: w as u16,
            h: h as u16,
            scale,
            mon,
            mapped: false,
            swap_bytes,
            shape_to_card,
            painted: None,
            close,
            over_close: false,
            inputs: Vec::new(),
        });
        Ok(())
    }

    fn scale(&self) -> f32 {
        self.win.as_ref().map_or(1.0, |w| w.scale)
    }

    fn input(&mut self) -> Vec<Input> {
        let Some(win) = self.win.as_mut() else {
            return Vec::new();
        };
        pump(win);
        std::mem::take(&mut win.inputs)
    }

    fn push_frame(&mut self, f: &Frame) -> Result<()> {
        let win = self.win.as_mut().context("push_frame before open")?;
        if (f.w, f.h) != (win.w as u32, win.h as u32) {
            bail!("frame {}x{} for a {}x{} window", f.w, f.h, win.w, win.h);
        }

        // While the bubble slides only its position changes. Sending
        // the whole 260 kB image and a fresh shape with every frame
        // cost megabytes a second for a picture that stayed the same.
        let fp = fingerprint(f.bgra);
        if win.painted != Some(fp) {
            upload(win, f.bgra)?;
            if win.shape_to_card {
                clip_to_opaque(win, f.bgra);
            }
            win.painted = Some(fp);
        }

        let m = win.mon;
        let x = (m.x + ((m.w - win.w as i32) / 2).max(0)) as i16;
        let margin = (f.margin_bottom as f32 * win.scale).round() as i32;
        let y = (m.y + (m.h - win.h as i32 - margin).clamp(-(win.h as i32), m.h)) as i16;

        if !win.mapped {
            win.conn.map_window(win.window)?;
            win.mapped = true;
        }
        win.conn.configure_window(
            win.window,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .x(x as i32)
                .y(y as i32)
                .stack_mode(StackMode::ABOVE),
        )?;
        win.conn
            .copy_area(win.pixmap, win.window, win.gc, 0, 0, 0, 0, win.w, win.h)?;
        win.conn.flush()?;
        pump(win);
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        if let Some(win) = self.win.take() {
            let _ = win.conn.free_gc(win.gc);
            let _ = win.conn.free_pixmap(win.pixmap);
            let _ = win.conn.destroy_window(win.window);
            let _ = win.conn.flush();
        }
        Ok(())
    }
}

impl Drop for X11Or {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// The root window's resource database, as `xrdb -query` shows it.
fn resources(conn: &RustConnection, root: Window) -> Option<String> {
    let reply = conn
        .get_property(
            false,
            root,
            AtomEnum::RESOURCE_MANAGER,
            AtomEnum::STRING,
            0,
            u32::MAX / 4,
        )
        .ok()?
        .reply()
        .ok()?;
    String::from_utf8(reply.value).ok()
}

fn xft_dpi(db: &str) -> Option<f32> {
    db.lines()
        .find_map(|l| l.strip_prefix("Xft.dpi:"))
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|d| *d > 0.0)
}

fn argb_visual(screen: &x11rb::protocol::xproto::Screen) -> Option<(u8, u32)> {
    for d in &screen.allowed_depths {
        if d.depth != 32 {
            continue;
        }
        // A depth-32 visual of another class would not take our BGRA
        // pixels as written; 30-bit deep-colour screens ship one.
        if let Some(v) = d
            .visuals
            .iter()
            .find(|v| v.class == x11rb::protocol::xproto::VisualClass::TRUE_COLOR)
        {
            return Some((32, v.visual_id));
        }
    }
    None
}

/// A compositing manager owns the `_NET_WM_CM_S<screen>` selection. With
/// none the X server paints our RGB bytes and ignores alpha.
fn has_compositor(conn: &RustConnection, screen_num: usize) -> bool {
    let name = format!("_NET_WM_CM_S{screen_num}");
    let Ok(atom) = intern(conn, name.as_bytes()) else {
        return false;
    };
    conn.get_selection_owner(atom)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some_and(|r| r.owner != x11rb::NONE)
}

/// Clip the window to the rows' opaque runs, so an uncomposited server
/// shows the card instead of a black rectangle around it.
fn clip_to_opaque(win: &Win, bgra: &[u8]) {
    let rects = opaque_rects(bgra, win.w as usize, win.h as usize);
    let _ = win.conn.shape_rectangles(
        ShapeOp::SET,
        ShapeKind::BOUNDING,
        x11rb::protocol::xproto::ClipOrdering::YX_BANDED,
        win.window,
        0,
        0,
        &rects,
    );
}

/// X core `PutImage` is capped by the server max-request length; upload
/// the pixmap in horizontal bands so a single request never exceeds it.
fn upload(win: &Win, bgra: &[u8]) -> Result<()> {
    let stride = win.w as usize * 4;
    let max_req = win.conn.setup().maximum_request_length as usize * 4;
    let hdr = 64;
    let rows_per = ((max_req.saturating_sub(hdr)) / stride).clamp(1, win.h as usize);

    let swapped;
    let bgra = if win.swap_bytes {
        swapped = bgra
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[3], p[2], p[1], p[0]])
            .collect::<Vec<u8>>();
        &swapped[..]
    } else {
        bgra
    };

    let mut y = 0usize;
    while y < win.h as usize {
        let band = rows_per.min(win.h as usize - y);
        let start = y * stride;
        let end = start + band * stride;
        if end > bgra.len() {
            bail!("frame buffer shorter than {}x{}", win.w, win.h);
        }
        win.conn.put_image(
            ImageFormat::Z_PIXMAP,
            win.pixmap,
            win.gc,
            win.w,
            band as u16,
            0,
            y as i16,
            0,
            32,
            &bgra[start..end],
        )?;
        y += band;
    }
    Ok(())
}

/// Read what the server sent and keep the pointer activity. Moving
/// between the card and the close button's child window reports a
/// leave/enter of kind Inferior on the card, which is no leaving at all.
fn pump(win: &mut Win) {
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::NotifyDetail;
    while let Ok(Some(ev)) = win.conn.poll_for_event() {
        match ev {
            Event::EnterNotify(e) if e.event == win.close => {
                win.over_close = true;
                win.inputs.push(Input::OverClose(true));
            }
            Event::LeaveNotify(e) if e.event == win.close => {
                win.over_close = false;
                win.inputs.push(Input::OverClose(false));
            }
            Event::EnterNotify(e) if e.detail != NotifyDetail::INFERIOR => {
                win.inputs.push(Input::Hover(true));
            }
            Event::LeaveNotify(e) if e.detail != NotifyDetail::INFERIOR => {
                if win.over_close {
                    win.over_close = false;
                    win.inputs.push(Input::OverClose(false));
                }
                win.inputs.push(Input::Hover(false));
            }
            Event::ButtonPress(_) => win.inputs.push(Input::Click),
            _ => {}
        }
    }
}

fn set_notification_type(conn: &RustConnection, window: Window) -> Result<()> {
    let wt = intern(conn, b"_NET_WM_WINDOW_TYPE")?;
    // Compositors key their shadow and fade rules off this; picom ships
    // notification rules out of the box, none for utility windows.
    let kind = intern(conn, b"_NET_WM_WINDOW_TYPE_NOTIFICATION")?;
    conn.change_property32(
        PropMode::REPLACE,
        window,
        wt,
        x11rb::protocol::xproto::AtomEnum::ATOM,
        &[kind],
    )?;
    Ok(())
}

/// Only the card takes clicks; the shadow margin lets them through to
/// whatever is underneath. Without the Shape extension the whole window
/// stays clickable.
fn take_input_on_card(conn: &RustConnection, window: Window, scale: f32) {
    let (x, y, w, h) = INPUT_RECT;
    let px = |v: i32| (v as f32 * scale).round();
    let card = Rectangle {
        x: px(x) as i16,
        y: px(y) as i16,
        width: px(w) as u16,
        height: px(h) as u16,
    };
    let _ = conn.shape_rectangles(
        ShapeOp::SET,
        ShapeKind::INPUT,
        x11rb::protocol::xproto::ClipOrdering::UNSORTED,
        window,
        0,
        0,
        &[card],
    );
}

/// The X cursor font's pointing hand, `XC_hand2`, and its mask.
const XC_HAND2: u16 = 60;

/// An input-only child over the close button. The server shows its hand
/// cursor by itself, and its enter/leave tell when the pointer is on the
/// button; clicks pass up to the card.
fn close_hotspot(conn: &RustConnection, parent: Window, scale: f32) -> Result<Window> {
    let (x, y, side) = CLOSE_HIT_BOX;
    let font = conn.generate_id()?;
    conn.open_font(font, b"cursor")?;
    let cursor = conn.generate_id()?;
    conn.create_glyph_cursor(
        cursor,
        font,
        font,
        XC_HAND2,
        XC_HAND2 + 1,
        0,
        0,
        0,
        0xFFFF,
        0xFFFF,
        0xFFFF,
    )?;
    conn.close_font(font)?;
    let child = conn.generate_id()?;
    conn.create_window(
        0,
        child,
        parent,
        (x * scale).round() as i16,
        (y * scale).round() as i16,
        (side * scale).round() as u16,
        (side * scale).round() as u16,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new()
            .cursor(cursor)
            .event_mask(EventMask::ENTER_WINDOW | EventMask::LEAVE_WINDOW),
    )?;
    conn.free_cursor(cursor)?;
    conn.map_window(child)?;
    Ok(child)
}

fn intern(conn: &RustConnection, name: &[u8]) -> Result<u32> {
    Ok(conn.intern_atom(false, name)?.reply()?.atom)
}

/// One rectangle per opaque run per row, in the YX-banded order the
/// SHAPE request expects.
fn opaque_rects(bgra: &[u8], w: usize, h: usize) -> Vec<Rectangle> {
    let mut rects = Vec::new();
    for y in 0..h {
        let Some(row) = bgra.get(y * w * 4..(y + 1) * w * 4) else {
            break;
        };
        let mut x = 0usize;
        while x < w {
            if row[x * 4 + 3] < 128 {
                x += 1;
                continue;
            }
            let start = x;
            while x < w && row[x * 4 + 3] >= 128 {
                x += 1;
            }
            rects.push(Rectangle {
                x: start as i16,
                y: y as i16,
                width: (x - start) as u16,
                height: 1,
            });
        }
    }
    rects
}

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

/// Centring on the root window straddles the seam on a multi-head X
/// screen, so pick one RandR monitor: the configured output, else the
/// one under the pointer (where one is working), else the primary, else
/// the first.
fn pick_monitor(conn: &RustConnection, root: Window, want: Option<&str>) -> Option<Rect> {
    let mons = conn
        .randr_get_monitors(root, true)
        .ok()?
        .reply()
        .ok()?
        .monitors;
    let rect = |m: &x11rb::protocol::randr::MonitorInfo| Rect {
        x: m.x as i32,
        y: m.y as i32,
        w: m.width as i32,
        h: m.height as i32,
    };

    if let Some(want) = want {
        for m in &mons {
            let name = conn.get_atom_name(m.name).ok().and_then(|c| c.reply().ok());
            if name.is_some_and(|n| n.name == want.as_bytes()) {
                return Some(rect(m));
            }
        }
    }
    if let Some(p) = conn.query_pointer(root).ok().and_then(|c| c.reply().ok()) {
        let (px, py) = (p.root_x as i32, p.root_y as i32);
        if let Some(m) = mons
            .iter()
            .map(rect)
            .find(|r| px >= r.x && px < r.x + r.w && py >= r.y && py < r.y + r.h)
        {
            return Some(m);
        }
    }
    if let Some(m) = mons.iter().find(|m| m.primary) {
        return Some(rect(m));
    }
    mons.first().map(rect)
}

#[cfg(test)]
mod tests {
    use super::{opaque_rects, xft_dpi};

    #[test]
    fn reads_xft_dpi() {
        let db = "Xcursor.size:\t24\nXft.dpi:\t144\nXft.hinting:\t1\n";
        assert_eq!(xft_dpi(db), Some(144.0));
        assert_eq!(xft_dpi("Xft.dpi: 120.5"), Some(120.5));
        assert_eq!(xft_dpi("Xft.dpi:\t0"), None);
        assert_eq!(xft_dpi("Xcursor.size:\t24"), None);
    }

    fn px(alpha: u8) -> [u8; 4] {
        [0, 0, 0, alpha]
    }

    #[test]
    fn keeps_only_the_opaque_runs() {
        // 4x2: row 0 has one run of two, row 1 two runs of one.
        let mut buf = Vec::new();
        for a in [0, 255, 255, 10] {
            buf.extend(px(a));
        }
        for a in [255, 0, 0, 200] {
            buf.extend(px(a));
        }
        let r = opaque_rects(&buf, 4, 2);
        assert_eq!(r.len(), 3);
        assert_eq!((r[0].x, r[0].y, r[0].width), (1, 0, 2));
        assert_eq!((r[1].x, r[1].y, r[1].width), (0, 1, 1));
        assert_eq!((r[2].x, r[2].y, r[2].width), (3, 1, 1));
    }

    #[test]
    fn a_fully_transparent_frame_shapes_away_to_nothing() {
        let buf = vec![0u8; 4 * 4 * 2];
        assert!(opaque_rects(&buf, 4, 2).is_empty());
    }
}
