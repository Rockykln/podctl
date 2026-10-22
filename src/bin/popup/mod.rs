mod anim;
mod art;
mod assets;
mod backend;
mod config;
mod layers;
mod paint;
mod render;
mod scheme;

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use tokio::signal::unix::{SignalKind, signal};
use tokio::time::sleep_until;

use tracing::{debug, info, warn};

use podctl::exitcode;
use podctl::model::{Battery, DeviceState, Event};
use podctl::{Request, Response, WatchRole};

use anim::{Pose, RISE};
use art::Art;
use backend::{Backend, Frame, Input};
use config::{MotionPick, Pick, ThemePick};
use layers::Layers;
use render::{BUF_H, BUF_W, Pod, Snapshot, Theme};

const DEMO_HOLD_MS: u64 = 5_000;
const DEBOUNCE_MS: u64 = 500;
const AUTO_MODE_WINDOW: Duration = Duration::from_secs(3);
const BACKOFF_INITIAL_MS: u64 = 500;
const BACKOFF_MAX_MS: u64 = 30_000;
/// After the pointer leaves the card the bubble stays at least this long,
/// so it does not vanish the moment one looks away.
const AFTER_HOVER: Duration = Duration::from_millis(1500);
/// However long the pointer rests on it, the bubble goes after this:
/// a hover whose end never arrived must not keep it up for good.
const MAX_HOVER: Duration = Duration::from_secs(30);
/// How often the hold loop looks at pointer input.
const INPUT_SLICE: Duration = Duration::from_millis(30);
/// The entry waits at most this long for its layers, else it shows the
/// card standing still.
const LAYER_WAIT: Duration = Duration::from_millis(200);

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    podctl::logging::init("info");
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        return ExitCode::SUCCESS;
    }

    let dumping = flag(&args, "--dump").is_some();
    let pick = match flag(&args, "--theme") {
        Some(v) => match ThemePick::parse(v) {
            Some(p) => p,
            None => {
                return fail(
                    &format!("theme '{v}' (auto, dark or light)"),
                    exitcode::USAGE,
                );
            }
        },
        // A dump stays the same on every desktop unless asked otherwise.
        None if dumping => ThemePick::Dark,
        None => ThemePick::Auto,
    };
    let theme = Theme::of(is_dark(pick).await);

    if let Some(path) = flag(&args, "--dump") {
        return match dump(path, &theme, &args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&format!("dump: {e}"), exitcode::OSERR),
        };
    }

    if args.iter().any(|a| a == "--demo") {
        let want = flag(&args, "--backend").map(str::to_string);
        let hold = flag(&args, "--hold")
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(Duration::from_millis(DEMO_HOLD_MS), Duration::from_millis);
        // The notification backend drives zbus on a runtime of its own,
        // which panics if this thread is already running one.
        let calm = match flag(&args, "--motion").map(MotionPick::parse) {
            Some(None) => return fail("motion (auto, full or reduced)", exitcode::USAGE),
            Some(Some(m)) => is_calm(m).await,
            None => is_calm(MotionPick::Auto).await,
        };
        let updates = args.iter().any(|a| a == "--updates");
        let out = thread::spawn(move || demo(&theme, calm, updates, want, hold)).join();
        return match out {
            Ok(Ok(())) => ExitCode::SUCCESS,
            Ok(Err(e)) => fail(&format!("demo: {e:#}"), exitcode::UNAVAILABLE),
            Err(_) => fail("demo: render thread panicked", exitcode::SOFTWARE),
        };
    }

    run().await
}

async fn run() -> ExitCode {
    let cfg = config::load();
    if !cfg.enabled {
        info!("popup disabled by config — exiting");
        return ExitCode::SUCCESS;
    }

    // One popup process at a time, or a service + a stray manual run
    // would each pop their own window. The flock releases on exit.
    let _lock = match single_instance() {
        Some(f) => f,
        None => {
            info!("another instance already holds the lock — exiting");
            return ExitCode::SUCCESS;
        }
    };

    let pick = BackendPick {
        name: match cfg.backend {
            // The extension first: on GNOME the next best is XWayland,
            // which cannot keep the bubble above everything.
            Pick::Auto if backend::gnome::present().await => "gnome",
            Pick::Auto => backend::detect::detect(),
            other => other.as_str(),
        }
        .to_string(),
        explicit: cfg.backend != Pick::Auto,
    };
    let dark = Arc::new(AtomicBool::new(is_dark(cfg.theme).await));
    let calm = Arc::new(AtomicBool::new(is_calm(cfg.motion).await));
    if cfg.theme == ThemePick::Auto || cfg.motion == MotionPick::Auto {
        tokio::spawn(scheme::follow(
            (cfg.theme == ThemePick::Auto).then(|| dark.clone()),
            (cfg.motion == MotionPick::Auto).then(|| calm.clone()),
        ));
    }
    let hold = Duration::from_millis(cfg.duration_ms);
    let visible = Duration::from_millis(config::visible_ms(&cfg));

    info!(
        backend = %pick.name,
        theme = cfg.theme.as_str(),
        dark = dark.load(Ordering::Relaxed),
        motion = cfg.motion.as_str(),
        calm = calm.load(Ordering::Relaxed),
        visible_ms = visible.as_millis() as u64,
        "podctl-popup ready"
    );

    // Left running across an upgrade, the bubble reads a state it may
    // not understand. The tray shows this as a notification; here it
    // goes in the log, which is where 'podctl debug' points.
    if let Ok(Some(note)) = tokio::task::spawn_blocking(podctl::client::version_mismatch).await {
        warn!("{note}");
    }

    let (tx, rx) = mpsc::channel::<Cmd>();
    let render = thread::spawn(move || render_loop(rx, &pick, &dark, &calm, hold));

    tokio::select! {
        _ = watch(&tx, visible) => {}
        _ = await_term() => {}
    }

    let _ = tx.send(Cmd::Quit);
    let _ = render.join();
    ExitCode::SUCCESS
}

enum Cmd {
    Show(Snapshot),
    Refresh(Snapshot),
    Hide,
    Quit,
}

/// How a single event may present itself. `allow` is false while the
/// session is locked: a bubble the screen lock covers is noise, and one
/// that ends up above the lock is worse.
#[derive(Clone, Copy)]
struct ShowOpts {
    visible: Duration,
    allow: bool,
    /// Connected before this event's fresh Status was applied.
    was_connected: bool,
}

/// Session lock state, asked at most once a second. A bubble triggers a
/// handful of events, and logind answers over D-Bus.
#[derive(Default)]
struct LockState {
    last: Option<(Instant, bool)>,
}

impl LockState {
    async fn get(&mut self) -> bool {
        if let Some((at, v)) = self.last
            && at.elapsed() < Duration::from_secs(1)
        {
            return v;
        }
        let v = session_locked().await;
        self.last = Some((Instant::now(), v));
        v
    }
}

