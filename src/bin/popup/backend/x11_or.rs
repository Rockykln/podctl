use anyhow::{Context, Result, anyhow, bail};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    ColormapAlloc, ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, Gcontext, ImageFormat,
    Pixmap, PropMode, StackMode, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::{Backend, Frame};

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
    mon: Rect,
    mapped: bool,
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

        let cmap = conn.generate_id()?;
        conn.create_colormap(ColormapAlloc::NONE, cmap, root, visual)?;

        let window = conn.generate_id()?;
        let aux = CreateWindowAux::new()
            .override_redirect(1)
            .background_pixel(0)
            .border_pixel(0)
            .colormap(cmap)
            .event_mask(EventMask::EXPOSURE);
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

        set_utility_type(&conn, window)?;

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
            mon,
            mapped: false,
        });
        Ok(())
    }

    fn push_frame(&mut self, f: &Frame) -> Result<()> {
        let win = self.win.as_mut().context("push_frame before open")?;

        upload(win, f.bgra)?;

        let m = win.mon;
        let x = (m.x + ((m.w - win.w as i32) / 2).max(0)) as i16;
        let y = (m.y + (m.h - win.h as i32 - f.margin_bottom).clamp(-(win.h as i32), m.h)) as i16;

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
        drain(win);
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

fn argb_visual(screen: &x11rb::protocol::xproto::Screen) -> Option<(u8, u32)> {
    for d in &screen.allowed_depths {
        if d.depth == 32
            && let Some(v) = d.visuals.first()
        {
            return Some((32, v.visual_id));
        }
    }
    None
}

/// X core `PutImage` is capped by the server max-request length; upload
/// the pixmap in horizontal bands so a single request never exceeds it.
fn upload(win: &Win, bgra: &[u8]) -> Result<()> {
    let stride = win.w as usize * 4;
    let max_req = win.conn.setup().maximum_request_length as usize * 4;
    let hdr = 64;
    let rows_per = ((max_req.saturating_sub(hdr)) / stride).clamp(1, win.h as usize);

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

fn drain(win: &Win) {
    while let Ok(Some(_)) = win.conn.poll_for_event() {}
}

fn set_utility_type(conn: &RustConnection, window: Window) -> Result<()> {
    let wt = intern(conn, b"_NET_WM_WINDOW_TYPE")?;
    let util = intern(conn, b"_NET_WM_WINDOW_TYPE_UTILITY")?;
    conn.change_property32(
        PropMode::REPLACE,
        window,
        wt,
        x11rb::protocol::xproto::AtomEnum::ATOM,
        &[util],
    )?;
    Ok(())
}

fn intern(conn: &RustConnection, name: &[u8]) -> Result<u32> {
    Ok(conn.intern_atom(false, name)?.reply()?.atom)
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
/// primary, else the one under the pointer, else the first.
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
    if let Some(m) = mons.iter().find(|m| m.primary) {
        return Some(rect(m));
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
    mons.first().map(rect)
}
