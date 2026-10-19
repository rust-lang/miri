//@ignore-target: windows  # No libc socket on Windows
//@compile-flags: -Zmiri-disable-isolation
//@run-native

#[path = "../../utils/libc.rs"]
mod libc_utils;

use std::io::ErrorKind;
use std::time::Duration;

use libc_utils::*;

const TEST_BYTES: &[u8] = b"these are some test bytes!";

fn main() {
    test_send_peek_recv_nonblock();
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "solaris",
        target_os = "illumos"
    ))]
    test_send_recv_dontwait();
    test_write_read_nonblock();
    test_send_not_connected();
    test_send_message_too_big();
    test_recv_discard_excess();
}

/// Test sending bytes into an UDP socket and peeking and receiving them from another UDP socket
/// without blocking.
fn test_send_peek_recv_nonblock() {
    let (rx_sockfd, addr) = net::make_local_ipv4(libc::SOCK_DGRAM).unwrap();
    let tx_sockfd =
        unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)).unwrap() };

    unsafe {
        // Change both sockets to be non-blocking.
        errno_check(libc::fcntl(rx_sockfd, libc::F_SETFL, libc::O_NONBLOCK));
        errno_check(libc::fcntl(tx_sockfd, libc::F_SETFL, libc::O_NONBLOCK));
    }

    // Set the default destination address of the transmitting socket.
    net::connect_ipv4(tx_sockfd, addr).unwrap();

    let mut buffer = [0; TEST_BYTES.len()];

    // Receiving from a socket when no data is available is not possible without blocking.
    let err = unsafe {
        errno_result(libc::recv(rx_sockfd, buffer.as_mut_ptr().cast(), buffer.len(), 0))
            .unwrap_err()
    };
    assert_eq!(err.kind(), ErrorKind::WouldBlock);

    // Write `TEST_BYTES` into the transmitting socket.
    unsafe {
        errno_result(libc::send(tx_sockfd, TEST_BYTES.as_ptr().cast(), TEST_BYTES.len(), 0))
            .unwrap()
    };

    // Peek `TEST_BYTES` from the receiving socket.
    unsafe {
        libc_utils::read_exact_generic(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            libc_utils::RetryAfter(Duration::from_millis(10)),
            |buf, count| libc::recv(rx_sockfd, buf, count, libc::MSG_PEEK),
        )
        .unwrap()
    };
    assert_eq!(&buffer, TEST_BYTES);

    // The peek shouldn't have removed the data from the socket; we should
    // thus be able to read it again.

    let mut buffer = [0; TEST_BYTES.len()];

    // Read `TEST_BYTES` from the receiving socket.
    unsafe {
        libc_utils::read_exact_generic(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            libc_utils::RetryAfter(Duration::from_millis(10)),
            |buf, count| libc::recv(rx_sockfd, buf, count, 0),
        )
        .unwrap()
    };
    assert_eq!(&buffer, TEST_BYTES);
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "solaris",
    target_os = "illumos"
))]
/// Test sending bytes into an UDP socket and receiving them from another UDP socket without blocking.
/// In this case, the sockets are blocking sockets, however, all `recv` and `send` operations have
/// the `libc::MSG_DONTWAIT` flag set which means that those operations should be performed in a
/// non-blocking manner.
fn test_send_recv_dontwait() {
    let (rx_sockfd, addr) = net::make_local_ipv4(libc::SOCK_DGRAM).unwrap();
    let tx_sockfd =
        unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)).unwrap() };

    // Set the default destination address of the transmitting socket.
    net::connect_ipv4(tx_sockfd, addr).unwrap();

    let mut buffer = [0; TEST_BYTES.len()];

    // Write `TEST_BYTES` into the transmitting socket.
    unsafe {
        errno_result(libc::send(
            tx_sockfd,
            TEST_BYTES.as_ptr().cast(),
            TEST_BYTES.len(),
            libc::MSG_DONTWAIT,
        ))
        .unwrap()
    };

    // Read `TEST_BYTES` from the receiving socket.
    unsafe {
        libc_utils::read_exact_generic(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            libc_utils::RetryAfter(Duration::from_millis(10)),
            |buf, count| libc::recv(rx_sockfd, buf, count, libc::MSG_DONTWAIT),
        )
        .unwrap()
    };
    assert_eq!(&buffer, TEST_BYTES);
}