/// logind's LockedHint is the reliable answer where the desktop sets it
/// (KDE, GNOME); the ScreenSaver service covers the rest.
async fn session_locked() -> bool {
    if let Ok(conn) = zbus::Connection::system().await
        && let Ok(p) = zbus::Proxy::new(
            &conn,
            "org.freedesktop.login1",
            "/org/freedesktop/login1/session/auto",
            "org.freedesktop.login1.Session",
        )
        .await
        && let Ok(true) = p.get_property::<bool>("LockedHint").await
    {
        return true;
    }
    let Ok(conn) = zbus::Connection::session().await else {
        return false;
    };
    let Ok(p) = zbus::Proxy::new(
        &conn,
        "org.freedesktop.ScreenSaver",
        "/org/freedesktop/ScreenSaver",
        "org.freedesktop.ScreenSaver",
    )
    .await
    else {
        return false;
    };
    p.call_method("GetActive", &())
        .await
        .ok()
        .and_then(|m| m.body().deserialize::<bool>().ok())
        .unwrap_or(false)
}

/// Backend names as the config and `--backend` spell them.
fn short(kind: &str) -> &str {
    match kind {
        "wl_layer" => "wl",
        "x11_or" => "x11",
        k => k,
    }
}

/// Which backend to open, and whether the user named it. Kept together
/// because the second answer decides whether a failure may fall back.
struct BackendPick {
    name: String,
    explicit: bool,
}

async fn is_calm(pick: MotionPick) -> bool {
    match pick {
        MotionPick::Full => false,
        MotionPick::Reduced => true,
        MotionPick::Auto => scheme::reduced_motion().await,
    }
}

/// Without a portal, `auto` keeps the bubble dark as it always was.
async fn is_dark(pick: ThemePick) -> bool {
    match pick {
        ThemePick::Dark => true,
        ThemePick::Light => false,
        ThemePick::Auto => scheme::current().await.unwrap_or(true),
    }
}

fn render_loop(
    rx: Receiver<Cmd>,
    pick: &BackendPick,
    dark: &AtomicBool,
    calm: &AtomicBool,
    hold: Duration,
) {
    // Drawn ahead at the last bubble's scale; usually the next one shows
    // on the same screen and nothing has to be drawn twice.
    let mut scale = 1.0;
    while let Ok(cmd) = rx.recv() {
        let snap = match cmd {
            Cmd::Show(s) => s,
            // A refresh with nothing on screen is stale state — ignore
            // it instead of popping an unrequested bubble.
            Cmd::Refresh(_) | Cmd::Hide => continue,
            Cmd::Quit => return,
        };
        // A change of desktop scheme shows from the next bubble on.
        let theme = Theme::of(dark.load(Ordering::Relaxed));
        if matches!(
            show_cycle(
                &rx,
                pick,
                &theme,
                calm.load(Ordering::Relaxed),
                hold,
                snap,
                &mut scale
            ),
            Outcome::Quit
        ) {
            return;
        }
    }
}

#[derive(Clone, Copy)]
enum Outcome {
    Done,
    Quit,
}

enum Drained {
    Continue,
    ResetHold,
    Stop(Outcome),
}

/// What one command does: redraw (`Show` also restarts the hold timer),
/// or end the cycle.
enum Applied {
    Redrawn { reset: bool },
    Stop(Outcome),
}

/// The card drawn for one scale, and the frame it is placed in: the
/// frame keeps the room below the card the entry rises from.
struct Card {
    snap: Snapshot,
    theme: Theme,
    scale: f32,
    drawn: Option<render::Drawn>,
    /// The close button: lit (towards `lit_to`, from `lit_from` at
    /// `lit_at`) and pressed.
    lit_to: bool,
    lit_from: f32,
    lit_at: Instant,
    pressed: f32,
    bgra: Vec<u8>,
    w: u32,
    h: u32,
    frame: Vec<u8>,
    frame_h: u32,
    /// The parts the entry moves, built on a thread of their own; until
    /// they are there the entry shows the card standing still.
    layers: Option<Layers>,
    pending: Option<Receiver<Layers>>,
    /// When the readouts' values came in, and what the status line said
    /// before it last changed: both play out over the next frames.
    values_at: Option<Instant>,
    status_from: Option<(Instant, &'static str)>,
    /// Less motion asked for: new values just appear.
    calm: bool,
    /// A change while the card is up: when, and the card before it.
    update: Option<(Instant, Snapshot)>,
    charge_at: Option<Instant>,
    /// `bgra` shows an older snapshot. Drawing the card whole takes a
    /// good part of a second at 2×; while the layers make the frames it
    /// waits until they are done.
    stale: bool,
}

/// One readout through a change from `old` to `new`, `ms` in.
fn changing(old: Pod, new: Pod, ms: f32) -> anim::Gauge {
    let mut g = anim::Gauge::REST;
    if let (Some(o), Some(n)) = (old.level, new.level)
        && o != n
    {
        let shown = o as f32 + (n as f32 - o as f32) * anim::tween(ms);
        g.fill = if n == 0 { 1.0 } else { shown / n as f32 };
        g.text = anim::digits(ms);
        g.prev = Some(o);
    }
    if new.charging && !old.charging {
        g.green = anim::green(ms);
        g.bolt = anim::bolt(ms);
    }
    g
}

fn has_values(s: &Snapshot) -> bool {
    [s.left, s.right, s.case].iter().any(|p| p.level.is_some())
}

/// What the layers are drawn from; the values and the status line are
/// drawn on top per frame.
fn layer_key(s: &Snapshot) -> (String, Art) {
    (s.name.clone(), s.art)
}

impl Card {
    fn new(snap: Snapshot, theme: &Theme, scale: f32) -> Self {
        let mut c = Card {
            snap,
            theme: *theme,
            scale,
            drawn: None,
            lit_to: false,
            lit_from: 0.0,
            lit_at: Instant::now(),
            pressed: 0.0,
            bgra: Vec::new(),
            w: 0,
            h: 0,
            frame: Vec::new(),
            frame_h: 0,
            layers: None,
            pending: None,
            values_at: None,
            status_from: None,
            calm: false,
            update: None,
            charge_at: None,
            stale: false,
        };
        c.draw();
        c
    }

    /// The card maps at `t0`: values it already has fill in from there.
    fn mapped(&mut self, t0: Instant) {
        if has_values(&self.snap) {
            self.values_at = Some(t0 + Duration::from_secs_f32(anim::RINGS_AFTER_MAP_MS / 1000.0));
        }
    }

    fn lit(&self, now: Instant) -> f32 {
        let to = if self.lit_to { 1.0 } else { 0.0 };
        let u = now.saturating_duration_since(self.lit_at).as_secs_f32() * 1000.0 / anim::KNOB_MS;
        self.lit_from + (to - self.lit_from) * anim::ease_out_cubic(u)
    }

