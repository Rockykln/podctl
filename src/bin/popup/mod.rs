mod anim;
mod assets;
mod backend;
mod config;
mod render;

use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, sleep};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::signal::unix::{SignalKind, signal};
use tokio::time::sleep_until;

use tracing::{debug, info};

use podctl::exitcode;
use podctl::model::{Battery, DeviceState, Event};
use podctl::{Request, Response, WatchRole, socket_path};

use anim::{FRAME, Slide};
use backend::{Backend, Frame};
use config::Pick;
use render::{CARD_H, CARD_W, Pod, Snapshot, Theme};

const REST_MARGIN: i32 = 24;
const DEMO_ANIM_MS: u32 = 200;
const DEMO_HOLD_MS: u64 = 5_000;
const DEBOUNCE_MS: u64 = 500;
const BACKOFF_INITIAL_MS: u64 = 500;
const BACKOFF_MAX_MS: u64 = 30_000;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    init_tracing();
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        return ExitCode::SUCCESS;
    }

    let theme = Theme::by_name(flag(&args, "--theme").unwrap_or("dark"));

    if let Some(path) = flag(&args, "--dump") {
        return match dump(path, &theme) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&format!("dump: {e}"), exitcode::OSERR),
        };
    }

    if args.iter().any(|a| a == "--demo") {
        let want = flag(&args, "--backend").unwrap_or("wl");
        return match demo(&theme, want) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&format!("demo: {e:#}"), exitcode::UNAVAILABLE),
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

    let pick = match cfg.backend {
        Pick::Auto => backend::detect::detect(),
        other => other.as_str(),
    }
    .to_string();
    let theme = Theme::by_name(&cfg.theme);
    let hold = Duration::from_millis(cfg.duration_ms);
    let anim_ms = cfg.anim_ms;
    let visible = Duration::from_millis(config::visible_ms(&cfg));

    info!(
        backend = %pick,
        theme = %cfg.theme,
        visible_ms = visible.as_millis() as u64,
        "podctl-popup ready"
    );

    let (tx, rx) = mpsc::channel::<Cmd>();
    let render = thread::spawn(move || render_loop(rx, &pick, theme, anim_ms, hold));

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

