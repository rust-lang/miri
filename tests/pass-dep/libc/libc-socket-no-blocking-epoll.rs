//@only-target: linux android illumos
//@compile-flags: -Zmiri-disable-isolation

#![feature(io_error_inprogress)]

#[path = "../../utils/libc.rs"]
mod libc_utils;

use std::io::ErrorKind;
use std::thread;
use std::time::Duration;

use libc_utils::epoll::*;
use libc_utils::*;

const TEST_BYTES: &[u8] = b"these are some test bytes!";

fn main() {
    test_connect_nonblock();
    test_recv_nonblock();
}

/// Test that connecting to a server socket works when the client
/// socket is non-blocking before the `connect` call.
/// Instead of busy waiting until we no longer get ENOTCONN, we register
/// the client socket to epoll and wait for a WRITABLE event.
fn test_connect_nonblock() {
    let (server_sockfd, addr) = net::make_listener_ipv4().unwrap();
    let client_sockfd =
        unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0)).unwrap() };
    let epfd = errno_result(unsafe { libc::epoll_create1(0) }).unwrap();

    unsafe {
        // Change client socket to be non-blocking.
        errno_check(libc::fcntl(client_sockfd, libc::F_SETFL, libc::O_NONBLOCK));
    }

    // Spawn the server thread.
    let server_thread = thread::spawn(move || {
        net::accept_ipv4(server_sockfd).unwrap();
    });

    // Yield to server thread to ensure that it's currently accepting.
    thread::sleep(Duration::from_millis(10));

    // Non-blocking connects always "fail" with EINPROGRESS.
    let err = net::connect_ipv4(client_sockfd, addr).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InProgress);

    // Add client socket with WRITABLE interest to epoll.
    epoll_ctl_add(epfd, client_sockfd, EPOLLOUT | EPOLLET | libc::EPOLLERR).unwrap();

    check_epoll_wait::<8>(epfd, &[Ev { events: EPOLLOUT, data: client_sockfd }], -1);

    // FIXME: Check SO_ERROR here once we implemented `getsockopt`.

    // We should now be connected and thus getting the peer name should work.
    net::sockname_ipv4(|storage, len| unsafe { libc::getpeername(client_sockfd, storage, len) })
        .unwrap();

    server_thread.join().unwrap();
}

/// Test receiving bytes from a connected stream without blocking.
/// Instead of busy waiting until we no longer receive EWOULDBLOCK when trying to
/// read from the client, we register the client socket to epoll and wait for
/// READABLE events.
fn test_recv_nonblock() {
    let (server_sockfd, addr) = net::make_listener_ipv4().unwrap();
    let client_sockfd =
        unsafe { errno_result(libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0)).unwrap() };
    let epfd = errno_result(unsafe { libc::epoll_create1(0) }).unwrap();

    // Spawn the server thread.
    let server_thread = thread::spawn(move || {
        let (peerfd, _) = net::accept_ipv4(server_sockfd).unwrap();
        // `peerfd` is a blocking socket now. But that's okay, the client still does non-blocking
        // reads/writes.

        // Yield back to client so that it starts receiving before we start sending.
        thread::sleep(Duration::from_millis(10));

        unsafe {
            errno_result(libc_utils::write_all_generic(
                TEST_BYTES.as_ptr().cast(),
                TEST_BYTES.len(),
                libc_utils::NoRetry,
                |buf, count| libc::send(peerfd, buf, count, 0),
            ))
            .unwrap()
        };
    });

    net::connect_ipv4(client_sockfd, addr).unwrap();

    unsafe {
        // Change client socket to be non-blocking.
        errno_check(libc::fcntl(client_sockfd, libc::F_SETFL, libc::O_NONBLOCK));
    }

    // We are connected and the server socket is not writing.

    let mut buffer = [0; TEST_BYTES.len()];
    // Receiving from a socket when the peer is not writing is
    // not possible without blocking.
    let err = unsafe {
        errno_result(libc::recv(client_sockfd, buffer.as_mut_ptr().cast(), buffer.len(), 0))
            .unwrap_err()
    };
    assert_eq!(err.kind(), ErrorKind::WouldBlock);

    // Try to receive bytes from the peer socket without blocking.
    // Since the peer socket might do partial writes, we might need to
    // call `epoll_wait` multiple times until we received everything.

    // Add client socket with READABLE interest to epoll.
    epoll_ctl_add(epfd, client_sockfd, EPOLLIN | EPOLLET | libc::EPOLLERR).unwrap();

    let mut bytes_received = 0;

    while bytes_received != buffer.len() {
        check_epoll_wait::<8>(epfd, &[Ev { events: EPOLLIN, data: client_sockfd }], -1);

        // Receive until we get an EWOULDBLOCK or we read everything.
        // We're only allowed to call `epoll_wait` again once we received
        // an EWOULDBLOCK because otherwise we could deadlock.
        while bytes_received != buffer.len() {
            let read_result = unsafe {
                errno_result(libc::recv(
                    client_sockfd,
                    buffer.as_mut_ptr().byte_add(bytes_received).cast(),
                    buffer.len() - bytes_received,
                    0,
                ))
            };

            match read_result {
                Ok(received) => bytes_received += received as usize,
                Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                Err(err) => panic!("unexpected error whilst receiving: {err}"),
            }
        }
    }

    assert_eq!(&buffer, TEST_BYTES);

    server_thread.join().unwrap();
}