    fn knob(&self, now: Instant) -> render::Knob {
        render::Knob {
            lit: self.lit(now),
            pressed: self.pressed,
        }
    }

    /// Whether the values, the status line or the close button are
    /// still on their way somewhere.
    fn settling(&self, now: Instant) -> bool {
        let knob =
            now.saturating_duration_since(self.lit_at).as_secs_f32() * 1000.0 < anim::KNOB_MS;
        let n = render::readout_count(&self.snap);
        let since = |t: Instant| now.saturating_duration_since(t).as_secs_f32() * 1000.0;
        let changing = self
            .update
            .as_ref()
            .is_some_and(|(t, _)| since(*t) < anim::UPDATE_MS);
        // Before its start the timeline counts as running too.
        knob || changing
            || self
                .values_at
                .is_some_and(|t| t > now || anim::gauges_moving(since(t), n))
            || self
                .status_from
                .is_some_and(|(t, _)| anim::status_swap(since(t)).is_some())
    }

    /// The readouts `ms` into a change from `old`. Where L and R come
    /// apart or together the two layouts cross over; the case readout
    /// stays where it is.
    fn draw_change(&self, pm: &mut tiny_skia::Pixmap, old: &Snapshot, ms: f32) {
        let (pal, s) = (&self.theme, self.scale);
        let (was, now) = (render::readout_pods(old), render::readout_pods(&self.snap));
        if was.len() == now.len() {
            render::draw_readouts(pm, &self.snap, pal, s, &|i, _| changing(was[i], now[i], ms));
            return;
        }
        let (out, inn, slid) = anim::split(ms);
        let case_old = was.len() - 1;
        let case_new = now.len() - 1;
        let splitting = now.len() > was.len();
        render::draw_readouts(pm, old, pal, s, &|i, _| anim::Gauge {
            alpha: if i == case_old { 0.0 } else { out },
            inset: if splitting {
                0.0
            } else {
                anim::SPLIT_INSET * (1.0 - out)
            },
            ..anim::Gauge::REST
        });
        render::draw_readouts(pm, &self.snap, pal, s, &|i, _| {
            if i == case_new {
                return changing(was[case_old], now[case_new], ms);
            }
            anim::Gauge {
                alpha: inn,
                inset: if splitting {
                    anim::SPLIT_INSET * (1.0 - slid)
                } else {
                    0.0
                },
                ..anim::Gauge::REST
            }
        });
    }

    /// The status line and the readouts as they are at `now`.
    fn overlay(&self, now: Instant) -> impl Fn(&mut tiny_skia::Pixmap) + '_ {
        let ms = move |t: Instant| {
            if t > now {
                -(t - now).as_secs_f32() * 1000.0
            } else {
                (now - t).as_secs_f32() * 1000.0
            }
        };
        let values = self.values_at.map(ms);
        let status = self
            .status_from
            .and_then(|(t, old)| anim::status_swap(ms(t)).map(|sw| (old, sw)));
        move |pm: &mut tiny_skia::Pixmap| {
            let (pal, s) = (&self.theme, self.scale);
            match status {
                Some((old, (a, ady, b, bdy))) => {
                    render::draw_status(pm, old, pal, s, a, ady);
                    render::draw_status(pm, self.snap.status(), pal, s, b, bdy);
                }
                None => render::draw_status(pm, self.snap.status(), pal, s, 1.0, 0.0),
            }
            if let Some((t, old)) = &self.update
                && ms(*t) < anim::UPDATE_MS
            {
                self.draw_change(pm, old, ms(*t));
                return;
            }
            render::draw_readouts(pm, &self.snap, pal, s, &|i, low| match values {
                Some(ms) => anim::gauge(ms, i, low),
                None => anim::Gauge::REST,
            });
        }
    }

