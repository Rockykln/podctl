//! Raw L2CAP socket via libc. No external bluetooth crate; the kernel
//! does the work, we just speak the right sockaddr.

use std::ffi::c_void;
use std::io::{self, Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

const AF_BLUETOOTH: i32 = 31;
const SOCK_SEQPACKET: i32 = 5;
/// Without this every process the daemon starts — `bluetoothctl`,
/// `dbus-send`, `pactl` — inherits the open AAP channel.
const SOCK_CLOEXEC: i32 = 0o2000000;
const BTPROTO_L2CAP: i32 = 0;
const BDADDR_BREDR: u8 = 0x00;
/// Connect without blocking, so the attempt can be given a deadline
/// instead of waiting on the kernel's own (tens of seconds, and not
/// cancellable).
const SOCK_NONBLOCK: i32 = 0o4000;
const SOL_SOCKET: i32 = 1;
const SO_ERROR: i32 = 4;
const SO_SNDTIMEO: i32 = 21;
const SO_RCVTIMEO: i32 = 20;
const POLLOUT: i16 = 0x004;
const F_GETFL: i32 = 3;
const F_SETFL: i32 = 4;
const O_NONBLOCK: i32 = 0o4000;
const EINPROGRESS: i32 = 115;
/// How often the wait for a connect looks at the cancel flag.
const CONNECT_SLICE: Duration = Duration::from_millis(200);

#[repr(C)]
struct Pollfd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[repr(C)]
struct Timeval {
    tv_sec: i64,
    tv_usec: i64,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
struct SockaddrL2 {
    l2_family: u16,
    l2_psm: u16,
    l2_bdaddr: [u8; 6],
    l2_cid: u16,
    l2_bdaddr_type: u8,
}

unsafe extern "C" {
    fn socket(domain: i32, ty: i32, protocol: i32) -> i32;
    fn connect(fd: i32, addr: *const SockaddrL2, len: u32) -> i32;
    // `c_void` rather than `u8`: rustc checks these two against the
    // signatures the standard library links against and warns on a
    // mismatch.
    fn poll(fds: *mut Pollfd, nfds: u64, timeout: i32) -> i32;
    fn getsockopt(fd: i32, level: i32, name: i32, val: *mut c_void, len: *mut u32) -> i32;
    fn setsockopt(fd: i32, level: i32, name: i32, val: *const c_void, len: u32) -> i32;
    fn fcntl(fd: i32, cmd: i32, arg: i32) -> i32;
    #[link_name = "read"]
    fn libc_read(fd: i32, buf: *mut c_void, count: usize) -> isize;
    #[link_name = "write"]
    fn libc_write(fd: i32, buf: *const c_void, count: usize) -> isize;
    fn shutdown(fd: i32, how: i32) -> i32;
    fn __errno_location() -> *mut i32;
}

fn last_err() -> io::Error {
    io::Error::from_raw_os_error(unsafe { *__errno_location() })
}

pub struct L2capStream {
    fd: OwnedFd,
}

impl L2capStream {
    /// `mac` is colon-separated, six hex bytes (e.g. "AA:BB:CC:DD:EE:FF").
    ///
    /// The attempt gives up after `timeout` and asks `cancel` roughly
    /// five times a second whether to stop early. Without that, a pair
    /// of buds that is out of range holds the caller for the kernel's
    /// own timeout — tens of seconds, during which the daemon's poll
    /// loop stands still and notices neither a disconnect nor a
    /// shutdown. Reads and writes get the same timeout, so neither can
    /// block for good either.
    pub fn connect(
        mac: &str,
        psm: u16,
        timeout: Duration,
        cancel: &dyn Fn() -> bool,
    ) -> io::Result<Self> {
        let bdaddr = parse_mac(mac).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("bad MAC '{mac}'"))
        })?;

        let fd = unsafe {
            socket(
                AF_BLUETOOTH,
                SOCK_SEQPACKET | SOCK_CLOEXEC | SOCK_NONBLOCK,
                BTPROTO_L2CAP,
            )
        };
        if fd < 0 {
            return Err(last_err());
        }
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };

        let addr = SockaddrL2 {
            l2_family: AF_BLUETOOTH as u16,
            l2_psm: psm,
            l2_bdaddr: bdaddr,
            l2_cid: 0,
            l2_bdaddr_type: BDADDR_BREDR,
        };

        let rc = unsafe {
            connect(
                owned.as_raw_fd(),
                &addr,
                std::mem::size_of::<SockaddrL2>() as u32,
            )
        };
        if rc < 0 {
            let e = last_err();
            if e.raw_os_error() != Some(EINPROGRESS) {
                return Err(e);
            }
            wait_connected(owned.as_raw_fd(), timeout, cancel)?;
        }

        set_blocking(owned.as_raw_fd())?;
        set_timeout(owned.as_raw_fd(), SO_SNDTIMEO, timeout)?;
        set_timeout(owned.as_raw_fd(), SO_RCVTIMEO, timeout)?;
        Ok(Self { fd: owned })
    }

    pub fn shutdown_both(&self) -> io::Result<()> {
        let rc = unsafe { shutdown(self.fd.as_raw_fd(), 2) };
        if rc < 0 { Err(last_err()) } else { Ok(()) }
    }

    /// Send a single L2CAP packet. Safe to call from any thread, even
    /// concurrently with `Read::read` on the same socket — the kernel
    /// serialises sendmsg/recvmsg per fd.
    pub fn send(&self, buf: &[u8]) -> io::Result<()> {
        let n = unsafe { libc_write(self.fd.as_raw_fd(), buf.as_ptr().cast(), buf.len()) };
        if n < 0 {
            return Err(last_err());
        }
        if (n as usize) != buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!("short L2CAP write: {n} of {} bytes", buf.len()),
            ));
        }
        Ok(())
    }
}

