//! Zero-Trust OCap TCP, TLS (`startTls`), and UDP Socket Engine (`cloudflare:sockets`, `Slice.connect`, `node:net`, `node:dgram`).

mod policy_and_mock;
mod tcp;
mod udp;

pub use self::policy_and_mock::register_socket_mock;
pub use self::tcp::{
    tcp_socket_close, tcp_socket_connect, tcp_socket_read, tcp_socket_starttls, tcp_socket_write,
    tcp_socket_write_bytes,
};
pub use self::udp::{
    udp_socket_bind, udp_socket_close, udp_socket_recv, udp_socket_send, udp_socket_send_bytes,
};

pub(crate) use self::policy_and_mock::BOA_SOCKET_MOCKS;

use crate::boa_realm::{DIRTY_SOCKETS, mark_subsystem_dirty};
use std::cell::{Cell, RefCell};
use std::net::IpAddr;
use std::time::Duration;

/// Maximum concurrent open TCP + UDP sockets per isolate execution.
pub const MAX_OPEN_SOCKETS_PER_ISOLATE: usize = 32;

/// Maximum size of a single socket read or write payload (`4 MB`).
pub const MAX_SOCKET_IO_BYTES: usize = 4 * 1024 * 1024;

pub(super) const DEFAULT_SOCKET_TIMEOUT: Duration = Duration::from_secs(5);

thread_local! {
    static NEXT_SOCKET_ID: Cell<u32> = const { Cell::new(1) };
    pub(super) static SOCKET_IO_BUF: RefCell<Box<[u8; 65_536]>> = RefCell::new(Box::new([0u8; 65_536]));
}

/// Clears all open TCP/UDP sockets and mock definitions on the current worker thread.
pub fn clear_thread_socket_state() {
    tcp::clear_tcp_sockets();
    udp::clear_udp_sockets();
    BOA_SOCKET_MOCKS.with(|m| m.borrow_mut().clear());
    NEXT_SOCKET_ID.with(|id| id.set(1));
}

/// Closes all active TCP/UDP sockets without clearing mock definitions (used between event loop resets).
pub(crate) fn close_all_active_sockets() {
    tcp::clear_tcp_sockets();
    udp::clear_udp_sockets();
}

pub(super) fn next_socket_handle_id() -> Result<u32, String> {
    let active_count = tcp::active_tcp_socket_count() + udp::active_udp_socket_count();
    if active_count >= MAX_OPEN_SOCKETS_PER_ISOLATE {
        return Err(format!(
            "SocketError: Maximum concurrent open sockets ({}) exceeded",
            MAX_OPEN_SOCKETS_PER_ISOLATE
        ));
    }
    let id = NEXT_SOCKET_ID.with(|c| {
        let cur = c.get();
        c.set(cur.wrapping_add(1).max(1));
        cur
    });
    mark_subsystem_dirty(DIRTY_SOCKETS);
    Ok(id)
}

/// Classifies an IP string for `net.isIP(input)`: returns `4` for IPv4, `6` for IPv6, `0` otherwise.
#[inline]
pub fn classify_ip_literal(input: &str) -> u8 {
    match input.trim().parse::<IpAddr>() {
        Ok(IpAddr::V4(_)) => 4,
        Ok(IpAddr::V6(_)) => 6,
        Err(_) => 0,
    }
}