    fn start_layers(&mut self) {
        let (snap, theme, scale) = (self.snap.clone(), self.theme, self.scale);
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let t0 = Instant::now();
            let l = Layers::build(&snap, &theme, scale);
            debug!(ms = ms_since(t0), scale, "layers built");
            let _ = tx.send(l);
        });
        self.layers = None;
        self.pending = Some(rx);
    }

    /// Block until the layers are there or `wait` is over.
    fn wait_layers(&mut self, wait: Duration) {
        if let Some(rx) = &self.pending
            && let Ok(l) = rx.recv_timeout(wait)
        {
            self.layers = Some(l);
            self.pending = None;
        }
    }

    fn layers(&mut self) -> Option<&Layers> {
        if let Some(rx) = &self.pending
            && let Ok(l) = rx.try_recv()
        {
            self.layers = Some(l);
            self.pending = None;
        }
        self.layers.as_ref()
    }

    /// Place the card in the frame as `pose` has it, with the products
    /// as `art` has them while the layers are there. Offset and opacity
    /// are row copies and a multiply; only the shrink of a dismissal
    /// needs a real transform.
    fn compose(&mut self, pose: Pose, art: Option<anim::Art>) {
        let now = Instant::now();
        let art = art.or_else(|| self.settling(now).then_some(anim::Art::REST));
        let _ = self.layers();
        let knob = self.knob(now);
        // The layers hold the case light either way; it shows only on a
        // charging case, coming on where the charge began on screen.
        let art = art.map(|mut a| {
            a.led = match self.charge_at {
                _ if !self.snap.case.charging => 0.0,
                Some(t) => a.led.min(anim::ease_out_cubic(
                    now.saturating_duration_since(t).as_secs_f32() * 1000.0 / anim::LED_MS,
                )),
                None => a.led,
            };
            a
        });
        if self.stale && (art.is_none() || self.layers.is_none()) {
            self.draw();
        }
        let mut moving = match (art, &self.layers) {
            (Some(a), Some(l)) => Some(render::to_bgra_premul(&l.compose(a, &self.overlay(now)))),
            _ => None,
        };
        if knob != render::Knob::REST {
            let mut bgra = moving.take().unwrap_or_else(|| self.bgra.clone());
            self.patch_close(&mut bgra, knob);
            moving = Some(bgra);
        }
        let src = moving.as_deref().unwrap_or(&self.bgra);
        let (w, h) = (self.w as usize, self.h as usize);
        self.frame_h = render::buf_px(BUF_W, BUF_H + RISE as u32, self.scale).1;
        let fh = self.frame_h as usize;
        self.frame.clear();
        self.frame.resize(w * fh * 4, 0);
        if pose.scale != 1.0 {
            let (Some(src), Some(mut dst)) = (
                tiny_skia::PixmapRef::from_bytes(src, self.w, self.h),
                tiny_skia::PixmapMut::from_bytes(&mut self.frame, self.w, self.frame_h),
            ) else {
                return;
            };
            let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
            let t = tiny_skia::Transform::from_translate(cx, cy + pose.dy * self.scale)
                .pre_scale(pose.scale, pose.scale)
                .pre_translate(-cx, -cy);
            let paint = tiny_skia::PixmapPaint {
                opacity: pose.alpha,
                quality: tiny_skia::FilterQuality::Bilinear,
                ..Default::default()
            };
            dst.draw_pixmap(0, 0, src, &paint, t, None);
            return;
        }
        let dy = (pose.dy * self.scale).round().max(0.0) as usize;
        let rows = h.min(fh.saturating_sub(dy));
        let n = rows * w * 4;
        self.frame[dy * w * 4..dy * w * 4 + n].copy_from_slice(&src[..n]);
        if pose.alpha < 1.0 {
            let a = (pose.alpha.clamp(0.0, 1.0) * 256.0) as u32;
            for b in &mut self.frame {
                *b = ((*b as u32 * a) >> 8) as u8;
            }
        }
    }

    fn draw(&mut self) {
        let d = render::render_card(&self.snap, &self.theme, self.scale);
        (self.w, self.h) = (d.pm.width(), d.pm.height());
        self.bgra = render::to_bgra_premul(&d.pm);
        self.drawn = Some(d);
        self.stale = false;
    }

    /// Start lighting the close button up or down; frames draw only the
    /// button again, not the whole card.
    fn set_hover(&mut self, on: bool) -> bool {
        if self.lit_to == on {
            return false;
        }
        let now = Instant::now();
        self.lit_from = if self.calm {
            if on { 0.0 } else { 1.0 }
        } else {
            self.lit(now)
        };
        // Less motion: the button switches at once.
        self.lit_at = if self.calm {
            now - Duration::from_secs(1)
        } else {
            now
        };
        self.lit_to = on;
        true
    }

    /// Put the close button, as `k` has it, into a card's pixels.
    fn patch_close(&self, bgra: &mut [u8], k: render::Knob) {
        let Some(d) = &self.drawn else { return };
        let patch = d.close_patch(&self.theme, k);
        let (x0, y0) = d.close_at();
        let row = patch.width() as usize * 4;
        let src = render::to_bgra_premul(&patch);
        for (y, line) in src.chunks_exact(row).enumerate() {
            let at = ((y0 as usize + y) * self.w as usize + x0 as usize) * 4;
            if let Some(dst) = bgra.get_mut(at..at + row) {
                dst.copy_from_slice(line);
            }
        }
    }

    fn set(&mut self, snap: Snapshot) {
        let now = Instant::now();
        if !self.calm && !has_values(&self.snap) && has_values(&snap) {
            self.values_at = Some(now);
        }
        if !self.calm && self.snap.status() != snap.status() {
            self.status_from = Some((now, self.snap.status()));
        }
        if !self.calm && has_values(&self.snap) && has_values(&snap) {
            if render::readout_pods(&self.snap) != render::readout_pods(&snap) {
                self.update = Some((now, self.snap.clone()));
            }
            if snap.case.charging && !self.snap.case.charging {
                self.charge_at = Some(now);
            }
        }
        let rebuild = layer_key(&self.snap) != layer_key(&snap)
            && (self.layers.is_some() || self.pending.is_some());
        self.snap = snap;
        let layered = self.layers.is_some() && !rebuild;
        if layered && self.settling(now) {
            self.stale = true;
        } else {
            self.draw();
        }
        if rebuild {
            self.start_layers();
        }
    }

    /// Draw again if the backend now shows at another scale.
    fn fit(&mut self, be: &dyn Backend) {
        let s = be.scale();
        if (s - self.scale).abs() > 0.001 {
            debug!(from = self.scale, to = s, "redraw for the output scale");
            let had_layers = self.layers.is_some() || self.pending.is_some();
            self.scale = s;
            self.layers = None;
            self.pending = None;
            self.draw();
            if had_layers {
                self.start_layers();
            }
        }
    }
}

fn apply(cmd: Cmd, card: &mut Card, be: &mut dyn Backend) -> Applied {
    let (snap, reset) = match cmd {
        Cmd::Show(s) => (s, true),
        Cmd::Refresh(s) => (s, false),
        Cmd::Hide => return Applied::Stop(Outcome::Done),
        Cmd::Quit => return Applied::Stop(Outcome::Quit),
    };
    let (title, body) = labels(&snap);
    be.set_labels(&title, &body);
    card.set(snap);
    Applied::Redrawn { reset }
}

/// The card in words: device name, then the levels it shows.
fn labels(snap: &Snapshot) -> (String, String) {
    let mut parts = Vec::with_capacity(3);
    for (side, pod) in [("L", snap.left), ("R", snap.right), ("Case", snap.case)] {
        if let Some(level) = pod.level {
            parts.push(if pod.charging {
                format!("{side} {level} % charging")
            } else {
                format!("{side} {level} %")
            });
        }
    }
    let body = if !parts.is_empty() {
        parts.join(" · ")
    } else if snap.connected {
        snap.mode
            .map(render::mode_label)
            .unwrap_or("Connected")
            .to_string()
    } else {
        "Disconnected".into()
    };
    (snap.name.clone(), body)
}

/// Apply every queued command. `Show` redraws and asks the hold timer
/// to restart (a fresh trigger); `Refresh` only redraws (battery tick);
/// `Hide`/`Quit`/dropped-sender stop the cycle.
fn drain(rx: &Receiver<Cmd>, card: &mut Card, be: &mut dyn Backend) -> Drained {
    let mut reset = false;
    loop {
        match rx.try_recv() {
            Ok(cmd) => match apply(cmd, card, be) {
                Applied::Redrawn { reset: r } => reset |= r,
                Applied::Stop(o) => return Drained::Stop(o),
            },
            Err(TryRecvError::Disconnected) => return Drained::Stop(Outcome::Quit),
            Err(TryRecvError::Empty) => {
                return if reset {
                    Drained::ResetHold
                } else {
                    Drained::Continue
                };
            }
        }
    }
}

/// The hold timer. It stands still while the pointer rests on the card.
struct Hold {
    deadline: Instant,
    /// Time that was left, and since when the pointer has been on it.
    paused: Option<(Duration, Instant)>,
}

impl Hold {
    fn new(now: Instant, hold: Duration) -> Self {
        Hold {
            deadline: now + hold,
            paused: None,
        }
    }

    fn restart(&mut self, now: Instant, hold: Duration) {
        match &mut self.paused {
            Some((left, _)) => *left = hold,
            None => self.deadline = now + hold,
        }
    }

    fn pause(&mut self, now: Instant) {
        if self.paused.is_none() {
            self.paused = Some((self.deadline.saturating_duration_since(now), now));
        }
    }

    fn resume(&mut self, now: Instant) {
        if let Some((left, _)) = self.paused.take() {
            self.deadline = now + left.max(AFTER_HOVER);
        }
    }

