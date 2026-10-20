//! The one way to talk to the daemon. Connecting, writing a line,
//! reading one back and telling a timeout from a hangup was spelled out
//! again in the CLI, the tray, the bubble and the debug report, each
//! with its own timeouts and its own idea of what an error is.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::ipc::{Hello, PROTO, Request, Response, socket_path};

/// Writing never blocks for long — the daemon reads a line at a time.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum Error {
    /// No answer within the time allowed. The daemon may still be
    /// working on it.
    Timeout(Duration),
    /// The daemon closed the connection without answering.
    Closed,
    Io(std::io::Error),
    /// An answer we could not read — a daemon of another version.
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Timeout(d) => write!(
                f,
                "no answer from the daemon within {}s — it may still be working on it.",
                d.as_secs()
            ),
            Error::Closed => f.write_str("the daemon closed the connection without answering."),
            Error::Io(e) => write!(f, "{e}"),
            Error::Malformed(e) => write!(f, "malformed answer: {e}"),
        }
    }
}

impl std::error::Error for Error {}

pub struct Client {
    /// One buffer for the whole conversation: a reader built per answer
    /// and dropped again loses whatever it read past the newline, and
    /// the daemon does send two lines in a row — a `watch` subscription
    /// is acknowledged and the state follows at once.
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    /// `None` when nothing is listening on the socket — the caller then
    /// runs standalone or says the daemon is missing.
    pub fn connect() -> Option<Self> {
        Self::connect_at(&socket_path())
    }

    /// The same for a socket somewhere else than the usual place.
    pub fn connect_at(path: &std::path::Path) -> Option<Self> {
        let stream = UnixStream::connect(path).ok()?;
        let writer = stream.try_clone().ok()?;
        Some(Self {
            reader: BufReader::new(stream),
            writer,
        })
    }

    /// One request, one answer. `wait` is how long the answer may take;
    /// `timeout_for` knows the usual values per request.
    pub fn request(&mut self, req: &Request, wait: Duration) -> Result<Response, Error> {
        self.writer
            .set_write_timeout(Some(WRITE_TIMEOUT))
            .map_err(Error::Io)?;
        let line = serde_json::to_string(req).expect("a request serialises");
        writeln!(self.writer, "{line}").map_err(Error::Io)?;
        self.writer.flush().map_err(Error::Io)?;
        self.next_line(Some(wait))
    }

    /// The next line the daemon sends, for a `watch` stream that goes on
    /// after the first answer. `None` waits for as long as it takes — an
    /// event stream may be quiet for hours.
    pub fn next_line(&mut self, wait: Option<Duration>) -> Result<Response, Error> {
        self.reader
            .get_ref()
            .set_read_timeout(wait)
            .map_err(Error::Io)?;
        let mut answer = String::new();
        let read = self
            .reader
            .read_line(&mut answer)
            .map_err(|e| match e.kind() {
                // A socket timeout surfaces as EAGAIN.
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
                    Error::Timeout(wait.unwrap_or_default())
                }
                _ => Error::Io(e),
            })?;
        if read == 0 {
            return Err(Error::Closed);
        }
        serde_json::from_str(answer.trim()).map_err(|e| Error::Malformed(e.to_string()))
    }
}

/// How long an answer may take. Everything that reads cached state is
/// immediate, but `pair` scans for twelve seconds before it answers at
/// all, and BlueZ takes its time over a connect.
pub fn timeout_for(req: &Request) -> Duration {
    let secs = match req {
        Request::Pair { .. } => 45,
        Request::Connect | Request::Disconnect | Request::Unpair => 30,
        _ => 10,
    };
    Duration::from_secs(secs)
}

/// Who is on the other end: the daemon's version and protocol. `None`
/// when no daemon is listening; `Some(None)` when one is, but is too old
/// to introduce itself.
pub fn hello() -> Option<Option<Hello>> {
    let mut client = Client::connect()?;
    match client.request(&Request::Hello, Duration::from_secs(2)) {
        Ok(Response::Hello(h)) => Some(Some(h)),
        // A daemon from before the handshake answers "malformed
        // request" — that is an answer too.
        Ok(_) | Err(_) => Some(None),
    }
}

