//! Raw L2CAP socket via libc. No external bluetooth crate; the kernel
//! does the work, we just speak the right sockaddr.

use std::ffi::c_void;
use std::io::{self, Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

const AF_BLUETOOTH: i32 = 31;
const SOCK_SEQPACKET: i32 = 5;
/// Without this every process the daemon starts — `bluetoothctl`,
/// `dbus-send`, `pactl` — inherits the open AAP channel.
const SOCK_CLOEXEC: i32 = 0o2000000;
const BTPROTO_L2CAP: i32 = 0;
const BDADDR_BREDR: u8 = 0x00;

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
    pub fn connect(mac: &str, psm: u16) -> io::Result<Self> {
        let bdaddr = parse_mac(mac).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("bad MAC '{mac}'"))
        })?;

        let fd = unsafe { socket(AF_BLUETOOTH, SOCK_SEQPACKET | SOCK_CLOEXEC, BTPROTO_L2CAP) };
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
            return Err(last_err());
        }
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
    use super::parse_mac;

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
}
