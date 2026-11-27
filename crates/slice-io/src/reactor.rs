//! Zero-Dependency Cross-Platform Non-Blocking Network Readiness Reactor (`epoll` / `kqueue` / `WSAPoll`).
//!
//! Designed for Slice's thread-per-core share-nothing worker architecture (`#133`):
//! - Linux: direct `epoll_create1` / `epoll_ctl` / `epoll_wait` (`EPOLLIN | EPOLLOUT | EPOLLRDHUP`).
//! - macOS / BSD: direct `kqueue` / `kevent` (`EVFILT_READ` / `EVFILT_WRITE`).
//! - Windows: direct Winsock2 `WSAPoll` (`POLLRDNORM | POLLWRNORM`).
//!
//! Adds zero external runtime crates (`tokio`/`hyper`) and supports 10,000+ concurrent sockets
//! with `O(ready)` event dispatch and compact per-connection state.

use std::io;
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// Socket readiness interest flags for [`NetReactor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interest {
    pub readable: bool,
    pub writable: bool,
}

impl Interest {
    pub const READABLE: Self = Self {
        readable: true,
        writable: false,
    };
    pub const WRITABLE: Self = Self {
        readable: false,
        writable: true,
    };
    pub const READ_WRITE: Self = Self {
        readable: true,
        writable: true,
    };
}