    /// Time left, or `None` while paused.
    fn left(&mut self, now: Instant) -> Option<Duration> {
        if let Some((_, since)) = self.paused
            && now.saturating_duration_since(since) >= MAX_HOVER
        {
            info!("pointer on the bubble for {MAX_HOVER:?}, letting it go");
            self.resume(since + MAX_HOVER);
        }
        match self.paused {
            Some(_) => None,
            None => Some(self.deadline.saturating_duration_since(now)),
        }
    }
}

/// `calm`: the desktop asks for less motion; the card then only fades.
fn show_cycle(
    rx: &Receiver<Cmd>,
    pick: &BackendPick,
    theme: &Theme,
    calm: bool,
    hold: Duration,
    snap: Snapshot,
    scale: &mut f32,
) -> Outcome {
    let mut card = Card::new(snap, theme, *scale);
    // Built while the window opens; usually the scale stays the same.
    card.start_layers();
    // Entry and exit included — the notification backend hands this to the server
    // as the expire timeout, and a bubble the server retires early is
    // exactly the "gone too soon" the hold is meant to prevent.
    let on_screen = hold + Duration::from_millis((anim::ENTRY_MS + anim::EXIT_MS) as u64);
    let mut be = match open_backend(&pick.name, pick.explicit, on_screen) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("podctl-popup: backend: {e:#}");
            return Outcome::Done;
        }
    };
    if short(be.kind()) != pick.name {
        info!(wanted = %pick.name, using = short(be.kind()), "backend fell back");
    }
    let (title, body) = labels(&card.snap);
    be.set_labels(&title, &body);
    card.fit(be.as_ref());
    *scale = card.scale;

    // A server that cannot blend (a notification, X without a
    // compositor) gets the card standing still.
    let moves = be.animates();
    card.calm = calm;
    if moves && !calm {
        card.wait_layers(LAYER_WAIT);
    }
    let mut pose = Pose::REST;
    let mut stop = None;
    let t0 = Instant::now();
    if moves && !calm {
        card.mapped(t0);
    }
    let enter = |ms: f32| {
        if calm {
            anim::calm_entry(ms)
        } else {
            anim::entry(ms)
        }
    };
    let mut frames = 0u32;
    while let Some(p) = moves.then(|| enter(ms_since(t0))).flatten() {
        frames += 1;
        pose = p;
        let art = (!calm).then(|| anim::art(ms_since(t0)));
        if push_moving(be.as_mut(), &mut card, pose, art).is_err() {
            let _ = be.close();
            return Outcome::Done;
        }
        if let Drained::Stop(o) = drain(rx, &mut card, be.as_mut()) {
            stop = Some(o);
            break;
        }
        be.wait_frame();
    }
    debug!(frames, ms = ms_since(t0), "entry");
    let mut clicked = false;

    let outcome = match stop {
        Some(o) => o,
        None => {
            pose = Pose::REST;
            let mut timer = Hold::new(Instant::now(), hold);
            let mut o = Outcome::Done;
            let mut settled = false;
            // Nothing moves while the bubble stands still, so it waits
            // for the next command instead of pushing the same frame
            // sixty times a second: that was ~340 commits per bubble,
            // and on X11 a full 260 kB upload plus a shape change each
            // time.
            'hold: loop {
                let mut redraw = false;
                for ev in be.input() {
                    debug!(?ev, "pointer");
                    let now = Instant::now();
                    match ev {
                        Input::Hover(true) => timer.pause(now),
                        Input::Hover(false) => {
                            timer.resume(now);
                            redraw |= card.set_hover(false);
                        }
                        Input::OverClose(on) => redraw |= card.set_hover(on),
                        Input::Click => {
                            info!("hiding: clicked");
                            clicked = true;
                            break 'hold;
                        }
                    }
                }
                if redraw && push(be.as_mut(), &mut card, Pose::REST).is_err() {
                    let _ = be.close();
                    return o;
                }
                // Values still coming in are drawn frame by frame; the
                // first still frame after them is the card drawn whole.
                let settling = moves && card.settling(Instant::now());
                if settling || !settled {
                    if push(be.as_mut(), &mut card, Pose::REST).is_err() {
                        let _ = be.close();
                        return o;
                    }
                    settled = !settling;
                    if settling {
                        be.wait_frame();
                    }
                }
                let wait = match timer.left(Instant::now()) {
                    Some(d) if d.is_zero() => {
                        info!("hiding: time is up");
                        break;
                    }
                    _ if settling => Duration::ZERO,
                    Some(d) => d.min(INPUT_SLICE),
                    None => INPUT_SLICE,
                };
                let cmd = match rx.recv_timeout(wait) {
                    Ok(c) => c,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => {
                        o = Outcome::Quit;
                        break;
                    }
                };
                let mut reset = match apply(cmd, &mut card, be.as_mut()) {
                    Applied::Redrawn { reset } => reset,
                    Applied::Stop(s) => {
                        o = s;
                        break;
                    }
                };
                // Whatever else is queued belongs to the same redraw.
                match drain(rx, &mut card, be.as_mut()) {
                    Drained::Stop(s) => {
                        o = s;
                        break;
                    }
                    Drained::ResetHold => reset = true,
                    Drained::Continue => {}
                }
                if reset {
                    timer.restart(Instant::now(), hold);
                }
                settled = false;
                if push(be.as_mut(), &mut card, Pose::REST).is_err() {
                    let _ = be.close();
                    return o;
                }
            }
            o
        }
    };

    if clicked && !calm && moves {
        let t0 = Instant::now();
        while let Some(p) = anim::press(ms_since(t0)) {
            card.pressed = ms_since(t0) / anim::PRESS_MS;
            if push(be.as_mut(), &mut card, p).is_err() {
                break;
            }
            pose = p;
            be.wait_frame();
        }
    }
    let leave = match (calm, clicked) {
        (true, _) => anim::calm_exit,
        (false, true) => anim::dismiss,
        (false, false) => anim::exit,
    };
    let t0 = Instant::now();
    while let Some(p) = moves.then(|| leave(ms_since(t0), pose)).flatten() {
        if push(be.as_mut(), &mut card, p).is_err() {
            break;
        }
        be.wait_frame();
    }
    let _ = be.close();
    outcome
}

fn ms_since(t0: Instant) -> f32 {
    t0.elapsed().as_secs_f32() * 1000.0
}

