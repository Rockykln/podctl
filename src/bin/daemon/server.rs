//! Unix-domain socket server. One accept loop, one task per connection.
//!
//! On startup we unlink any stale socket file (left over from a crashed
//! previous daemon), then bind fresh. The socket is created with the
//! daemon's umask so it's owned-and-readable only by the user.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use podctl::ipc::ErrorCode;
use podctl::{Request, Response, WatchRole};

use super::Daemon;

/// A request is one JSON line. Without a cap, a client that never sends
/// a newline makes the daemon buffer until it is killed.
const MAX_REQUEST_BYTES: u64 = 64 * 1024;

/// How long a connection may stay silent after opening. A socket that
/// connects and says nothing used to hold its task for good.
const FIRST_LINE: Duration = Duration::from_secs(5);

/// Connections handled at once. `watch` streams are long-lived, so this
/// is what stops a loop of clients from using up the daemon's memory
/// and file descriptors.
const MAX_CONNECTIONS: usize = 64;

pub async fn serve(daemon: Arc<Daemon>) -> anyhow::Result<()> {
    let path = podctl::socket_path();
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent)?;
    }
    // Stale-socket handling: bind first; only on EADDRINUSE do we probe
    // whether a real daemon is listening. This collapses the previous
    // exists / connect / remove / bind sequence into one race-free path.
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            match UnixStream::connect(&path).await {
                Ok(_) => anyhow::bail!(
                    "another podctld is already listening on {} — stop it first",
                    path.display()
                ),
                Err(_) => {
                    debug!(path = %path.display(), "removing stale socket");
                    std::fs::remove_file(&path)?;
                    UnixListener::bind(&path)?
                }
            }
        }
        Err(e) => return Err(e.into()),
    };
    info!(path = %path.display(), "podctld listening");

    // Best-effort: tighten the permissions to 0600 so even other users on
    // the box can't poke our daemon.
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(&path, perm);
    }

    let gate = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (sock, _addr) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                // A transient accept error (EMFILE, ECONNABORTED…) must
                // not tear down the whole daemon. Log, breathe, retry.
                warn!(error = %e, "accept failed, retrying");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(permit) = Arc::clone(&gate).try_acquire_owned() else {
            warn!(
                limit = MAX_CONNECTIONS,
                "too many connections — turning one away"
            );
            let mut sock = sock;
            let resp = Response::err("the daemon is handling too many connections right now.");
            let _ = write_line(&mut sock, &resp).await;
            continue;
        };
        let d = Arc::clone(&daemon);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(sock, d).await {
                warn!(error = %e, "connection ended with error");
            }
            drop(permit);
        });
    }
}

async fn handle_connection(sock: UnixStream, daemon: Arc<Daemon>) -> anyhow::Result<()> {
    let (rx, mut tx) = sock.into_split();
    let mut lines = BufReader::new(rx.take(MAX_REQUEST_BYTES)).lines();

    let first = match timeout(FIRST_LINE, lines.next_line()).await {
        Ok(Ok(Some(line))) => line,
        Ok(Ok(None)) => return Ok(()),
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => {
            let resp = Response::err_code(ErrorCode::Usage, "no request within five seconds.");
            let _ = write_line(&mut tx, &resp).await;
            return Ok(());
        }
    };
    if first.len() as u64 >= MAX_REQUEST_BYTES {
        let resp = Response::err_code(
            ErrorCode::Usage,
            format!("request longer than {MAX_REQUEST_BYTES} bytes."),
        );
        write_line(&mut tx, &resp).await?;
        return Ok(());
    }
    let req: Request = match serde_json::from_str(first.trim()) {
        Ok(r) => r,
        Err(e) => {
            // Usage, because the request is what is wrong — a client
            // of another version, most of the time.
            let resp = Response::err_code(ErrorCode::Usage, format!("malformed request: {e}"));
            write_line(&mut tx, &resp).await?;
            return Ok(());
        }
    };

    // `watch` flips this connection into a long-lived event stream.
    if let Request::Watch { role } = req {
        write_line(&mut tx, &Response::ok_done()).await?;
        // Held for the life of the stream so `show-popup` can tell
        // whether anyone is left to draw the bubble.
        let _popup = match role {
            WatchRole::Popup => Some(daemon.attach_popup()),
            WatchRole::Observer => None,
        };
        let mut events = daemon.subscribe();
        let mut resync = daemon.subscribe_resync();
        resync.mark_unchanged();
        // The state as it is right now, so a subscriber does not have to
        // ask on a second connection — between those two an event could
        // slip through unseen. The cached one: polling BlueZ here would
        // hold up the subscription.
        write_line(&mut tx, &Response::ok_state(daemon.snapshot().await)).await?;
        loop {
            tokio::select! {
                evt = events.recv() => match evt {
                    Ok(evt) => {
                        let frame = Response::ok_event(evt);
                        if write_line(&mut tx, &frame).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // Events were dropped, so the client's picture is
                        // incomplete: send the whole state instead of
                        // leaving it to guess.
                        warn!(dropped = n, "watch client lagged — resending the state");
                        if write_line(&mut tx, &Response::ok_state(daemon.snapshot().await))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                Ok(()) = resync.changed() => {
                    if write_line(&mut tx, &Response::ok_state(daemon.snapshot().await))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                // A hangup, or anything the client says on a stream that
                // takes no requests. Without this the task and, worse, a
                // popup registration outlived the client that left, so
                // `show-popup` kept reporting success with nobody there
                // to draw.
                end = lines.next_line() => {
                    match end {
                        Ok(None) => debug!("watch client hung up"),
                        Ok(Some(_)) => debug!("watch client sent a request on a stream"),
                        Err(e) => debug!(error = %e, "watch client read ended"),
                    }
                    break;
                }
            }
        }
        return Ok(());
    }

    let resp = daemon.handle(req).await;
    write_line(&mut tx, &resp).await?;
    Ok(())
}

async fn write_line<W: AsyncWriteExt + Unpin>(w: &mut W, resp: &Response) -> anyhow::Result<()> {
    let mut json = serde_json::to_vec(resp)?;
    json.push(b'\n');
    w.write_all(&json).await?;
    w.flush().await?;
    Ok(())
}
