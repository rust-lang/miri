//@ignore-target: windows # No libc socket on Windows
//@ignore-host: windows # Non-blocking connect is not supported on windows
//@compile-flags: -Zmiri-disable-isolation

#![feature(io_error_inprogress)]

#[path = "../../utils/libc.rs"]
mod libc_utils;
use std::io;
#[allow(unused)]
use std::{mem::MaybeUninit, thread, time::Duration};

use libc_utils::*;

fn main() {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "solaris",
        target_os = "illumos"
    ))]
    {
        test_connect_nonblock();
    }
}

// Test that nonblocking TCP client sockets return [`io::ErrorKind::InProgress`] when trying to
/// connect to a server where the connection cannot be established immediately.
///
/// At the moment we can only test this on the targets where passing `SOCK_NONBLOCK` to `socket` is
/// supported as it's currently not supported to set fd blocking mode using `ioctl`.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "solaris",
    target_os = "illumos"
))]
fn test_connect_nonblock() {
    // Create a new non-blocking client socket.
    let sockfd = unsafe {
        errno_result(libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_NONBLOCK, 0))
            .unwrap()
    };
    // Defined in RFC 5737, 192.0.2.0/24 is a "blackhole" address space, meaning
    // the OS won't ever get a SYN ACK back. This is ideal to test whether we get
    // an EINPROGRESS when trying to connect to such an address non-blockingly.
    // See <https://datatracker.ietf.org/doc/html/rfc5737>.
    let addr = net::ipv4_sock_addr([192, 0, 2, 1], 56378);

    let result = unsafe {
        errno_result(libc::connect(
            sockfd,
            (&addr as *const libc::sockaddr_in).cast::<libc::sockaddr>(),
            size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ))
    };
    let err = result.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InProgress)
}