async fn watch(tx: &Sender<Cmd>, visible: Duration) {
    let mut snap = Snapshot::sample();
    snap.connected = false;
    snap.mode = None;
    snap.left = Pod::default();
    snap.right = Pod::default();
    snap.case = Pod::default();

    let mut backoff = BACKOFF_INITIAL_MS;
    let mut shown_until: Option<Instant> = None;
    let mut low_armed = false;
    loop {
        let attached = serve(tx, &mut snap, &mut shown_until, &mut low_armed, visible).await;
        if shown_until.take().is_some() {
            let _ = tx.send(Cmd::Hide);
        }
        // Only a stream we never got backs off. Without this reset the
        // delay ratcheted up to 30 s for the lifetime of the process, so
        // a daemon restart could leave the popup deaf to lid events for
        // half a minute — the bubble just never appeared.
        backoff = if attached {
            BACKOFF_INITIAL_MS
        } else {
            backoff.saturating_mul(2).min(BACKOFF_MAX_MS)
        };
        tokio::time::sleep(Duration::from_millis(backoff)).await;
    }
}

/// Run one watch session. Returns true if the event stream was actually
/// established, which is what tells the caller whether to back off.
async fn serve(
    tx: &Sender<Cmd>,
    snap: &mut Snapshot,
    shown_until: &mut Option<Instant>,
    low_armed: &mut bool,
    visible: Duration,
) -> bool {
    if let Ok(Response::State(ds)) = oneshot(&Request::Status).await {
        apply_status(snap, ds);
    }

    let Ok(mut lines) = podctl::client::async_io::watch(WatchRole::Popup).await else {
        return false;
    };
    let mut pending_open: Option<Instant> = None;
    let mut ear_changed: Option<Instant> = None;
    let mut locked = LockState::default();
    loop {
        let tick = pending_open.map(tokio::time::Instant::from_std);
        tokio::select! {
            _ = async { sleep_until(tick.unwrap()).await }, if tick.is_some() => {
                pending_open = None;
                let _ = tx.send(Cmd::Show(snap.clone()));
                *shown_until = Some(Instant::now() + visible);
            }
            line = lines.next_line() => {
                let Ok(Some(l)) = line else { return true };
                if l.is_empty() {
                    continue;
                }
                if let Ok(Response::Event(ev)) = serde_json::from_str::<Response>(&l) {
                    // Inserting or removing a bud makes the AirPods switch
                    // mode on their own; that is no reason to pop up.
                    if matches!(ev, Event::InEar(_)) {
                        ear_changed = Some(Instant::now());
                    }
                    if let Event::Mode { mode } = ev
                        && ear_changed.is_some_and(|t| t.elapsed() < AUTO_MODE_WINDOW)
                    {
                        snap.mode = Some(mode);
                        if is_shown(shown_until) {
                            let _ = tx.send(Cmd::Refresh(snap.clone()));
                        }
                        continue;
                    }
                    let was_connected = snap.connected;
                    // Anything that puts the bubble on screen gets a
                    // fresh Status first: Connect carries no caps or
                    // model, and a process that attached while the
                    // daemon was still starting would otherwise keep
                    // showing that startup moment for its whole life.
                    if matches!(
                        ev,
                        Event::Connected { .. } | Event::ShowPopup | Event::CaseLid { open: true }
                    ) && let Ok(Response::State(ds)) = oneshot(&Request::Status).await
                    {
                        apply_status(snap, ds);
                    }
                    let opts = ShowOpts {
                        visible,
                        allow: matches!(ev, Event::ShowPopup) || !locked.get().await,
                        was_connected,
                    };
                    if !opts.allow {
                        debug!(?ev, "session locked — not showing");
                    }
                    handle_event(tx, snap, ev, &mut pending_open, shown_until, low_armed, opts);
                }
            }
        }
    }
}

fn handle_event(
    tx: &Sender<Cmd>,
    snap: &mut Snapshot,
    ev: Event,
    pending_open: &mut Option<Instant>,
    shown_until: &mut Option<Instant>,
    low_armed: &mut bool,
    opts: ShowOpts,
) {
    let visible = opts.visible;
    match ev {
        Event::CaseLid { open: true } if !is_shown(shown_until) && opts.allow => {
            *pending_open = Some(Instant::now() + Duration::from_millis(DEBOUNCE_MS));
        }
        Event::CaseLid { open: false } => {
            *pending_open = None;
            if shown_until.take().is_some() {
                info!("hiding: case closed");
                let _ = tx.send(Cmd::Hide);
            }
        }
        Event::Battery(b) => {
            let was_blank = snap.blank();
            apply_battery(snap, &b);
            if snap.low() && !*low_armed {
                // Crossed below the low-battery threshold — pop once.
                *low_armed = true;
                if opts.allow {
                    info!("showing: battery low");
                    show_now(tx, snap, pending_open, shown_until, visible);
                }
            } else {
                if battery_recovered(&b) {
                    *low_armed = false;
                }
                if !is_shown(shown_until) {
                    return;
                }
                if was_blank && !snap.blank() {
                    // The bubble went up before the first battery frame
                    // landed, so it has been showing three em-dashes.
                    // Restart the hold: the numbers get the full window,
                    // not whatever was left of it.
                    debug!("first levels arrived, hold starts over");
                    show_now(tx, snap, pending_open, shown_until, visible);
                } else {
                    let _ = tx.send(Cmd::Refresh(snap.clone()));
                }
            }
        }
        // Only a change shows the bubble: while the AirPods reconnect
        // or pair, the same mode and connect come again and again, and
        // each one used to start the hold over — it never went away.
        Event::Mode { mode } => {
            let changed = snap.mode != Some(mode);
            snap.mode = Some(mode);
            if changed && opts.allow {
                info!(?mode, "showing: mode changed");
                show_now(tx, snap, pending_open, shown_until, visible);
            } else if is_shown(shown_until) {
                let _ = tx.send(Cmd::Refresh(snap.clone()));
            }
        }
        Event::Connected { .. } => {
            snap.connected = true;
            if !opts.was_connected && opts.allow {
                info!("showing: connected");
                show_now(tx, snap, pending_open, shown_until, visible);
            }
        }
        Event::ShowPopup => {
            info!("showing: asked for");
            show_now(tx, snap, pending_open, shown_until, visible)
        }
        Event::Disconnected => {
            snap.connected = false;
            snap.mode = None;
            snap.left = Pod::default();
            snap.right = Pod::default();
            snap.case = Pod::default();
        }
        _ => {}
    }
}

/// Show the bubble immediately (no lid debounce) and refresh it if it
/// is already on screen. Used for mode changes, connect, and the
/// explicit `podctl popup` request.
fn show_now(
    tx: &Sender<Cmd>,
    snap: &Snapshot,
    pending_open: &mut Option<Instant>,
    shown_until: &mut Option<Instant>,
    visible: Duration,
) {
    *pending_open = None;
    let _ = tx.send(Cmd::Show(snap.clone()));
    *shown_until = Some(Instant::now() + visible);
}

fn is_shown(shown_until: &Option<Instant>) -> bool {
    shown_until.is_some_and(|t| Instant::now() < t)
}

