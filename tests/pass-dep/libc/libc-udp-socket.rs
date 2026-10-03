//@ignore-target: windows # No libc socket on Windows
//@compile-flags: -Zmiri-disable-isolation
//@run-native

#[path = "../../utils/libc.rs"]
mod libc_utils;
#[path = "../../utils/mod.rs"]
mod utils;

#[expect(unused)]
use std::io::ErrorKind;
#[expect(unused)]
use std::time::{Duration, Instant};
#[expect(unused)]
use std::{ptr, slice, thread};

use libc_utils::*;

#[expect(unused)]
const TEST_BYTES: &[u8] = b"these are some test bytes!";

fn main() {
    test_create_close();
    test_create_close_udp();
    test_bind_ipv4();
    test_bind_wrong_address_family();
}

/// Test creating a socket and then closing it afterwards.
fn test_create_close() {
    let sockfd = unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)).unwrap() };

    let flags = unsafe { errno_result(libc::fcntl(sockfd, libc::F_GETFL, 0)).unwrap() };

    // Ensure that socket is initially blocking.
    assert_eq!(flags & libc::O_NONBLOCK, 0);

    unsafe { errno_check(libc::close(sockfd)) };
}

/// Test creating a socket and then closing it afterwards but we explicitly
/// specify that the UDP protocol should be used.
fn test_create_close_udp() {
    let sockfd = unsafe {
        errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, libc::IPPROTO_UDP)).unwrap()
    };

    let flags = unsafe { errno_result(libc::fcntl(sockfd, libc::F_GETFL, 0)).unwrap() };

    // Ensure that socket is initially blocking.
    assert_eq!(flags & libc::O_NONBLOCK, 0);

    unsafe { errno_check(libc::close(sockfd)) };
}

/// Test binding a newly created UDP socket to an IPv4 address.
fn test_bind_ipv4() {
    let sockfd = unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)).unwrap() };
    let addr = net::sock_addr_ipv4(net::IPV4_LOCALHOST, 0);

    unsafe {
        errno_check(libc::bind(
            sockfd,
            (&addr as *const libc::sockaddr_in).cast::<libc::sockaddr>(),
            size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ));
    }
}

/// Test binding a newly created IPv6 UDP socket to an IPv4 address.
fn test_bind_wrong_address_family() {
    let sockfd =
        unsafe { errno_result(libc::socket(libc::AF_INET6, libc::SOCK_DGRAM, 0)).unwrap() };
    let addr = net::sock_addr_ipv4(net::IPV4_LOCALHOST, 0);

    let err = unsafe {
        errno_result(libc::bind(
            sockfd,
            (&addr as *const libc::sockaddr_in).cast::<libc::sockaddr>(),
            size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ))
        .unwrap_err()
    };

    if cfg!(any(target_os = "linux", target_os = "android", target_os = "macos")) {
        assert_eq!(err.raw_os_error(), Some(libc::EINVAL))
    } else {
        assert_eq!(err.raw_os_error(), Some(libc::EAFNOSUPPORT))
    }
}