fn render_loop(rx: Receiver<Cmd>, pick: &str, theme: Theme, anim_ms: u32, hold: Duration) {
    while let Ok(cmd) = rx.recv() {
        let snap = match cmd {
            Cmd::Show(s) => s,
            // A refresh with nothing on screen is stale state — ignore
            // it instead of popping an unrequested bubble.
            Cmd::Refresh(_) | Cmd::Hide => continue,
            Cmd::Quit => return,
        };
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

/// Apply every queued command. `Show` redraws and asks the hold timer
/// to restart (a fresh trigger); `Refresh` only redraws (battery tick);
/// `Hide`/`Quit`/dropped-sender stop the cycle.
fn drain(rx: &Receiver<Cmd>, bgra: &mut Vec<u8>, theme: &Theme) -> Drained {
    let mut reset = false;
    loop {
        match rx.try_recv() {
            Ok(Cmd::Show(s)) => {
                *bgra = render::to_bgra_premul(&render::render(&s, theme));
                reset = true;
            }
            Ok(Cmd::Refresh(s)) => {
                *bgra = render::to_bgra_premul(&render::render(&s, theme));
            }
            Ok(Cmd::Hide) => return Drained::Stop(Outcome::Done),
            Ok(Cmd::Quit) | Err(TryRecvError::Disconnected) => {
                return Drained::Stop(Outcome::Quit);
            }
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
    pick: &str,
    theme: &Theme,
    anim_ms: u32,
    hold: Duration,
    snap: Snapshot,
) -> Outcome {
    let mut bgra = render::to_bgra_premul(&render::render(&snap, theme));
    // Slides included — the notification backend hands this to the server
    // as the expire timeout, and a bubble the server retires early is
    // exactly the "gone too soon" the hold is meant to prevent.
    let on_screen = hold + 2 * Duration::from_millis(anim_ms as u64);
    let mut be = match open_backend(pick, on_screen) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("podctl-popup: backend: {e:#}");
            return Outcome::Done;
        }
    };

    let hidden = -(CARD_H as f32);
    let rest = REST_MARGIN as f32;

    let mut last = hidden;
    let mut stop = None;
    for m in Slide::new(hidden, rest, anim_ms) {
        last = m;
        if push(be.as_mut(), &bgra, m).is_err() {
            let _ = be.close();
            return Outcome::Done;
        }
        if let Drained::Stop(o) = drain(rx, &mut bgra, theme) {
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
            while Instant::now() < deadline {
                match drain(rx, &mut bgra, theme) {
                    Drained::Stop(s) => {
                        o = s;
                        break;
                    }
                    Drained::ResetHold => deadline = Instant::now() + hold,
                    Drained::Continue => {}
                }
                if push(be.as_mut(), &bgra, rest).is_err() {
                    let _ = be.close();
                    return o;
                }
                sleep(FRAME);
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

    let stream = match UnixStream::connect(socket_path()).await {
        Ok(s) => s,
        Err(_) => return false,
    };
    let (rx, mut wx) = stream.into_split();
    let Ok(mut hello) = serde_json::to_vec(&Request::Watch {
        role: WatchRole::Popup,
    }) else {
        return false;
    };
    hello.push(b'\n');
    if wx.write_all(&hello).await.is_err() || wx.flush().await.is_err() {
        return false;
    }
    drop(wx);

    let mut lines = BufReader::new(rx).lines();
    let mut pending_open: Option<Instant> = None;
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
                    // Connect carries no caps/model — re-pull a full
                    // Status so the bubble shows the right model & mode.
                    if matches!(ev, Event::Connected { .. })
                        && let Ok(Response::State(ds)) = oneshot(&Request::Status).await
                    {
                        apply_status(snap, ds);
                    }
                    handle_event(tx, snap, ev, &mut pending_open, shown_until, low_armed, visible);
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
    visible: Duration,
) {
    match ev {
        Event::CaseLid { open: true } if !is_shown(shown_until) => {
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
                show_now(tx, snap, pending_open, shown_until, visible);
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
        Event::Mode(m) => {
            snap.mode = Some(m);
            show_now(tx, snap, pending_open, shown_until, visible);
        }
        Event::Connected { .. } => {
            snap.connected = true;
            show_now(tx, snap, pending_open, shown_until, visible);
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
    if ds.capabilities.model != podctl::caps::Model::Unknown {
        snap.model = ds.capabilities.model.label().to_string();
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
    let stream = UnixStream::connect(socket_path()).await?;
    let (rx, mut wx) = stream.into_split();
    let mut line = serde_json::to_vec(req)?;
    line.push(b'\n');
    wx.write_all(&line).await?;
    wx.flush().await?;
    let mut buf = String::new();
    BufReader::new(rx).read_line(&mut buf).await?;
    Ok(serde_json::from_str(buf.trim())?)
}

/// Without this podctl-popup wrote nothing at all — not a startup line,
/// not an error — so a bubble that never appeared left no trace in the
/// journal. The unit sets RUST_LOG=info and that went nowhere.
fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
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

fn dump(path: &str, theme: &Theme) -> std::io::Result<()> {
    let pm = render::render(&Snapshot::sample(), theme);
    let png = pm
        .encode_png()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(path, png)
}

fn demo(theme: &Theme, want: &str) -> anyhow::Result<()> {
    let pm = render::render(&Snapshot::sample(), theme);
    let bgra = render::to_bgra_premul(&pm);
    let mut be = open_backend(want, Duration::from_millis(DEMO_HOLD_MS))?;
    eprintln!("podctl-popup: backend {}", be.kind());
    present(be.as_mut(), &bgra)?;
    be.close()?;
    Ok(())
}

fn open_backend(pick: &str, on_screen: Duration) -> anyhow::Result<Box<dyn Backend>> {
    let order: &[&str] = match pick {
        "wl" | "wayland" | "wl_layer" => &["wl", "x11", "notify"],
        "x11" | "x11_or" => &["x11", "notify"],
        "notify" => &["notify"],
        other => anyhow::bail!("unknown backend '{other}' (use wl|x11|notify)"),
    };
    let mut last = anyhow::anyhow!("no backend available");
    for kind in order {
        let mut be: Box<dyn Backend> = match *kind {
            "wl" => Box::new(backend::wl_layer::WlLayer::new()),
            "x11" => Box::new(backend::x11_or::X11Or::new()),
            _ => Box::new(backend::notify::Notify::new()),
        };
        be.set_hold(on_screen);
        match be.open(CARD_W, CARD_H) {
            Ok(()) => return Ok(be),
            Err(e) => last = e.context(format!("{kind} backend")),
        }
    }
    Err(last)
}

fn present(be: &mut dyn Backend, bgra: &[u8]) -> anyhow::Result<()> {
    let hidden = -(CARD_H as f32);
    for m in Slide::new(hidden, REST_MARGIN as f32, DEMO_ANIM_MS) {
        push(be, bgra, m)?;
        sleep(FRAME);
    }
    for _ in 0..(DEMO_HOLD_MS / 100) {
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
    println!("  podctl-popup --dump <file.png> [--theme dark|light]");
    println!("  podctl-popup --demo [--backend wl|x11|notify] [--theme dark|light]");
    println!("  podctl-popup                               (run; watches the daemon)");
}