fn apply_status(snap: &mut Snapshot, ds: DeviceState) {
    snap.connected = ds.connected;
    // Sticky model: a Status from the daemon's startup window can be
    // Unknown — don't overwrite a model we already know.
    let model = ds.capabilities.model;
    if model != podctl::caps::Model::Unknown {
        snap.art = Art::of(model);
        snap.name = model.label().to_string();
    }
    if let Some(name) = ds.name.filter(|n| !n.trim().is_empty()) {
        snap.name = name;
    }
    snap.mode = ds.settings.mode;
    apply_battery(snap, &ds.battery);
}

/// Re-arm the low-battery trigger only once every present level is
/// comfortably back above the threshold (hysteresis against flapping).
fn battery_recovered(b: &Battery) -> bool {
    [b.left, b.right, b.case]
        .into_iter()
        .flatten()
        .all(|l| l >= render::LOW_PCT + 3)
}

fn apply_battery(snap: &mut Snapshot, b: &Battery) {
    snap.left = Pod {
        level: b.left,
        charging: b.left_charging,
    };
    snap.right = Pod {
        level: b.right,
        charging: b.right_charging,
    };
    snap.case = Pod {
        level: b.case,
        charging: b.case_charging,
    };
}

async fn oneshot(req: &Request) -> anyhow::Result<Response> {
    Ok(podctl::client::async_io::send(req).await?)
}

fn single_instance() -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let path = std::path::Path::new(&dir).join("podctl-popup.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .ok()?;
    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;
    if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0 {
        Some(file)
    } else {
        None
    }
}

unsafe extern "C" {
    fn flock(fd: i32, op: i32) -> i32;
}

async fn await_term() {
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

fn dump(path: &str, theme: &Theme, args: &[String]) -> std::io::Result<()> {
    let bad = |m: String| std::io::Error::new(std::io::ErrorKind::InvalidInput, m);
    let art = match flag(args, "--model") {
        Some(slug) => Art::by_slug(slug).ok_or_else(|| {
            let known: Vec<&str> = Art::ALL.iter().map(|a| a.slug()).collect();
            bad(format!(
                "unknown model '{slug}' (one of: {})",
                known.join(", ")
            ))
        })?,
        None => Art::Pro2C,
    };
    let scale = match flag(args, "--scale") {
        Some(v) => v
            .parse::<f32>()
            .ok()
            .filter(|s| (0.5..=4.0).contains(s))
            .ok_or_else(|| bad(format!("scale '{v}' (0.5 to 4)")))?,
        None => 1.0,
    };
    let mut pm = match flag(args, "--at") {
        Some(at) => frame_at(art, theme, scale, at).map_err(bad)?,
        None => render::render(&Snapshot::sample_for(art), theme, scale),
    };
    if args.iter().any(|a| a == "--backdrop") {
        pm = render::on_backdrop(&pm, theme.dark);
    }
    let png = pm
        .encode_png()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(path, png)
}

/// One frame of the motion, as `<phase>:<ms>` names it (`entry:120`,
/// `exit:100`, `dismiss:60`, or `values:200` for values that arrive
/// while "Connecting…" shows), in the frame the window shows.
fn frame_at(art: Art, theme: &Theme, scale: f32, at: &str) -> Result<tiny_skia::Pixmap, String> {
    let wrong = || format!("at '{at}' (entry|exit|dismiss|values:<ms>)");
    let (phase, ms) = at.split_once(':').ok_or_else(wrong)?;
    let ms: f32 = ms.parse().map_err(|_| wrong())?;
    let pose = match phase {
        "entry" => anim::entry(ms),
        "exit" => anim::exit(ms, Pose::REST),
        "dismiss" => anim::dismiss(ms, Pose::REST),
        "values" => Some(Pose::REST),
        _ => return Err(wrong()),
    }
    .unwrap_or(match phase {
        "entry" => Pose::REST,
        _ => Pose {
            alpha: 0.0,
            ..Pose::REST
        },
    });
    let mut card = Card::new(Snapshot::sample_for(art), theme, scale);
    if phase == "entry" || phase == "values" {
        card.layers = Some(Layers::build(&card.snap, theme, scale));
    }
    // After the build, or the frame comes out that much later.
    let now = Instant::now();
    let back = |ms: f32| {
        let d = Duration::from_secs_f32(ms.abs() / 1000.0);
        if ms >= 0.0 { now - d } else { now + d }
    };
    let moving = match phase {
        "entry" => {
            card.mapped(back(ms));
            Some(anim::art(ms))
        }
        "values" => {
            card.values_at = Some(back(ms));
            card.status_from = Some((back(ms), "Connecting…"));
            None
        }
        _ => None,
    };
    card.compose(pose, moving);
    let mut rgba = std::mem::take(&mut card.frame);
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    tiny_skia::Pixmap::from_vec(
        rgba,
        tiny_skia::IntSize::from_wh(card.w, card.frame_h).ok_or_else(wrong)?,
    )
    .ok_or_else(wrong)
}

type Change = fn(&mut Snapshot);

/// `updates`: the levels change while the card is up, as the design's
/// update sequence has it, to see those changes play.
fn demo(
    theme: &Theme,
    calm: bool,
    updates: bool,
    want: Option<String>,
    hold: Duration,
) -> anyhow::Result<()> {
    let pick = BackendPick {
        name: want.clone().unwrap_or_else(|| "wl".into()),
        explicit: want.is_some(),
    };
    // Kept alive, or the cycle reads the closed channel as Quit.
    let (tx, rx) = mpsc::channel::<Cmd>();
    let mut snap = Snapshot::sample();
    if updates {
        snap.case.charging = false;
        let mut s = snap.clone();
        let tx = tx.clone();
        thread::spawn(move || {
            let steps: [(u64, Change); 4] = [
                (1100, |s| s.case.charging = true),
                (2000, |s| {
                    (s.left.level, s.right.level) = (Some(78), Some(76))
                }),
                (2900, |s| s.left.level = Some(18)),
                (4200, |s| s.left.level = Some(76)),
            ];
            let t0 = Instant::now();
            for (at, change) in steps {
                thread::sleep(Duration::from_millis(at).saturating_sub(t0.elapsed()));
                change(&mut s);
                if tx.send(Cmd::Refresh(s.clone())).is_err() {
                    return;
                }
            }
        });
    }
    let mut scale = 1.0;
    show_cycle(&rx, &pick, theme, calm, hold, snap, &mut scale);
    drop(tx);
    eprintln!("podctl-popup: scale {scale}");
    Ok(())
}

/// `explicit` means the backend was named by the user rather than
/// detected. Falling back then hides the very failure they asked about:
/// a popup service that quietly became a desktop notification still
/// logged the backend it had picked, not the one it was using.
fn open_backend(
    pick: &str,
    explicit: bool,
    on_screen: Duration,
) -> anyhow::Result<Box<dyn Backend>> {
    let order: &[&str] = match (pick, explicit) {
        ("gnome", false) => &["gnome", "x11", "notify"],
        ("gnome", true) => &["gnome"],
        ("wl" | "wayland" | "wl_layer", false) => &["wl", "x11", "notify"],
        ("x11" | "x11_or", false) => &["x11", "notify"],
        ("wl" | "wayland" | "wl_layer", true) => &["wl"],
        ("x11" | "x11_or", true) => &["x11"],
        ("notify", _) => &["notify"],
        (other, _) => anyhow::bail!("unknown backend '{other}' (use wl|gnome|x11|notify)"),
    };
    let mut last = anyhow::anyhow!("no backend available");
    for kind in order {
        let mut be: Box<dyn Backend> = match *kind {
            "wl" => Box::new(backend::wl_layer::WlLayer::new()),
            "gnome" => Box::new(backend::gnome::Gnome::new()),
            "x11" => Box::new(backend::x11_or::X11Or::new()),
            _ => Box::new(backend::notify::Notify::new()),
        };
        be.set_hold(on_screen);
        match be.open(BUF_W, BUF_H + RISE as u32) {
            Ok(()) => return Ok(be),
            Err(e) => last = e.context(format!("{kind} backend")),
        }
    }
    Err(last)
}

fn push(be: &mut dyn Backend, card: &mut Card, pose: Pose) -> anyhow::Result<()> {
    push_moving(be, card, pose, None)
}

fn push_moving(
    be: &mut dyn Backend,
    card: &mut Card,
    pose: Pose,
    art: Option<anim::Art>,
) -> anyhow::Result<()> {
    card.fit(be);
    card.compose(pose, art);
    be.push_frame(&Frame {
        bgra: &card.frame,
        w: card.w,
        h: card.frame_h,
    })
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).map(String::as_str)
}