/// Test writing bytes into an UDP socket and reading them from another UDP socket without
/// blocking.
fn test_write_read_nonblock() {
    let (rx_sockfd, addr) = net::make_local_ipv4(libc::SOCK_DGRAM).unwrap();
    let tx_sockfd =
        unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)).unwrap() };

    unsafe {
        // Change both sockets to be non-blocking.
        errno_check(libc::fcntl(rx_sockfd, libc::F_SETFL, libc::O_NONBLOCK));
        errno_check(libc::fcntl(tx_sockfd, libc::F_SETFL, libc::O_NONBLOCK));
    }

    // Set the default destination address of the transmitting socket.
    net::connect_ipv4(tx_sockfd, addr).unwrap();

    let mut buffer = [0; TEST_BYTES.len()];

    // Reading from a socket when no data is available is not possible without blocking.
    let err = unsafe {
        errno_result(libc::read(rx_sockfd, buffer.as_mut_ptr().cast(), buffer.len())).unwrap_err()
    };
    assert_eq!(err.kind(), ErrorKind::WouldBlock);

    // Write `TEST_BYTES` into the transmitting socket.
    libc_utils::write_all(tx_sockfd, TEST_BYTES).unwrap();

    // Read `TEST_BYTES` from the receiving socket.
    unsafe {
        libc_utils::read_exact_generic(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            libc_utils::RetryAfter(Duration::from_millis(10)),
            |buf, count| libc::read(rx_sockfd, buf, count),
        )
        .unwrap()
    };
    assert_eq!(&buffer, TEST_BYTES);
}

/// Test sending bytes into a UDP socket with `send` but there hasn't been set a
/// default destination using the `connect` function. This should fail with EDESTADDRREQ.
fn test_send_not_connected() {
    let sockfd = unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)).unwrap() };

    unsafe {
        // Change the socket to be non-blocking.
        errno_check(libc::fcntl(sockfd, libc::F_SETFL, libc::O_NONBLOCK));
    }

    // Attempt to write `TEST_BYTES` into the socket.
    let err = unsafe {
        errno_result(libc::send(sockfd, TEST_BYTES.as_ptr().cast(), TEST_BYTES.len(), 0))
            .unwrap_err()
    };
    // The `send` should fail since the socket has no default destination address set.
    assert_eq!(err.raw_os_error(), Some(libc::EDESTADDRREQ));
}

/// Test sending a UDP message which doesn't fit into a single UDP datagram.
/// The `send` operation should fail with EMSGSIZE.
fn test_send_message_too_big() {
    let (_rx_sockfd, addr) = net::make_local_ipv4(libc::SOCK_DGRAM).unwrap();
    let tx_sockfd =
        unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)).unwrap() };

    unsafe {
        // Change the transmitting socket to be non-blocking.
        errno_check(libc::fcntl(tx_sockfd, libc::F_SETFL, libc::O_NONBLOCK));
    }

    // Set the default destination address of the transmitting socket.
    net::connect_ipv4(tx_sockfd, addr).unwrap();

    // The maximum UDP packet size is `u16::MAX` which is less than 70'000 bytes.
    let buffer = [1u8; 70_000];

    let err = unsafe {
        errno_result(libc::send(tx_sockfd, buffer.as_ptr().cast(), buffer.len(), 0)).unwrap_err()
    };
    // The `send` should fail with EMSGSIZE since the message doesn't fit into a UDP packet.
    assert_eq!(err.raw_os_error(), Some(libc::EMSGSIZE));
}

/// Test that reading a partial message discards the remaining bytes of the message
/// i.e. they cannot be read by a subsequent `recv`.
fn test_recv_discard_excess() {
    let (rx_sockfd, addr) = net::make_local_ipv4(libc::SOCK_DGRAM).unwrap();
    let tx_sockfd =
        unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)).unwrap() };

    unsafe {
        // Change both sockets to be non-blocking.
        errno_check(libc::fcntl(rx_sockfd, libc::F_SETFL, libc::O_NONBLOCK));
        errno_check(libc::fcntl(tx_sockfd, libc::F_SETFL, libc::O_NONBLOCK));
    }

    // Set the default destination address of the transmitting socket.
    net::connect_ipv4(tx_sockfd, addr).unwrap();

    let mut buffer = [0u8; TEST_BYTES.len() / 2];

    // Write `TEST_BYTES` into the transmitting socket.
    libc_utils::write_all(tx_sockfd, TEST_BYTES).unwrap();

    // Read `TEST_BYTES` partially from the receiving socket.
    unsafe {
        libc_utils::read_exact_generic(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            libc_utils::RetryAfter(Duration::from_millis(10)),
            |buf, count| libc::recv(rx_sockfd, buf, count, 0),
        )
        .unwrap()
    };
    assert_eq!(&buffer, &TEST_BYTES[0..buffer.len()]);

    // The bytes from the message which didn't fit into the buffer are discarded.
    // The read buffer should thus now be empty.
    let err = unsafe {
        errno_result(libc::recv(rx_sockfd, buffer.as_mut_ptr().cast(), buffer.len(), 0))
            .unwrap_err()
    };
    assert_eq!(err.kind(), ErrorKind::WouldBlock);
}
