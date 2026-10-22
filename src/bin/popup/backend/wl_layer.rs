use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::ptr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer::WlBuffer,
    wl_callback::{self, WlCallback},
    wl_compositor::WlCompositor,
    wl_output::{self, WlOutput},
    wl_pointer::{self, WlPointer},
    wl_region::WlRegion,
    wl_registry::WlRegistry,
    wl_seat::{self, WlSeat},
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1},
    wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, ZwlrLayerSurfaceV1},
};

use tracing::{debug, warn};

use super::{Backend, Frame, Input};
use crate::render::{Hit, INPUT_RECT, hit};

pub struct WlLayer {
    win: Option<Win>,
}

struct Win {
    conn: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    shm: WlShm,
    surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    /// Set when the compositor scales by fractions: buffers are drawn at
    /// device size and the viewport maps them onto the logical surface.
    viewport: Option<(WpViewport, WpFractionalScaleV1)>,
    pool: Option<Pool>,
    cur: usize,
    /// Logical size of the surface.
    w: u32,
    h: u32,
}

/// Buffers of one device size in one shared mapping.
struct Pool {
    bufs: [Buf; BUFFERS],
    map: *mut u8,
    map_len: usize,
    w: u32,
    h: u32,
    buffer_scale: i32,
}

struct Buf {
    wl: WlBuffer,
    offset: usize,
    busy: bool,
}

#[derive(Default)]
struct State {
    configured: bool,
    closed: bool,
    released: Vec<u32>,
    /// `wp_fractional_scale_v1.preferred_scale`, in 120ths.
    fractional: Option<u32>,
    /// `wl_surface.preferred_buffer_scale` (wl_compositor 6).
    integer: Option<i32>,
    outputs: Vec<(WlOutput, String)>,
    shapes: Option<WpCursorShapeManagerV1>,
    pointer: Option<(WlPointer, Option<WpCursorShapeDeviceV1>)>,
    enter_serial: u32,
    over: Option<Hit>,
    inputs: Vec<Input>,
    frame_due: bool,
}

impl State {
    /// Turn a pointer position into what changed for the card.
    fn point_at(&mut self, x: f64, y: f64) {
        let old = self.over.unwrap_or(Hit::Outside);
        let new = hit(x as f32, y as f32);
        self.over = Some(new);
        if old == new {
            return;
        }
        if (old == Hit::Outside) != (new == Hit::Outside) {
            self.inputs.push(Input::Hover(new != Hit::Outside));
        }
        if (old == Hit::Close) != (new == Hit::Close) {
            self.inputs.push(Input::OverClose(new == Hit::Close));
            if let Some((_, Some(dev))) = &self.pointer {
                let shape = if new == Hit::Close {
                    Shape::Pointer
                } else {
                    Shape::Default
                };
                dev.set_shape(self.enter_serial, shape);
            }
        }
    }
}

impl WlLayer {
    pub fn new() -> Self {
        WlLayer { win: None }
    }
}

