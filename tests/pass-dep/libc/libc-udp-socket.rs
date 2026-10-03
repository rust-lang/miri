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
    test_getsockname_ipv4();
    test_getsockname_ipv4_random_port();
    test_getsockname_ipv4_unbound();
    test_getsockname_ipv6_unbound();
    test_getpeername_ipv4();
    test_getpeername_ipv6();
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

/// Test `getsockname` on an IPv4 socket which is explicitly bound.
/// It should return the same address as to which the socket was bound to.
fn test_getsockname_ipv4() {
    let sockfd = unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)).unwrap() };
    let addr = net::sock_addr_ipv4(net::IPV4_LOCALHOST, 7754);
    unsafe {
        errno_check(libc::bind(
            sockfd,
            (&addr as *const libc::sockaddr_in).cast::<libc::sockaddr>(),
            size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ));
    }

    let (_, sock_addr) =
        net::sockname_ipv4(|storage, len| unsafe { libc::getsockname(sockfd, storage, len) })
            .unwrap();

    assert_eq!(addr.sin_family, sock_addr.sin_family);
    assert_eq!(addr.sin_port, sock_addr.sin_port);
    assert_eq!(addr.sin_addr.s_addr, sock_addr.sin_addr.s_addr);
}

/// Test `getsockname` on an IPv4 socket which is bound to a socket address
/// with a zero port.
/// It should return the same IP address but with a randomly assigned non-zero
/// port.
fn test_getsockname_ipv4_random_port() {
    let sockfd = unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)).unwrap() };
    // Use zero-port to let the OS choose a free port to bind to.
    let addr = net::sock_addr_ipv4(net::IPV4_LOCALHOST, 0);
    unsafe {
        errno_check(libc::bind(
            sockfd,
            (&addr as *const libc::sockaddr_in).cast::<libc::sockaddr>(),
            size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ));
    }

    let (_, sock_addr) =
        net::sockname_ipv4(|storage, len| unsafe { libc::getsockname(sockfd, storage, len) })
            .unwrap();

    assert_eq!(addr.sin_family, sock_addr.sin_family);
    // The bound port must not be the zero port.
    assert!(sock_addr.sin_port > 0);
    assert_eq!(addr.sin_addr.s_addr, sock_addr.sin_addr.s_addr);
}

/// Test `getsockname` on an IPv4 socket which is not bound.
/// It should return the unspecified IPv4 address with a zero
/// port (i.e., 0.0.0.0:0).
fn test_getsockname_ipv4_unbound() {
    let sockfd = unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)).unwrap() };

    let (_, sock_addr) =
        net::sockname_ipv4(|storage, len| unsafe { libc::getsockname(sockfd, storage, len) })
            .unwrap();

    // Libc representation of an unspecified IPv4 address with zero port.
    let addr = net::sock_addr_ipv4([0, 0, 0, 0], 0);

    assert_eq!(addr.sin_family, sock_addr.sin_family);
    assert_eq!(addr.sin_port, sock_addr.sin_port);
    assert_eq!(addr.sin_addr.s_addr, sock_addr.sin_addr.s_addr);
}

/// Test `getsockname` on an IPv6 socket which is not bound.
/// It should return the unspecified IPv6 address with a zero
/// port (i.e., [::]:0).
fn test_getsockname_ipv6_unbound() {
    let sockfd =
        unsafe { errno_result(libc::socket(libc::AF_INET6, libc::SOCK_DGRAM, 0)).unwrap() };

    let (_, sock_addr) =
        net::sockname_ipv6(|storage, len| unsafe { libc::getsockname(sockfd, storage, len) })
            .unwrap();

    // Libc representation of an unspecified IPv6 address with zero port, flowinfo and scope id.
    let addr = net::sock_addr_full_ipv6(
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        /* port */ 0,
        /* flowinfo */ 0,
        /* scope_id */ 0,
    );

    assert_eq!(addr.sin6_family, sock_addr.sin6_family);
    assert_eq!(addr.sin6_port, sock_addr.sin6_port);
    assert_eq!(addr.sin6_flowinfo, sock_addr.sin6_flowinfo);
    assert_eq!(addr.sin6_scope_id, sock_addr.sin6_scope_id);
    assert_eq!(addr.sin6_addr.s6_addr, sock_addr.sin6_addr.s6_addr);
}

/// Test `getpeername` on an IPv4 socket.
/// For a socket whose default destination has been set using
/// `connect`, this should return the address of the specified
/// default destination. Otherwise, errno should be set to
/// ENOTCONN.
fn test_getpeername_ipv4() {
    let sockfd = unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)).unwrap() };

    // When no default destination has been set using `connect`,
    // `getpeername` should fail and set errno to ENOTCONN.
    let err = net::sockname_ipv4(|storage, len| unsafe { libc::getpeername(sockfd, storage, len) })
        .unwrap_err();
    assert_eq!(err.raw_os_error(), Some(libc::ENOTCONN));

    let addr = net::sock_addr_ipv4(net::IPV4_LOCALHOST, 1234);
    // Explicitly set default destination address.
    net::connect_ipv4(sockfd, addr).unwrap();

    let (_, dest_addr) =
        net::sockname_ipv4(|storage, len| unsafe { libc::getpeername(sockfd, storage, len) })
            .unwrap();

    assert_eq!(addr.sin_family, dest_addr.sin_family);
    assert_eq!(addr.sin_port, dest_addr.sin_port);
    assert_eq!(addr.sin_addr.s_addr, dest_addr.sin_addr.s_addr);
}

/// Test `getpeername` on an IPv6 socket.
/// For a socket whose default destination has been set using
/// `connect`, this should return the address of the specified
/// default destination. Otherwise, errno should be set to
/// ENOTCONN.
fn test_getpeername_ipv6() {
    let sockfd =
        unsafe { errno_result(libc::socket(libc::AF_INET6, libc::SOCK_DGRAM, 0)).unwrap() };

    // When no default destination has been set using `connect`,
    // `getpeername` should fail and set errno to ENOTCONN.
    let err = net::sockname_ipv6(|storage, len| unsafe { libc::getpeername(sockfd, storage, len) })
        .unwrap_err();
    assert_eq!(err.raw_os_error(), Some(libc::ENOTCONN));

    let addr = net::sock_addr_ipv6(net::IPV6_LOCALHOST, 1234);
    // Explicitly set default destination address.
    net::connect_ipv6(sockfd, addr).unwrap();

    let (_, dest_addr) =
        net::sockname_ipv6(|storage, len| unsafe { libc::getpeername(sockfd, storage, len) })
            .unwrap();

    assert_eq!(addr.sin6_family, dest_addr.sin6_family);
    assert_eq!(addr.sin6_port, dest_addr.sin6_port);
    assert_eq!(addr.sin6_flowinfo, dest_addr.sin6_flowinfo);
    assert_eq!(addr.sin6_scope_id, dest_addr.sin6_scope_id);
    assert_eq!(addr.sin6_addr.s6_addr, dest_addr.sin6_addr.s6_addr);
}
