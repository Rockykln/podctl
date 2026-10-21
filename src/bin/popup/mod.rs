mod anim;
mod art;
mod assets;
mod backend;
mod config;
mod paint;
mod render;
mod scheme;

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::{self, sleep};
use std::time::{Duration, Instant};

use tokio::signal::unix::{SignalKind, signal};
use tokio::time::sleep_until;

use tracing::{debug, info, warn};

use podctl::exitcode;
use podctl::model::{Battery, DeviceState, Event};
use podctl::{Request, Response, WatchRole};

use anim::{FRAME, Slide};
use art::Art;
use backend::{Backend, Frame};
use config::{Pick, ThemePick};
use render::{BUF_H, BUF_W, Pod, Snapshot, Theme};

/// The buffer carries the card's shadow, 24 px on every side, so at
/// rest the card itself sits that far above the screen edge.
const REST_MARGIN: i32 = 0;
const DEMO_ANIM_MS: u32 = 200;
const DEMO_HOLD_MS: u64 = 5_000;
const DEBOUNCE_MS: u64 = 500;
const AUTO_MODE_WINDOW: Duration = Duration::from_secs(3);
const BACKOFF_INITIAL_MS: u64 = 500;
const BACKOFF_MAX_MS: u64 = 30_000;

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
        let out = thread::spawn(move || demo(&theme, want, hold)).join();
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
            Pick::Auto => backend::detect::detect(),
            other => other.as_str(),
        }
        .to_string(),
        explicit: cfg.backend != Pick::Auto,
    };
    let dark = Arc::new(AtomicBool::new(is_dark(cfg.theme).await));
    if cfg.theme == ThemePick::Auto {
        tokio::spawn(scheme::follow(dark.clone()));
    }
    let hold = Duration::from_millis(cfg.duration_ms);
    let anim_ms = cfg.anim_ms;
    let visible = Duration::from_millis(config::visible_ms(&cfg));

    info!(
        backend = %pick.name,
        theme = cfg.theme.as_str(),
        dark = dark.load(Ordering::Relaxed),
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
    let render = thread::spawn(move || render_loop(rx, &pick, &dark, anim_ms, hold));

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
    anim_ms: u32,
    hold: Duration,
) {
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
            show_cycle(&rx, pick, &theme, anim_ms, hold, snap),
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

fn apply(cmd: Cmd, bgra: &mut Vec<u8>, theme: &Theme, be: &mut dyn Backend) -> Applied {
    let (snap, reset) = match cmd {
        Cmd::Show(s) => (s, true),
        Cmd::Refresh(s) => (s, false),
        Cmd::Hide => return Applied::Stop(Outcome::Done),
        Cmd::Quit => return Applied::Stop(Outcome::Quit),
    };
    let (title, body) = labels(&snap);
    be.set_labels(&title, &body);
    *bgra = render::to_bgra_premul(&render::render(&snap, theme, 1.0));
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
fn drain(rx: &Receiver<Cmd>, bgra: &mut Vec<u8>, theme: &Theme, be: &mut dyn Backend) -> Drained {
    let mut reset = false;
    loop {
        match rx.try_recv() {
            Ok(cmd) => match apply(cmd, bgra, theme, be) {
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

fn show_cycle(
    rx: &Receiver<Cmd>,
    pick: &BackendPick,
    theme: &Theme,
    anim_ms: u32,
    hold: Duration,
    snap: Snapshot,
) -> Outcome {
    let mut bgra = render::to_bgra_premul(&render::render(&snap, theme, 1.0));
    // Slides included — the notification backend hands this to the server
    // as the expire timeout, and a bubble the server retires early is
    // exactly the "gone too soon" the hold is meant to prevent.
    let on_screen = hold + 2 * Duration::from_millis(anim_ms as u64);
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
    let (title, body) = labels(&snap);
    be.set_labels(&title, &body);

    let hidden = -(BUF_H as f32);
    let rest = REST_MARGIN as f32;

    let mut last = hidden;
    let mut stop = None;
    for m in Slide::new(hidden, rest, anim_ms) {
        last = m;
        if push(be.as_mut(), &bgra, m).is_err() {
            let _ = be.close();
            return Outcome::Done;
        }
        if let Drained::Stop(o) = drain(rx, &mut bgra, theme, be.as_mut()) {
            stop = Some(o);
            break;
        }
        sleep(FRAME);
    }

    let outcome = match stop {
        Some(o) => o,
        None => {
            last = rest;
            let mut deadline = Instant::now() + hold;
            let mut o = Outcome::Done;
            // Nothing moves while the bubble stands still, so it waits
            // for the next command instead of pushing the same frame
            // sixty times a second: that was ~340 commits per bubble,
            // and on X11 a full 260 kB upload plus a shape change each
            // time.
            loop {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let cmd = match rx.recv_timeout(deadline - now) {
                    Ok(c) => c,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        o = Outcome::Quit;
                        break;
                    }
                };
                let mut reset = match apply(cmd, &mut bgra, theme, be.as_mut()) {
                    Applied::Redrawn { reset } => reset,
                    Applied::Stop(s) => {
                        o = s;
                        break;
                    }
                };
                // Whatever else is queued belongs to the same redraw.
                match drain(rx, &mut bgra, theme, be.as_mut()) {
                    Drained::Stop(s) => {
                        o = s;
                        break;
                    }
                    Drained::ResetHold => reset = true,
                    Drained::Continue => {}
                }
                if reset {
                    deadline = Instant::now() + hold;
                }
                if push(be.as_mut(), &bgra, rest).is_err() {
                    let _ = be.close();
                    return o;
                }
            }
            o
        }
    };

    for m in Slide::new(last, hidden, anim_ms) {
        if push(be.as_mut(), &bgra, m).is_err() {
            break;
        }
        sleep(FRAME);
    }
    let _ = be.close();
    outcome
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
                    show_now(tx, snap, pending_open, shown_until, visible);
                } else {
                    let _ = tx.send(Cmd::Refresh(snap.clone()));
                }
            }
        }
        Event::Mode { mode } => {
            snap.mode = Some(mode);
            if opts.allow {
                show_now(tx, snap, pending_open, shown_until, visible);
            }
        }
        Event::Connected { .. } => {
            snap.connected = true;
            if opts.allow {
                show_now(tx, snap, pending_open, shown_until, visible);
            }
        }
        Event::ShowPopup => {
            debug!("show-popup requested");
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
    let mut pm = render::render(&Snapshot::sample_for(art), theme, scale);
    if args.iter().any(|a| a == "--backdrop") {
        pm = render::on_backdrop(&pm, theme.dark);
    }
    let png = pm
        .encode_png()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(path, png)
}

fn demo(theme: &Theme, want: Option<String>, hold: Duration) -> anyhow::Result<()> {
    let snap = Snapshot::sample();
    let pm = render::render(&snap, theme, 1.0);
    let bgra = render::to_bgra_premul(&pm);
    let pick = want.as_deref().unwrap_or("wl");
    let mut be = open_backend(pick, want.is_some(), hold)?;
    eprintln!("podctl-popup: backend {}", short(be.kind()));
    let (title, body) = labels(&snap);
    be.set_labels(&title, &body);
    present(be.as_mut(), &bgra, hold)?;
    be.close()?;
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
        ("wl" | "wayland" | "wl_layer", false) => &["wl", "x11", "notify"],
        ("x11" | "x11_or", false) => &["x11", "notify"],
        ("wl" | "wayland" | "wl_layer", true) => &["wl"],
        ("x11" | "x11_or", true) => &["x11"],
        ("notify", _) => &["notify"],
        (other, _) => anyhow::bail!("unknown backend '{other}' (use wl|x11|notify)"),
    };
    let mut last = anyhow::anyhow!("no backend available");
    for kind in order {
        let mut be: Box<dyn Backend> = match *kind {
            "wl" => Box::new(backend::wl_layer::WlLayer::new()),
            "x11" => Box::new(backend::x11_or::X11Or::new()),
            _ => Box::new(backend::notify::Notify::new()),
        };
        be.set_hold(on_screen);
        match be.open(BUF_W, BUF_H) {
            Ok(()) => return Ok(be),
            Err(e) => last = e.context(format!("{kind} backend")),
        }
    }
    Err(last)
}

fn present(be: &mut dyn Backend, bgra: &[u8], hold: Duration) -> anyhow::Result<()> {
    let hidden = -(BUF_H as f32);
    for m in Slide::new(hidden, REST_MARGIN as f32, DEMO_ANIM_MS) {
        push(be, bgra, m)?;
        sleep(FRAME);
    }
    for _ in 0..(hold.as_millis() / 100) {
        push(be, bgra, REST_MARGIN as f32)?;
        sleep(Duration::from_millis(100));
    }
    for m in Slide::new(REST_MARGIN as f32, hidden, DEMO_ANIM_MS) {
        push(be, bgra, m)?;
        sleep(FRAME);
    }
    Ok(())
}

fn push(be: &mut dyn Backend, bgra: &[u8], margin: f32) -> anyhow::Result<()> {
    be.push_frame(&Frame {
        bgra,
        margin_bottom: margin.round() as i32,
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
        "  podctl-popup --dump <file.png> [--theme auto|dark|light] [--model <slug>] [--scale <n>] [--backdrop]"
    );
    println!(
        "  podctl-popup --demo [--backend wl|x11|notify] [--theme auto|dark|light] [--hold <ms>]"
    );
    println!("  podctl-popup                               (run; watches the daemon)");
}

#[cfg(test)]
mod tests {
    use super::labels;
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