/// What to tell the user when the daemon on the socket is not the one
/// this binary belongs to. `None` when they match, or when there is no
/// daemon at all.
pub fn version_mismatch() -> Option<String> {
    let ours = env!("CARGO_PKG_VERSION");
    match hello()? {
        Some(h) if h.version == ours && h.proto == PROTO => None,
        Some(h) => Some(format!(
            "podctld is {} (protocol {}), this is {ours} (protocol {PROTO}) — \
             restart the services with 'podctl reboot'.",
            h.version, h.proto
        )),
        None => Some(format!(
            "podctld is older than this {ours} build (it does not know the version handshake) — \
             restart the services with 'podctl reboot'."
        )),
    }
}

/// The same conversation for the two processes that run a tokio
/// runtime: the tray and the bubble. They each had their own copy, with
/// their own timeouts.
pub mod async_io {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
    use tokio::net::UnixStream;
    use tokio::net::unix::OwnedReadHalf;
    use tokio::time::timeout;

    use super::{Error, WRITE_TIMEOUT, timeout_for};
    use crate::ipc::{Request, Response, WatchRole, socket_path};

    /// One request, one answer.
    pub async fn send(req: &Request) -> Result<Response, Error> {
        let wait = timeout_for(req);
        let stream = UnixStream::connect(socket_path())
            .await
            .map_err(Error::Io)?;
        let (rx, mut tx) = stream.into_split();
        write_request(&mut tx, req).await?;
        drop(tx);

        let mut answer = String::new();
        let read = timeout(wait, BufReader::new(rx).read_line(&mut answer))
            .await
            .map_err(|_| Error::Timeout(wait))?
            .map_err(Error::Io)?;
        if read == 0 {
            return Err(Error::Closed);
        }
        serde_json::from_str(answer.trim()).map_err(|e| Error::Malformed(e.to_string()))
    }

    /// Subscribe to the event stream. The caller reads the lines: the
    /// acknowledgement, then the current state, then one event each.
    pub async fn watch(role: WatchRole) -> Result<Lines<BufReader<OwnedReadHalf>>, Error> {
        let stream = UnixStream::connect(socket_path())
            .await
            .map_err(Error::Io)?;
        let (rx, mut tx) = stream.into_split();
        write_request(&mut tx, &Request::Watch { role }).await?;
        // Nothing is sent on a watch connection after this, and the
        // daemon reads the half to notice a hangup.
        drop(tx);
        Ok(BufReader::new(rx).lines())
    }

    async fn write_request<W>(tx: &mut W, req: &Request) -> Result<(), Error>
    where
        W: AsyncWriteExt + Unpin,
    {
        let mut line = serde_json::to_vec(req).expect("a request serialises");
        line.push(b'\n');
        timeout(WRITE_TIMEOUT, tx.write_all(&line))
            .await
            .map_err(|_| Error::Timeout(WRITE_TIMEOUT))?
            .map_err(Error::Io)?;
        timeout(WRITE_TIMEOUT, tx.flush())
            .await
            .map_err(|_| Error::Timeout(WRITE_TIMEOUT))?
            .map_err(Error::Io)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, timeout_for};
    use crate::ipc::Request;
    use std::time::Duration;

    /// `pair` scans for twelve seconds, so the ten every other request
    /// gets ran out every single time.
    #[test]
    fn slow_requests_wait_longer() {
        let pair = timeout_for(&Request::Pair {
            address: None,
            yes: false,
        });
        assert!(pair.as_secs() > 12, "{pair:?}");
        assert!(timeout_for(&Request::Connect) > timeout_for(&Request::Status));
    }

    /// The three ways this can go wrong read as three different
    /// sentences; a timeout used to print as "os error 11".
    #[test]
    fn errors_say_what_happened() {
        assert!(
            Error::Timeout(Duration::from_secs(45))
                .to_string()
                .contains("within 45s")
        );
        assert!(Error::Closed.to_string().contains("closed the connection"));
        assert!(
            Error::Malformed("unknown variant".into())
                .to_string()
                .contains("malformed answer")
        );
    }
}
