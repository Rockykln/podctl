//! GNOME draws no layer surfaces and lets no client float a window, so
//! the bubble goes through podctl's shell extension, which only shows
//! the frames rendered here.

use std::os::fd::{AsFd, OwnedFd};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;
use tokio::runtime::Runtime;
use zbus::Fd;

use super::wl_layer::{shm_alloc, shm_free};
use super::{Backend, Frame, Input};
use crate::render::{Hit, INPUT_RECT, hit};

pub const BUS: &str = "com.rockykln.podctl1.Shell";
const PATH: &str = "/com/rockykln/podctl1/Shell";
const IFACE: &str = "com.rockykln.podctl1.Shell";

/// Frames in flight: one on screen, one being written.
const SLOTS: usize = 2;
/// How long a frame waits for the shell to show the last one.
const FRAME_WAIT: Duration = Duration::from_millis(40);

/// Whether the extension is there to talk to.
pub async fn present() -> bool {
    let Ok(conn) = zbus::Connection::session().await else {
        return false;
    };
    let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(BUS) else {
        return false;
    };
    dbus.name_has_owner(name).await.unwrap_or(false)
}

pub struct Gnome {
    link: Option<Link>,
}

struct Link {
    rt: Runtime,
    /// Only `None` while the link is being dropped.
    proxy: Option<zbus::Proxy<'static>>,
    scale: f32,
    pool: Option<Pool>,
    shown: Receiver<u32>,
    pointer: Receiver<(String, f64, f64)>,
    busy: [bool; SLOTS],
    next: usize,
    over: Hit,
    inputs: Vec<Input>,
}

struct Pool {
    _fd: OwnedFd,
    map: *mut u8,
    slot: usize,
    w: u32,
    h: u32,
}

impl Drop for Pool {
    fn drop(&mut self) {
        shm_free(self.map, self.slot * SLOTS);
    }
}

impl Gnome {
    pub fn new() -> Self {
        Gnome { link: None }
    }
}

impl Link {
    fn call<B>(&self, method: &str, body: &B) -> Result<()>
    where
        B: serde::Serialize + zbus::DynamicType,
    {
        self.rt
            .block_on(
                self.proxy
                    .as_ref()
                    .expect("dropped last")
                    .call_method(method, body),
            )
            .with_context(|| format!("{method} on the shell extension"))?;
        Ok(())
    }

    fn collect(&mut self) {
        while let Ok(slot) = self.shown.try_recv() {
            if let Some(b) = self.busy.get_mut(slot as usize) {
                *b = false;
            }
        }
        while let Ok((kind, x, y)) = self.pointer.try_recv() {
            let new = match kind.as_str() {
                "leave" => Hit::Outside,
                _ => hit(x as f32, y as f32),
            };
            self.point_at(new);
            if kind == "click" && new != Hit::Outside {
                self.inputs.push(Input::Click);
            }
        }
    }

    fn point_at(&mut self, new: Hit) {
        let old = self.over;
        self.over = new;
        if old == new {
            return;
        }
        if (old == Hit::Outside) != (new == Hit::Outside) {
            self.inputs.push(Input::Hover(new != Hit::Outside));
        }
        if (old == Hit::Close) != (new == Hit::Close) {
            self.inputs.push(Input::OverClose(new == Hit::Close));
            let _ = self.call("SetCursor", &(new == Hit::Close,));
        }
    }
}

fn forward<T>(
    rt: &Runtime,
    proxy: &zbus::Proxy<'static>,
    signal: &'static str,
    tx: Sender<T>,
) -> Result<()>
where
    T: for<'d> serde::Deserialize<'d> + zbus::Type + Send + 'static,
{
    let mut stream = rt
        .block_on(proxy.receive_signal(signal))
        .with_context(|| format!("subscribe to {signal}"))?;
    rt.spawn(async move {
        while let Some(msg) = stream.next().await {
            if let Ok(v) = msg.body().deserialize::<T>()
                && tx.send(v).is_err()
            {
                break;
            }
        }
    });
    Ok(())
}

type Opened = (
    zbus::Proxy<'static>,
    Receiver<u32>,
    Receiver<(String, f64, f64)>,
    f64,
);

