//! Unix-domain socket server. One accept loop, one task per connection.
//!
//! On startup we unlink any stale socket file (left over from a crashed
//! previous daemon), then bind fresh. The socket is created with the
//! daemon's umask so it's owned-and-readable only by the user.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, info, warn};

use podctl::{Request, Response, WatchRole};

use super::Daemon;

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
        let d = Arc::clone(&daemon);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(sock, d).await {
                warn!(error = %e, "connection ended with error");
            }
        });
    }
}

async fn handle_connection(sock: UnixStream, daemon: Arc<Daemon>) -> anyhow::Result<()> {
    let (rx, mut tx) = sock.into_split();
    let mut lines = BufReader::new(rx).lines();

    let Some(first) = lines.next_line().await? else {
        return Ok(());
    };
    let req: Request = match serde_json::from_str(first.trim()) {
        Ok(r) => r,
        Err(e) => {
            let resp = Response::err(format!("malformed request: {e}"));
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
        loop {
            match events.recv().await {
                Ok(evt) => {
                    let frame = Response::ok_event(evt);
                    if write_line(&mut tx, &frame).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(dropped = n, "watch client lagged — events skipped");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
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