impl Backend for WlLayer {
    fn kind(&self) -> &'static str {
        "wl_layer"
    }

    fn open(&mut self, w: u32, h: u32) -> Result<()> {
        let conn = Connection::connect_to_env().context("connect to Wayland display")?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).context("init registry")?;
        let qh = queue.handle();

        let compositor: WlCompositor = globals
            .bind(&qh, 4..=6, ())
            .map_err(|e| anyhow!("wl_compositor: {e}"))?;
        let shm: WlShm = globals
            .bind(&qh, 1..=2, ())
            .map_err(|e| anyhow!("wl_shm: {e}"))?;
        let layer_shell: ZwlrLayerShellV1 = globals
            .bind(&qh, 1..=4, ())
            .map_err(|_| anyhow!("compositor has no wlr-layer-shell"))?;

        let mut state = State {
            shapes: globals.bind(&qh, 1..=1, ()).ok(),
            ..State::default()
        };
        let _seat: Option<WlSeat> = globals.bind(&qh, 1..=7, ()).ok();
        let output = match super::super::config::load().output {
            Some(want) => find_output(&globals, &mut queue, &mut state, &want),
            None => None,
        };

        let fractional: Option<WpFractionalScaleManagerV1> = globals.bind(&qh, 1..=1, ()).ok();
        let viewporter: Option<WpViewporter> = globals.bind(&qh, 1..=1, ()).ok();

        let surface = compositor.create_surface(&qh, ());
        let viewport = match (fractional, viewporter) {
            (Some(f), Some(v)) => Some((
                v.get_viewport(&surface, &qh, ()),
                f.get_fractional_scale(&surface, &qh, ()),
            )),
            _ => None,
        };
        // Clicks on the shadow margin go to whatever is underneath.
        let region = compositor.create_region(&qh, ());
        let (rx, ry, rw, rh) = INPUT_RECT;
        region.add(rx, ry, rw, rh);
        surface.set_input_region(Some(&region));
        region.destroy();
        let layer = layer_shell.get_layer_surface(
            &surface,
            output.as_ref(),
            zwlr_layer_shell_v1::Layer::Overlay,
            "podctl-popup".into(),
            &qh,
            (),
        );
        layer.set_size(w, h);
        layer.set_anchor(Anchor::Bottom);
        // The room the entry rises from hangs below the screen edge, so
        // the card at rest sits where it always did.
        layer.set_margin(0, 0, -(crate::anim::RISE as i32), 0);
        layer.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
        surface.commit();

        // Waiting for one dispatch was waiting for *an* event: any
        // other event the compositor sent first ended the wait and the
        // surface was declared unconfigured.
        let deadline = Instant::now() + CONFIGURE_WAIT;
        while !state.configured && !state.closed {
            if Instant::now() >= deadline {
                bail!("no layer configure within {CONFIGURE_WAIT:?}");
            }
            read_into(&conn, &mut queue, &mut state, READ_SLICE)?;
        }
        if state.closed {
            bail!("compositor closed the layer surface");
        }

        // The scale comes with the output the surface lands on, which
        // the compositor may tell only after the configure.
        let deadline = Instant::now() + SCALE_WAIT;
        while state.fractional.is_none() && state.integer.is_none() && Instant::now() < deadline {
            read_into(&conn, &mut queue, &mut state, READ_SLICE)?;
        }
        debug!(
            fractional = ?state.fractional,
            integer = ?state.integer,
            viewport = viewport.is_some(),
            "wl surface scale"
        );

        self.win = Some(Win {
            conn,
            queue,
            qh,
            state,
            shm,
            surface,
            layer,
            viewport,
            pool: None,
            cur: 0,
            w,
            h,
        });
        Ok(())
    }

    fn scale(&self) -> f32 {
        let Some(win) = &self.win else { return 1.0 };
        match (&win.viewport, win.state.fractional, win.state.integer) {
            (Some(_), Some(f), _) => f as f32 / 120.0,
            (_, _, Some(n)) if n > 0 => n as f32,
            _ => 1.0,
        }
    }

    fn wait_frame(&mut self) {
        let Some(win) = self.win.as_mut() else { return };
        // A surface nobody sees gets no callback; the motion then runs
        // on at the fallback pace.
        let deadline = Instant::now() + FRAME_WAIT;
        while win.state.frame_due && Instant::now() < deadline {
            if read_into(&win.conn, &mut win.queue, &mut win.state, READ_SLICE).is_err() {
                return;
            }
        }
    }

    fn input(&mut self) -> Vec<Input> {
        let Some(win) = self.win.as_mut() else {
            return Vec::new();
        };
        let _ = read_into(&win.conn, &mut win.queue, &mut win.state, Duration::ZERO);
        std::mem::take(&mut win.state.inputs)
    }

    fn push_frame(&mut self, f: &Frame) -> Result<()> {
        let win = self.win.as_mut().context("push_frame before open")?;
        if win.state.closed {
            bail!("layer surface was closed by the compositor");
        }
        if win.pool.as_ref().is_none_or(|p| (p.w, p.h) != (f.w, f.h)) {
            resize(win, f.w, f.h)?;
        }
        let pool = win.pool.as_mut().expect("sized above");

        collect_releases(&mut win.state, pool);

        // The next free one, starting where the last frame left off.
        let slot = (0..BUFFERS)
            .map(|i| (win.cur + i) % BUFFERS)
            .find(|&i| !pool.bufs[i].busy)
            .unwrap_or(win.cur);
        // All in flight. Waiting for *an* event was not enough: any
        // event ended the wait, and the frame was then written into a
        // buffer the compositor was still reading from.
        let deadline = Instant::now() + RELEASE_WAIT;
        while pool.bufs[slot].busy {
            if Instant::now() >= deadline {
                // Rather drop this frame than paint into a buffer that
                // is not ours. The next one carries the same picture.
                return Ok(());
            }
            read_into(&win.conn, &mut win.queue, &mut win.state, READ_SLICE)?;
            collect_releases(&mut win.state, pool);
        }

        let off = pool.bufs[slot].offset;
        let len = f.w as usize * 4 * f.h as usize;
        debug_assert_eq!(f.bgra.len(), len);
        unsafe {
            ptr::copy_nonoverlapping(f.bgra.as_ptr(), pool.map.add(off), len.min(f.bgra.len()));
        }

        win.surface.set_buffer_scale(pool.buffer_scale);
        win.surface.attach(Some(&pool.bufs[slot].wl), 0, 0);
        win.surface.damage_buffer(0, 0, f.w as i32, f.h as i32);
        win.surface.frame(&win.qh, ());
        win.state.frame_due = true;
        win.surface.commit();
        pool.bufs[slot].busy = true;
        win.cur = (slot + 1) % BUFFERS;

        win.conn.flush().context("flush wayland")?;
        win.queue
            .dispatch_pending(&mut win.state)
            .context("dispatch pending")?;
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        if let Some(mut win) = self.win.take() {
            if let Some((viewport, fractional)) = win.viewport.take() {
                viewport.destroy();
                fractional.destroy();
            }
            if let Some((pointer, dev)) = win.state.pointer.take() {
                if let Some(dev) = dev {
                    dev.destroy();
                }
                if pointer.version() >= 3 {
                    pointer.release();
                }
            }
            win.layer.destroy();
            win.surface.destroy();
            if let Some(pool) = win.pool.take() {
                drop_pool(pool);
            }
            let _ = win.conn.flush();
        }
        Ok(())
    }
}