fn connect(rt: &Runtime, w: u32, h: u32) -> Result<Opened> {
    let proxy = rt.block_on(async {
        let conn = zbus::Connection::session().await?;
        zbus::Proxy::new(&conn, BUS, PATH, IFACE).await
    })?;
    let (shown_tx, shown) = channel();
    let (pointer_tx, pointer) = channel();
    // Subscribed before Open, or the first Shown could slip by.
    forward::<u32>(rt, &proxy, "Shown", shown_tx)?;
    forward::<(String, f64, f64)>(rt, &proxy, "Pointer", pointer_tx)?;
    let below = crate::anim::RISE as u32;
    let scale: f64 = rt
        .block_on(proxy.call("Open", &(w, h, below, INPUT_RECT)))
        .context("the shell extension did not open the bubble")?;
    Ok((proxy, shown, pointer, scale))
}

impl Drop for Link {
    fn drop(&mut self) {
        let _inside = self.rt.enter();
        self.proxy.take();
    }
}

impl Backend for Gnome {
    fn open(&mut self, w: u32, h: u32) -> Result<()> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .context("runtime for the shell connection")?;
        // zbus cleans up after itself on its runtime: whatever is dropped
        // here, on success or on an error, has to be dropped inside it.
        let made = {
            let _inside = rt.enter();
            connect(&rt, w, h)
        };
        let (proxy, shown, pointer, scale) = made?;
        self.link = Some(Link {
            rt,
            proxy: Some(proxy),
            scale: scale as f32,
            pool: None,
            shown,
            pointer,
            busy: [false; SLOTS],
            next: 0,
            over: Hit::Outside,
            inputs: Vec::new(),
        });
        Ok(())
    }

    fn scale(&self) -> f32 {
        self.link.as_ref().map_or(1.0, |l| l.scale)
    }

    fn push_frame(&mut self, f: &Frame) -> Result<()> {
        let link = self
            .link
            .as_mut()
            .ok_or_else(|| anyhow!("push_frame before open"))?;
        if link.pool.as_ref().is_none_or(|p| (p.w, p.h) != (f.w, f.h)) {
            let slot = f.w as usize * f.h as usize * 4;
            let (fd, map) = shm_alloc(slot * SLOTS)?;
            link.call("SetBuffer", &(Fd::from(fd.as_fd()), slot as u32))?;
            link.pool = Some(Pool {
                _fd: fd,
                map,
                slot,
                w: f.w,
                h: f.h,
            });
            link.busy = [false; SLOTS];
        }
        link.collect();
        // Both slots out: wait for the shell rather than write into a
        // frame it may still be copying.
        let deadline = Instant::now() + FRAME_WAIT;
        while link.busy.iter().all(|&b| b) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            if let Ok(slot) = link.shown.recv_timeout(left) {
                link.busy[slot as usize % SLOTS] = false;
            }
        }
        let n = (0..SLOTS)
            .map(|i| (link.next + i) % SLOTS)
            .find(|&i| !link.busy[i])
            .unwrap_or(link.next);
        let pool = link.pool.as_ref().expect("sized above");
        let len = pool.slot.min(f.bgra.len());
        unsafe {
            std::ptr::copy_nonoverlapping(f.bgra.as_ptr(), pool.map.add(n * pool.slot), len);
        }
        link.busy[n] = true;
        link.next = (n + 1) % SLOTS;
        link.call("Frame", &(n as u32, f.w, f.h, f.w * 4))
    }

    fn wait_frame(&mut self) {
        let Some(link) = self.link.as_mut() else {
            std::thread::sleep(crate::anim::FRAME);
            return;
        };
        let last = (link.next + SLOTS - 1) % SLOTS;
        let deadline = Instant::now() + FRAME_WAIT;
        while link.busy[last] {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return;
            };
            match link.shown.recv_timeout(left) {
                Ok(slot) => link.busy[slot as usize % SLOTS] = false,
                Err(_) => return,
            }
        }
    }

    fn input(&mut self) -> Vec<Input> {
        let Some(link) = self.link.as_mut() else {
            return Vec::new();
        };
        link.collect();
        std::mem::take(&mut link.inputs)
    }

    fn close(&mut self) -> Result<()> {
        if let Some(link) = self.link.take() {
            link.call("Close", &())?;
        }
        Ok(())
    }

    fn kind(&self) -> &'static str {
        "gnome"
    }
}

impl Drop for Gnome {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