/// Readiness event returned by [`NetReactor::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadyEvent {
    pub token: usize,
    pub readable: bool,
    pub writable: bool,
    pub hangup: bool,
    pub error: bool,
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{Interest, ReadyEvent};
    use std::io;
    use std::os::unix::io::{AsRawFd, RawFd};
    use std::time::Duration;

    pub(super) struct ReactorInner {
        epfd: RawFd,
        scratch: Vec<libc::epoll_event>,
    }

    impl ReactorInner {
        pub(super) fn new() -> io::Result<Self> {
            // SAFETY: epoll_create1 with EPOLL_CLOEXEC takes no pointers and returns a valid fd or -1.
            let epfd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
            if epfd < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                epfd,
                scratch: vec![libc::epoll_event { events: 0, u64: 0 }; 512],
            })
        }

        pub(super) fn backend_name(&self) -> &'static str {
            "epoll"
        }

        fn interest_to_events(interest: Interest) -> u32 {
            let mut ev = libc::EPOLLRDHUP as u32;
            if interest.readable {
                ev |= libc::EPOLLIN as u32;
            }
            if interest.writable {
                ev |= libc::EPOLLOUT as u32;
            }
            ev
        }

        pub(super) fn register_fd(
            &mut self,
            fd: RawFd,
            token: usize,
            interest: Interest,
        ) -> io::Result<()> {
            let mut ev = libc::epoll_event {
                events: Self::interest_to_events(interest),
                u64: token as u64,
            };
            // SAFETY: epfd is a live epoll fd and &mut ev points to an initialized epoll_event.
            let rc = unsafe { libc::epoll_ctl(self.epfd, libc::EPOLL_CTL_ADD, fd, &mut ev) };
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        pub(super) fn reregister_fd(
            &mut self,
            fd: RawFd,
            token: usize,
            interest: Interest,
        ) -> io::Result<()> {
            let mut ev = libc::epoll_event {
                events: Self::interest_to_events(interest),
                u64: token as u64,
            };
            // SAFETY: epfd is a live epoll fd and &mut ev points to an initialized epoll_event.
            let rc = unsafe { libc::epoll_ctl(self.epfd, libc::EPOLL_CTL_MOD, fd, &mut ev) };
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        pub(super) fn deregister_fd(&mut self, fd: RawFd) -> io::Result<()> {
            // SAFETY: epfd is a live epoll fd; EPOLL_CTL_DEL ignores the event pointer on Linux >= 2.6.9.
            let rc = unsafe {
                libc::epoll_ctl(self.epfd, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut())
            };
            if rc < 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() != Some(libc::ENOENT)
                    && err.raw_os_error() != Some(libc::EBADF)
                {
                    return Err(err);
                }
            }
            Ok(())
        }

        pub(super) fn poll(
            &mut self,
            events: &mut Vec<ReadyEvent>,
            timeout: Option<Duration>,
        ) -> io::Result<usize> {
            events.clear();
            let timeout_ms: libc::c_int = match timeout {
                Some(d) => d.as_millis().min(i32::MAX as u128) as libc::c_int,
                None => -1,
            };
            // SAFETY: scratch is a valid mutable slice of epoll_event with length <= i32::MAX.
            let n = unsafe {
                libc::epoll_wait(
                    self.epfd,
                    self.scratch.as_mut_ptr(),
                    self.scratch.len() as libc::c_int,
                    timeout_ms,
                )
            };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    return Ok(0);
                }
                return Err(err);
            }
            let count = n as usize;
            events.reserve(count);
            for ev in &self.scratch[..count] {
                let bits = ev.events;
                events.push(ReadyEvent {
                    token: ev.u64 as usize,
                    readable: (bits & (libc::EPOLLIN as u32)) != 0,
                    writable: (bits & (libc::EPOLLOUT as u32)) != 0,
                    hangup: (bits & ((libc::EPOLLHUP | libc::EPOLLRDHUP) as u32)) != 0,
                    error: (bits & (libc::EPOLLERR as u32)) != 0,
                });
            }
            Ok(count)
        }

        #[inline]
        pub(super) fn fd_of<T: AsRawFd>(s: &T) -> RawFd {
            s.as_raw_fd()
        }
    }

    impl Drop for ReactorInner {
        fn drop(&mut self) {
            // SAFETY: epfd is uniquely owned by ReactorInner and closed once on Drop.
            unsafe {
                libc::close(self.epfd);
            }
        }
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
mod imp {
    use super::{Interest, ReadyEvent};
    use std::io;
    use std::os::unix::io::{AsRawFd, RawFd};
    use std::time::Duration;

    pub(super) struct ReactorInner {
        kq: RawFd,
        scratch: Vec<libc::kevent>,
    }

    impl ReactorInner {
        pub(super) fn new() -> io::Result<Self> {
            // SAFETY: kqueue() takes no arguments and returns a new fd or -1.
            let kq = unsafe { libc::kqueue() };
            if kq < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: Setting FD_CLOEXEC on our owned kqueue descriptor.
            unsafe {
                libc::fcntl(kq, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            // SAFETY: zeroed kevent is valid as output buffer for kevent().
            let zero_ev: libc::kevent = unsafe { std::mem::zeroed() };
            Ok(Self {
                kq,
                scratch: vec![zero_ev; 512],
            })
        }

        pub(super) fn backend_name(&self) -> &'static str {
            "kqueue"
        }

        fn apply_interest(
            &mut self,
            fd: RawFd,
            token: usize,
            interest: Interest,
        ) -> io::Result<()> {
            let mut changes: [libc::kevent; 2] = [
                // SAFETY: zeroed kevent initialized immediately below.
                unsafe { std::mem::zeroed() },
                // SAFETY: zeroed kevent initialized immediately below.
                unsafe { std::mem::zeroed() },
            ];
            changes[0].ident = fd as usize;
            changes[0].filter = libc::EVFILT_READ;
            changes[0].flags = if interest.readable {
                libc::EV_ADD | libc::EV_ENABLE
            } else {
                libc::EV_ADD | libc::EV_DISABLE
            };
            changes[0].udata = token as *mut libc::c_void;

            changes[1].ident = fd as usize;
            changes[1].filter = libc::EVFILT_WRITE;
            changes[1].flags = if interest.writable {
                libc::EV_ADD | libc::EV_ENABLE
            } else {
                libc::EV_ADD | libc::EV_DISABLE
            };
            changes[1].udata = token as *mut libc::c_void;

            // SAFETY: kq is a live kqueue fd and changes has length 2.
            let rc = unsafe {
                libc::kevent(
                    self.kq,
                    changes.as_ptr(),
                    2,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                )
            };
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        pub(super) fn register_fd(
            &mut self,
            fd: RawFd,
            token: usize,
            interest: Interest,
        ) -> io::Result<()> {
            self.apply_interest(fd, token, interest)
        }

        pub(super) fn reregister_fd(
            &mut self,
            fd: RawFd,
            token: usize,
            interest: Interest,
        ) -> io::Result<()> {
            self.apply_interest(fd, token, interest)
        }

        pub(super) fn deregister_fd(&mut self, fd: RawFd) -> io::Result<()> {
            let mut changes: [libc::kevent; 2] = [
                // SAFETY: zeroed kevent initialized immediately below.
                unsafe { std::mem::zeroed() },
                // SAFETY: zeroed kevent initialized immediately below.
                unsafe { std::mem::zeroed() },
            ];
            changes[0].ident = fd as usize;
            changes[0].filter = libc::EVFILT_READ;
            changes[0].flags = libc::EV_DELETE;
            changes[1].ident = fd as usize;
            changes[1].filter = libc::EVFILT_WRITE;
            changes[1].flags = libc::EV_DELETE;
            // SAFETY: best-effort EV_DELETE on owned kqueue fd.
            let _ = unsafe {
                libc::kevent(
                    self.kq,
                    changes.as_ptr(),
                    2,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                )
            };
            Ok(())
        }

        pub(super) fn poll(
            &mut self,
            events: &mut Vec<ReadyEvent>,
            timeout: Option<Duration>,
        ) -> io::Result<usize> {
            events.clear();
            let ts_val = timeout.map(|d| libc::timespec {
                tv_sec: d.as_secs() as libc::time_t,
                tv_nsec: d.subsec_nanos() as libc::c_long,
            });
            let ts_ptr = ts_val
                .as_ref()
                .map_or(std::ptr::null(), |t| t as *const libc::timespec);
            // SAFETY: kq is a valid kqueue fd, scratch is valid for scratch.len() events.
            let n = unsafe {
                libc::kevent(
                    self.kq,
                    std::ptr::null(),
                    0,
                    self.scratch.as_mut_ptr(),
                    self.scratch.len() as libc::c_int,
                    ts_ptr,
                )
            };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    return Ok(0);
                }
                return Err(err);
            }
            let count = n as usize;
            events.reserve(count);
            for ev in &self.scratch[..count] {
                events.push(ReadyEvent {
                    token: ev.udata as usize,
                    readable: ev.filter == libc::EVFILT_READ,
                    writable: ev.filter == libc::EVFILT_WRITE,
                    hangup: (ev.flags & libc::EV_EOF) != 0,
                    error: (ev.flags & libc::EV_ERROR) != 0,
                });
            }
            Ok(count)
        }

        #[inline]
        pub(super) fn fd_of<T: AsRawFd>(s: &T) -> RawFd {
            s.as_raw_fd()
        }
    }

    impl Drop for ReactorInner {
        fn drop(&mut self) {
            // SAFETY: kq is uniquely owned by ReactorInner and closed once on Drop.
            unsafe {
                libc::close(self.kq);
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::{Interest, ReadyEvent};
    use std::io;
    use std::os::windows::io::{AsRawSocket, RawSocket};
    use std::time::Duration;

    const POLLRDNORM: i16 = 0x0100;
    const POLLWRNORM: i16 = 0x0010;
    const POLLERR: i16 = 0x0001;
    const POLLHUP: i16 = 0x0002;
    const POLLNVAL: i16 = 0x0004;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WsaPollFd {
        fd: usize,
        events: i16,
        revents: i16,
    }

    #[link(name = "ws2_32")]
    unsafe extern "system" {
        fn WSAPoll(fdArray: *mut WsaPollFd, fds: u32, timeout: i32) -> i32;
    }

    pub(super) struct ReactorInner {
        fds: Vec<WsaPollFd>,
        tokens: Vec<usize>,
    }

    impl ReactorInner {
        pub(super) fn new() -> io::Result<Self> {
            Ok(Self {
                fds: Vec::with_capacity(64),
                tokens: Vec::with_capacity(64),
            })
        }

        pub(super) fn backend_name(&self) -> &'static str {
            "WSAPoll"
        }

        fn interest_to_events(interest: Interest) -> i16 {
            let mut ev = 0i16;
            if interest.readable {
                ev |= POLLRDNORM;
            }
            if interest.writable {
                ev |= POLLWRNORM;
            }
            ev
        }

        pub(super) fn register_fd(
            &mut self,
            fd: RawSocket,
            token: usize,
            interest: Interest,
        ) -> io::Result<()> {
            let raw = fd as usize;
            if let Some(pos) = self.fds.iter().position(|e| e.fd == raw) {
                self.fds[pos].events = Self::interest_to_events(interest);
                self.fds[pos].revents = 0;
                self.tokens[pos] = token;
            } else {
                self.fds.push(WsaPollFd {
                    fd: raw,
                    events: Self::interest_to_events(interest),
                    revents: 0,
                });
                self.tokens.push(token);
            }
            Ok(())
        }

        pub(super) fn reregister_fd(
            &mut self,
            fd: RawSocket,
            token: usize,
            interest: Interest,
        ) -> io::Result<()> {
            self.register_fd(fd, token, interest)
        }

        pub(super) fn deregister_fd(&mut self, fd: RawSocket) -> io::Result<()> {
            let raw = fd as usize;
            if let Some(pos) = self.fds.iter().position(|e| e.fd == raw) {
                self.fds.swap_remove(pos);
                self.tokens.swap_remove(pos);
            }
            Ok(())
        }

        pub(super) fn poll(
            &mut self,
            events: &mut Vec<ReadyEvent>,
            timeout: Option<Duration>,
        ) -> io::Result<usize> {
            events.clear();
            if self.fds.is_empty() {
                if let Some(d) = timeout
                    && !d.is_zero()
                {
                    std::thread::sleep(d.min(Duration::from_millis(5)));
                }
                return Ok(0);
            }
            for entry in &mut self.fds {
                entry.revents = 0;
            }
            let timeout_ms: i32 = match timeout {
                Some(d) => d.as_millis().min(i32::MAX as u128) as i32,
                None => -1,
            };
            // SAFETY: self.fds is a valid mutable slice of WsaPollFd of length self.fds.len().
            let rc = unsafe { WSAPoll(self.fds.as_mut_ptr(), self.fds.len() as u32, timeout_ms) };
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            if rc == 0 {
                return Ok(0);
            }
            for (idx, entry) in self.fds.iter().enumerate() {
                let rev = entry.revents;
                if rev != 0 {
                    events.push(ReadyEvent {
                        token: self.tokens[idx],
                        readable: (rev & POLLRDNORM) != 0,
                        writable: (rev & POLLWRNORM) != 0,
                        hangup: (rev & POLLHUP) != 0,
                        error: (rev & (POLLERR | POLLNVAL)) != 0,
                    });
                }
            }
            Ok(events.len())
        }

        #[inline]
        pub(super) fn fd_of<T: AsRawSocket>(s: &T) -> RawSocket {
            s.as_raw_socket()
        }
    }
}

/// Share-nothing per-worker non-blocking network readiness reactor.
pub struct NetReactor {
    inner: imp::ReactorInner,
}

impl NetReactor {
    /// Creates a new OS readiness reactor (`epoll` on Linux, `kqueue` on macOS/BSD, `WSAPoll` on Windows).
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            inner: imp::ReactorInner::new()?,
        })
    }

    /// Returns the active OS readiness backend name (`"epoll"`, `"kqueue"`, or `"WSAPoll"`).
    #[inline]
    pub fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }

    /// Registers a non-blocking [`TcpListener`] for readable (accept) readiness events.
    pub fn register_listener(&mut self, listener: &TcpListener, token: usize) -> io::Result<()> {
        self.inner.register_fd(
            imp::ReactorInner::fd_of(listener),
            token,
            Interest::READABLE,
        )
    }

    /// Registers a non-blocking [`TcpStream`] with the given [`Interest`].
    pub fn register_stream(
        &mut self,
        stream: &TcpStream,
        token: usize,
        interest: Interest,
    ) -> io::Result<()> {
        self.inner
            .register_fd(imp::ReactorInner::fd_of(stream), token, interest)
    }

    /// Updates readiness interest for an already-registered [`TcpStream`].
    pub fn reregister_stream(
        &mut self,
        stream: &TcpStream,
        token: usize,
        interest: Interest,
    ) -> io::Result<()> {
        self.inner
            .reregister_fd(imp::ReactorInner::fd_of(stream), token, interest)
    }

    /// Removes a [`TcpStream`] from the reactor before closing it.
    pub fn deregister_stream(&mut self, stream: &TcpStream) -> io::Result<()> {
        self.inner.deregister_fd(imp::ReactorInner::fd_of(stream))
    }

    /// Waits up to `timeout` for socket readiness events and appends them to `events`.
    pub fn poll(
        &mut self,
        events: &mut Vec<ReadyEvent>,
        timeout: Option<Duration>,
    ) -> io::Result<usize> {
        self.inner.poll(events, timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn test_v0150_net_reactor_multiplexes_concurrent_tcp_sockets() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();

        let mut reactor = NetReactor::new().unwrap();
        assert!(!reactor.backend_name().is_empty());
        reactor.register_listener(&listener, 999).unwrap();

        let mut c1 = TcpStream::connect(addr).unwrap();
        let mut c2 = TcpStream::connect(addr).unwrap();
        c1.write_all(b"ping1").unwrap();
        c2.write_all(b"ping2").unwrap();

        let mut events = Vec::new();
        let mut accepted = Vec::new();
        for _ in 0..20 {
            let _ = reactor
                .poll(&mut events, Some(Duration::from_millis(20)))
                .unwrap();
            for ev in &events {
                if ev.token == 999 && ev.readable {
                    while let Ok((s, _)) = listener.accept() {
                        s.set_nonblocking(true).unwrap();
                        let tok = accepted.len();
                        reactor
                            .register_stream(&s, tok, Interest::READABLE)
                            .unwrap();
                        accepted.push(s);
                    }
                }
            }
            if accepted.len() == 2 {
                break;
            }
        }
        assert_eq!(accepted.len(), 2);

        let mut received = 0usize;
        for _ in 0..20 {
            let _ = reactor
                .poll(&mut events, Some(Duration::from_millis(20)))
                .unwrap();
            for ev in &events {
                if ev.token < accepted.len() && ev.readable {
                    let mut buf = [0u8; 16];
                    if let Ok(n) = accepted[ev.token].read(&mut buf)
                        && n > 0
                    {
                        received += n;
                    }
                }
            }
            if received >= 10 {
                break;
            }
        }
        assert_eq!(received, 10);
        for s in &accepted {
            reactor.deregister_stream(s).unwrap();
        }
    }
}