/// The output named `want` (`wl_output` 4 tells names), if there is one.
fn find_output(
    globals: &wayland_client::globals::GlobalList,
    queue: &mut EventQueue<State>,
    state: &mut State,
    want: &str,
) -> Option<WlOutput> {
    let qh = queue.handle();
    for g in globals.contents().clone_list() {
        if g.interface == WlOutput::interface().name && g.version >= 4 {
            let _: WlOutput = globals.registry().bind(g.name, 4, &qh, ());
        }
    }
    queue.roundtrip(state).ok()?;
    let found = state
        .outputs
        .iter()
        .find(|(_, name)| name == want)
        .map(|(o, _)| o.clone());
    if found.is_none() {
        let known: Vec<&str> = state.outputs.iter().map(|(_, n)| n.as_str()).collect();
        warn!(want, ?known, "no such output — the compositor picks one");
    }
    found
}

/// Buffers for frames of `w`×`h` device pixels. Ones the compositor
/// still holds stay valid on its side after they are destroyed here.
fn resize(win: &mut Win, w: u32, h: u32) -> Result<()> {
    if let Some(old) = win.pool.take() {
        drop_pool(old);
    }
    let stride = w as usize * 4;
    let one = stride * h as usize;
    let map_len = one * BUFFERS;
    let (fd, map) = shm_alloc(map_len)?;
    let pool = win.shm.create_pool(fd.as_fd(), map_len as i32, &win.qh, ());
    let mk = |off: usize| Buf {
        wl: pool.create_buffer(
            off as i32,
            w as i32,
            h as i32,
            stride as i32,
            wl_shm::Format::Argb8888,
            &win.qh,
            off as u32,
        ),
        offset: off,
        busy: false,
    };
    let bufs = std::array::from_fn(|i| mk(i * one));
    pool.destroy();
    let buffer_scale = match &win.viewport {
        Some((viewport, _)) => {
            viewport.set_destination(win.w as i32, win.h as i32);
            1
        }
        None if w.is_multiple_of(win.w) && w / win.w == h / win.h => (w / win.w) as i32,
        None => 1,
    };
    win.pool = Some(Pool {
        bufs,
        map,
        map_len,
        w,
        h,
        buffer_scale,
    });
    win.cur = 0;
    Ok(())
}