impl AsRawFd for L2capStream {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

/// Wait for a connect in progress. `Ok(())` once the socket is
/// writable and carries no error.
fn wait_connected(fd: RawFd, timeout: Duration, cancel: &dyn Fn() -> bool) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if cancel() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "L2CAP connect"));
        }
        let mut pfd = Pollfd {
            fd,
            events: POLLOUT,
            revents: 0,
        };
        let slice = left.min(CONNECT_SLICE).as_millis() as i32;
        let rc = unsafe { poll(&mut pfd, 1, slice) };
        if rc < 0 {
            let e = last_err();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if rc == 0 {
            continue;
        }
        let mut err: i32 = 0;
        let mut len = std::mem::size_of::<i32>() as u32;
        let rc = unsafe {
            getsockopt(
                fd,
                SOL_SOCKET,
                SO_ERROR,
                (&raw mut err).cast::<c_void>(),
                &raw mut len,
            )
        };
        if rc < 0 {
            return Err(last_err());
        }
        if err != 0 {
            return Err(io::Error::from_raw_os_error(err));
        }
        return Ok(());
    }
}

fn set_blocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { fcntl(fd, F_GETFL, 0) };
    if flags < 0 {
        return Err(last_err());
    }
    if unsafe { fcntl(fd, F_SETFL, flags & !O_NONBLOCK) } < 0 {
        return Err(last_err());
    }
    Ok(())
}

fn set_timeout(fd: RawFd, which: i32, timeout: Duration) -> io::Result<()> {
    let tv = Timeval {
        tv_sec: timeout.as_secs() as i64,
        tv_usec: i64::from(timeout.subsec_micros()),
    };
    let rc = unsafe {
        setsockopt(
            fd,
            SOL_SOCKET,
            which,
            (&raw const tv).cast::<c_void>(),
            std::mem::size_of::<Timeval>() as u32,
        )
    };
    if rc < 0 {
        return Err(last_err());
    }
    Ok(())
}

impl FromRawFd for L2capStream {
    /// Takes ownership of `fd`, which must be a connected socket.
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        }
    }
}

impl IntoRawFd for L2capStream {
    fn into_raw_fd(self) -> RawFd {
        self.fd.into_raw_fd()
    }
}

impl Read for L2capStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = unsafe { libc_read(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            Err(last_err())
        } else {
            Ok(n as usize)
        }
    }
}

impl Write for L2capStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = unsafe { libc_write(self.fd.as_raw_fd(), buf.as_ptr().cast(), buf.len()) };
        if n < 0 {
            Err(last_err())
        } else {
            Ok(n as usize)
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Parses "AA:BB:CC:DD:EE:FF" → [FF, EE, DD, CC, BB, AA] (sockaddr_l2 wants
/// the address LSB-first).
fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let mut out = [0u8; 6];
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    for (i, part) in parts.iter().enumerate() {
        out[5 - i] = u8::from_str_radix(part, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{Duration, Instant, L2capStream, SO_RCVTIMEO, parse_mac, set_timeout};
    use std::io::Read;
    use std::os::unix::io::{AsRawFd, FromRawFd};

    #[test]
    fn mac_reverses() {
        assert_eq!(
            parse_mac("AA:BB:CC:DD:EE:FF"),
            Some([0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA])
        );
        assert_eq!(
            parse_mac("aa:bb:cc:dd:ee:ff"),
            Some([0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA])
        );
        assert_eq!(parse_mac("not-a-mac"), None);
        assert_eq!(parse_mac("AA:BB:CC:DD:EE"), None);
    }

    unsafe extern "C" {
        fn socketpair(domain: i32, ty: i32, protocol: i32, sv: *mut i32) -> i32;
        fn close(fd: i32) -> i32;
    }

    /// A read on a link that has gone quiet used to block with no way
    /// out: the thread sat there, the cancel flag went unread and the
    /// task could not be reaped.
    #[test]
    fn a_read_gives_up_instead_of_blocking() {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { socketpair(1, 1, 0, fds.as_mut_ptr()) }, 0);
        let mut stream = unsafe { L2capStream::from_raw_fd(fds[0]) };
        set_timeout(stream.as_raw_fd(), SO_RCVTIMEO, Duration::from_millis(80)).expect("timeout");

        let started = Instant::now();
        let mut buf = [0u8; 8];
        let err = stream.read(&mut buf).expect_err("nothing was sent");
        assert!(
            matches!(
                err.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
            "{err:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2), "came back late");
        unsafe { close(fds[1]) };
    }
}