fn fail(msg: &str, code: i32) -> ExitCode {
    eprintln!("podctl-popup: {msg}");
    ExitCode::from(code as u8)
}

fn print_usage() {
    println!("podctl-popup — AirPods case-open bubble");
    println!();
    println!("usage:");
    println!(
        "  podctl-popup --dump <file.png> [--theme auto|dark|light] [--model <slug>] [--scale <n>] [--at entry|exit|dismiss|values:<ms>] [--backdrop]"
    );
    println!(
        "  podctl-popup --demo [--backend wl|gnome|x11|notify] [--theme auto|dark|light] [--motion auto|full|reduced] [--hold <ms>] [--updates]"
    );
    println!("  podctl-popup                               (run; watches the daemon)");
}

#[cfg(test)]
mod tests {
    use super::{AFTER_HOVER, Hold, MAX_HOVER, labels};
    use std::time::{Duration, Instant};

    #[test]
    fn the_same_mode_or_connect_again_does_not_start_the_bubble_over() {
        use super::{Cmd, ShowOpts, handle_event};
        use podctl::model::{Event, Mode};
        let (tx, rx) = std::sync::mpsc::channel();
        let mut snap = snap();
        snap.mode = None;
        let (mut pending, mut until, mut low) = (None, None, false);
        let opts = |was_connected| ShowOpts {
            visible: Duration::from_secs(6),
            allow: true,
            was_connected,
        };
        let mut run = |ev: Event, was| {
            handle_event(
                &tx,
                &mut snap,
                ev,
                &mut pending,
                &mut until,
                &mut low,
                opts(was),
            )
        };
        run(
            Event::Mode {
                mode: Mode::NoiseCancellation,
            },
            true,
        );
        run(
            Event::Mode {
                mode: Mode::NoiseCancellation,
            },
            true,
        );
        run(
            Event::Mode {
                mode: Mode::NoiseCancellation,
            },
            true,
        );
        run(
            Event::Connected {
                name: "Test AirPods".into(),
                address: "AA:BB:CC:DD:EE:01".into(),
            },
            true,
        );
        run(
            Event::Mode {
                mode: Mode::Transparency,
            },
            true,
        );
        let shows = rx.try_iter().filter(|c| matches!(c, Cmd::Show(_))).count();
        assert_eq!(shows, 2, "first mode and the real change only");
    }

    #[test]
    fn the_pointer_holds_the_bubble() {
        let s = |n: u64| Duration::from_millis(n);
        let t0 = Instant::now();
        let mut h = Hold::new(t0, s(5500));
        assert_eq!(h.left(t0 + s(1000)), Some(s(4500)));
        h.pause(t0 + s(1000));
        assert_eq!(h.left(t0 + s(20_000)), None);
        // Back with plenty left: the rest runs on.
        h.resume(t0 + s(20_000));
        assert_eq!(h.left(t0 + s(20_000)), Some(s(4500)));
        // A hover that never ends lets go after MAX_HOVER.
        let mut h = Hold::new(t0, s(5500));
        h.pause(t0 + s(1000));
        assert_eq!(h.left(t0 + s(1000) + MAX_HOVER), Some(s(4500)));
        // Back with almost nothing left: at least AFTER_HOVER.
        let mut h = Hold::new(t0, s(5500));
        h.pause(t0 + s(5400));
        h.resume(t0 + s(9000));
        assert_eq!(h.left(t0 + s(9000)), Some(AFTER_HOVER));
        // A fresh trigger while paused only refills what is left.
        let mut h = Hold::new(t0, s(5500));
        h.pause(t0 + s(5000));
        h.restart(t0 + s(6000), s(5500));
        assert_eq!(h.left(t0 + s(7000)), None);
        h.resume(t0 + s(7000));
        assert_eq!(h.left(t0 + s(7000)), Some(s(5500)));
    }
    use crate::art::Art;
    use crate::render::{Pod, Snapshot};

    fn snap() -> Snapshot {
        Snapshot {
            name: "AirPods Pro 2".into(),
            art: Art::Pro2C,
            connected: true,
            mode: Some(podctl::model::Mode::Transparency),
            left: Pod {
                level: Some(82),
                charging: false,
            },
            right: Pod {
                level: Some(79),
                charging: false,
            },
            case: Pod {
                level: Some(64),
                charging: true,
            },
        }
    }

    /// The notification fallback shows these two lines instead of the
    /// card; it used to send the device name and an empty body.
    #[test]
    fn levels_become_one_line() {
        let (title, body) = labels(&snap());
        assert_eq!(title, "AirPods Pro 2");
        assert_eq!(body, "L 82 % · R 79 % · Case 64 % charging");
    }

    #[test]
    fn without_levels_the_state_stands_in() {
        let mut s = snap();
        s.left = Pod::default();
        s.right = Pod::default();
        s.case = Pod::default();
        assert_eq!(labels(&s).1, "Transparency");
        s.mode = None;
        assert_eq!(labels(&s).1, "Connected");
        s.connected = false;
        assert_eq!(labels(&s).1, "Disconnected");
    }
}
