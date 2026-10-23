//! The socket's promises, against a real daemon: the handshake, the
//! state a subscriber gets, and the three ways a client can misbehave.
//!
//! The daemon needs nothing but a writable `XDG_RUNTIME_DIR` to bind
//! its socket; where it cannot (no D-Bus in a build container, say) the
//! test says so and stops instead of failing.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    socket: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(dir) = self.socket.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// `None` when the daemon never came up — see the module note.
fn start(name: &str) -> Option<Daemon> {
    let dir = std::env::temp_dir().join(format!("podctl-ipc-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let socket = dir.join("podctl.sock");
    let _ = std::fs::remove_file(&socket);
    let child = Command::new(env!("CARGO_BIN_EXE_podctld"))
        .env("XDG_RUNTIME_DIR", &dir)
        .env("RUST_LOG", "warn")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let daemon = Daemon { child, socket };
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if UnixStream::connect(&daemon.socket).is_ok() {
            return Some(daemon);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!("podctld did not bind a socket here — skipping");
    None
}

fn ask(daemon: &Daemon, line: &str) -> String {
    let stream = UnixStream::connect(&daemon.socket).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    writeln!(&stream, "{line}").expect("write");
    let mut answer = String::new();
    BufReader::new(&stream)
        .read_line(&mut answer)
        .expect("read");
    answer
}

/// The handshake tells a client whether the two halves fit, before a
/// setter fails with "malformed request".
#[test]
fn hello_names_the_version_and_the_protocol() {
    let Some(daemon) = start("hello") else { return };
    let answer = ask(&daemon, r#"{"op":"hello"}"#);
    let v: serde_json::Value = serde_json::from_str(&answer).expect("json");
    assert_eq!(v["kind"], "hello", "{answer}");
    assert_eq!(v["data"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["data"]["proto"], podctl::ipc::PROTO);
    assert!(v["data"].get("device").is_some(), "{answer}");
}

/// A subscriber used to need a second connection for the state, and an
/// event could slip through between the two.
#[test]
fn a_watcher_is_given_the_state_at_once() {
    let Some(daemon) = start("watch") else { return };
    let stream = UnixStream::connect(&daemon.socket).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    writeln!(&stream, r#"{{"op":"watch"}}"#).expect("write");
    let mut reader = BufReader::new(&stream);
    let mut ack = String::new();
    reader.read_line(&mut ack).expect("ack");
    assert!(ack.contains("\"done\""), "{ack}");
    let mut state = String::new();
    reader.read_line(&mut state).expect("state");
    let v: serde_json::Value = serde_json::from_str(&state).expect("json");
    assert_eq!(v["kind"], "state", "{state}");
}

/// A connection that says nothing held its task for good.
#[test]
fn a_silent_connection_is_let_go() {
    let Some(daemon) = start("silent") else {
        return;
    };
    let stream = UnixStream::connect(&daemon.socket).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .expect("timeout");
    let mut answer = String::new();
    BufReader::new(&stream)
        .read_line(&mut answer)
        .expect("read");
    assert!(answer.contains("no request within"), "{answer}");
}

/// Without a cap the daemon buffers a line that never ends until it is
/// killed.
#[test]
fn an_endless_request_is_refused() {
    let Some(daemon) = start("long") else { return };
    let padded = format!(r#"{{"op":"status","pad":"{}"}}"#, "x".repeat(70_000));
    let stream = UnixStream::connect(&daemon.socket).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    // The daemon answers and hangs up once the limit is reached, so the
    // rest of the write can find the pipe gone; the answer is there anyway.
    let _ = writeln!(&stream, "{padded}");
    let mut answer = String::new();
    BufReader::new(&stream)
        .read_line(&mut answer)
        .expect("read");
    assert!(answer.contains("longer than"), "{answer}");
}

/// Through the library's own client, which reads the stream with one
/// buffer: a reader built per answer and dropped again swallowed the
/// state line that follows the acknowledgement.
#[test]
fn the_client_sees_the_state_after_the_acknowledgement() {
    let Some(daemon) = start("client") else {
        return;
    };
    let mut client = podctl::client::Client::connect_at(&daemon.socket).expect("connect");
    let req = podctl::Request::Watch {
        role: podctl::WatchRole::Observer,
    };
    let ack = client
        .request(&req, Duration::from_secs(5))
        .expect("subscribe");
    assert!(matches!(ack, podctl::Response::Done), "{ack:?}");
    let state = client
        .next_line(Some(Duration::from_secs(5)))
        .expect("state");
    assert!(matches!(state, podctl::Response::State(_)), "{state:?}");
}

/// An error says what kind it is, so the CLI can turn it into an exit
/// code a script can act on: every daemon error used to be 69.
#[test]
fn errors_carry_a_kind() {
    let Some(daemon) = start("codes") else { return };
    let mut client = podctl::client::Client::connect_at(&daemon.socket).expect("connect");

    // Spatial audio is either not supported by the model or not
    // implemented by podctl — never a service that might come back.
    let answer = client
        .request(
            &podctl::Request::SetSpatial {
                mode: podctl::model::SpatialAudio::Off,
            },
            Duration::from_secs(5),
        )
        .expect("answer");
    let podctl::Response::Err(info) = answer else {
        panic!("expected an error, got {answer:?}");
    };
    assert!(
        matches!(
            info.code,
            podctl::ipc::ErrorCode::Unsupported | podctl::ipc::ErrorCode::NotImplemented
        ),
        "{info:?}"
    );

    // A request the daemon cannot read is the client's fault.
    let stream = UnixStream::connect(&daemon.socket).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    writeln!(&stream, r#"{{"op":"nonsense"}}"#).expect("write");
    let mut answer = String::new();
    BufReader::new(&stream)
        .read_line(&mut answer)
        .expect("read");
    let v: serde_json::Value = serde_json::from_str(&answer).expect("json");
    assert_eq!(v["data"]["code"], "usage", "{answer}");
}