fn drop_pool(pool: Pool) {
    for b in &pool.bufs {
        b.wl.destroy();
    }
    unsafe {
        munmap(pool.map, pool.map_len);
    }
}

impl Drop for WlLayer {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// How long a frame waits for the compositor to hand a buffer back. A
/// surface nobody is reading — screen off, bubble covered — never
/// releases one, and waiting for good would also keep `Hide` and
/// `Quit` from being read.
const RELEASE_WAIT: Duration = Duration::from_millis(250);
/// Slice of that wait, so the loop keeps looking at what arrived.
const READ_SLICE: Duration = Duration::from_millis(25);
/// How long the surface may take to be configured before the bubble
/// gives up and the next backend gets a turn.
const CONFIGURE_WAIT: Duration = Duration::from_secs(2);
/// One more than double buffering, so a frame never has to wait for the
/// compositor to hand one back.
const BUFFERS: usize = 3;
/// The longest a frame waits for the compositor's go.
const FRAME_WAIT: Duration = Duration::from_millis(40);
/// How long to wait for the scale after the configure before drawing
/// at the one the bubble had last time.
const SCALE_WAIT: Duration = Duration::from_millis(100);
const POLLIN: i16 = 0x001;

#[repr(C)]
struct Pollfd {
    fd: i32,
    events: i16,
    revents: i16,
}

const MFD_CLOEXEC: u32 = 0x0001;
const PROT_READ: i32 = 0x1;
const PROT_WRITE: i32 = 0x2;
const MAP_SHARED: i32 = 0x01;

unsafe extern "C" {
    fn poll(fds: *mut Pollfd, nfds: u64, timeout: i32) -> i32;
    fn memfd_create(name: *const std::ffi::c_char, flags: u32) -> i32;
    fn ftruncate(fd: i32, length: i64) -> i32;
    fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
    fn munmap(addr: *mut u8, len: usize) -> i32;
    fn __errno_location() -> *mut i32;
}

fn last_err() -> std::io::Error {
    std::io::Error::from_raw_os_error(unsafe { *__errno_location() })
}

/// Mark every buffer the compositor handed back as free again.
fn collect_releases(state: &mut State, pool: &mut Pool) {
    for released in state.released.drain(..) {
        for b in &mut pool.bufs {
            if b.offset as u32 == released {
                b.busy = false;
            }
        }
    }
}

/// Read what is on the socket, waiting at most `wait`. Unlike
/// `blocking_dispatch` this comes back on its own if nothing arrives,
/// which is what keeps a surface nobody reads from holding the thread.
fn read_into(
    conn: &Connection,
    queue: &mut EventQueue<State>,
    state: &mut State,
    wait: Duration,
) -> Result<()> {
    conn.flush().context("flush wayland")?;
    if let Some(guard) = conn.prepare_read() {
        let mut pfd = Pollfd {
            fd: conn.as_fd().as_raw_fd(),
            events: POLLIN,
            revents: 0,
        };
        let rc = unsafe { poll(&mut pfd, 1, wait.as_millis() as i32) };
        if rc > 0 {
            guard.read().context("read wayland events")?;
        }
    }
    queue.dispatch_pending(state).context("dispatch pending")?;
    Ok(())
}

fn shm_alloc(len: usize) -> Result<(OwnedFd, *mut u8)> {
    let name = c"podctl-popup";
    let raw = unsafe { memfd_create(name.as_ptr(), MFD_CLOEXEC) };
    if raw < 0 {
        return Err(last_err()).context("memfd_create");
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    if unsafe { ftruncate(fd.as_raw_fd(), len as i64) } != 0 {
        return Err(last_err()).context("ftruncate");
    }
    let map = unsafe {
        mmap(
            ptr::null_mut(),
            len,
            PROT_READ | PROT_WRITE,
            MAP_SHARED,
            fd.as_raw_fd(),
            0,
        )
    };
    if map as isize == -1 {
        return Err(last_err()).context("mmap");
    }
    Ok((fd, map))
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        st: &mut Self,
        layer: &ZwlrLayerSurfaceV1,
        ev: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match ev {
            zwlr_layer_surface_v1::Event::Configure { serial, .. } => {
                layer.ack_configure(serial);
                st.configured = true;
            }
            zwlr_layer_surface_v1::Event::Closed => {
                st.closed = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<WlBuffer, u32> for State {
    fn event(
        st: &mut Self,
        _: &WlBuffer,
        ev: wayland_client::protocol::wl_buffer::Event,
        offset: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(ev, wayland_client::protocol::wl_buffer::Event::Release) {
            st.released.push(*offset);
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlShmPool);
delegate_noop!(State: ignore WpFractionalScaleManagerV1);
delegate_noop!(State: ignore WlRegion);

impl Dispatch<WlCallback, ()> for State {
    fn event(
        st: &mut Self,
        _: &WlCallback,
        ev: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = ev {
            st.frame_due = false;
        }
    }
}
delegate_noop!(State: ignore WpCursorShapeManagerV1);
delegate_noop!(State: ignore WpCursorShapeDeviceV1);

impl Dispatch<WlOutput, ()> for State {
    fn event(
        st: &mut Self,
        out: &WlOutput,
        ev: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = ev {
            st.outputs.push((out.clone(), name));
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        st: &mut Self,
        seat: &WlSeat,
        ev: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = ev
            && caps.contains(wl_seat::Capability::Pointer)
            && st.pointer.is_none()
        {
            let pointer = seat.get_pointer(qh, ());
            let dev = st.shapes.as_ref().map(|m| m.get_pointer(&pointer, qh, ()));
            st.pointer = Some((pointer, dev));
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        st: &mut Self,
        _: &WlPointer,
        ev: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match ev {
            wl_pointer::Event::Enter {
                serial,
                surface_x,
                surface_y,
                ..
            } => {
                st.enter_serial = serial;
                if let Some((_, Some(dev))) = &st.pointer {
                    dev.set_shape(serial, Shape::Default);
                }
                st.over = None;
                st.point_at(surface_x, surface_y);
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => st.point_at(surface_x, surface_y),
            wl_pointer::Event::Leave { .. } => st.point_at(-1.0, -1.0),
            wl_pointer::Event::Button {
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } if st.over.is_some_and(|h| h != Hit::Outside) => st.inputs.push(Input::Click),
            _ => {}
        }
    }
}
delegate_noop!(State: ignore WpViewporter);
delegate_noop!(State: ignore WpViewport);

impl Dispatch<WlSurface, ()> for State {
    fn event(
        st: &mut Self,
        _: &WlSurface,
        ev: wayland_client::protocol::wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_surface::Event::PreferredBufferScale { factor } = ev {
            st.integer = Some(factor);
        }
    }
}

impl Dispatch<WpFractionalScaleV1, ()> for State {
    fn event(
        st: &mut Self,
        _: &WpFractionalScaleV1,
        ev: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = ev {
            st.fractional = Some(scale);
        }
    }
}
delegate_noop!(State: ignore ZwlrLayerShellV1);
