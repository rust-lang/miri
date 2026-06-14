//@ignore-target: windows # no libc

use std::thread;
use std::time::{Duration, Instant};

#[path = "../../utils/libc.rs"]
mod libc_utils;
use libc_utils::*;

const TEST_BYTES: &[u8] = b"these are some test bytes!";

fn main() {
    test_poll_unblock_with_events();
    test_poll_block_without_events();
    test_poll_readiness_update();
    test_poll_duplicate_fd_interest();
}

/// Test that the `poll` call unblocks when one of the
/// provided interests is fulfilled.
fn test_poll_unblock_with_events() {
    let fd = errno_result(unsafe { libc::eventfd(0, 0) }).unwrap();

    let mut interests = [libc::pollfd { fd, events: libc::POLLIN | libc::POLLOUT, revents: 0 }];
    let ready = unsafe {
        errno_result(libc::poll(interests.as_mut_ptr(), interests.len() as libc::nfds_t, -1))
            .unwrap()
    };
    assert_eq!(ready, 1);
    // Ensure that the correct `revents` has been set.
    assert_eq!(interests[0].revents, libc::POLLOUT);
}

/// Test that the `poll` blocks and returns zero when
/// none of the provided interests get fulfilled.
fn test_poll_block_without_events() {
    let fd = errno_result(unsafe { libc::eventfd(0, 0) }).unwrap();

    let mut interests = [libc::pollfd { fd, events: libc::POLLIN, revents: 0 }];
    let before = Instant::now();
    let ready = unsafe {
        errno_result(libc::poll(interests.as_mut_ptr(), interests.len() as libc::nfds_t, 50))
            .unwrap()
    };
    assert_eq!(ready, 0);
    // Ensure that the `poll` blocked at least for 50ms.
    assert!(Instant::now().duration_since(before) > Duration::from_millis(50))
}

/// Test that the `poll` blocks when the requested interests are not
/// fulfilled at creation. This also tests that the `poll` unblocks
/// once the readiness of a registered fd changes.
fn test_poll_readiness_update() {
    let mut fds = [-1, -1];
    unsafe { errno_check(libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr())) };

    let t1 = thread::spawn(move || {
        unsafe {
            errno_result(libc::write(fds[1], TEST_BYTES.as_ptr().cast(), TEST_BYTES.len())).unwrap()
        }
    });

    let mut interests = [libc::pollfd { fd: fds[0], events: libc::POLLIN, revents: 0 }];
    let ready = unsafe {
        errno_result(libc::poll(interests.as_mut_ptr(), interests.len() as libc::nfds_t, 50))
            .unwrap()
    };
    assert_eq!(ready, 1);
    // Ensure that the correct `revents` has been set.
    assert_eq!(interests[0].revents, libc::POLLIN);

    t1.join().unwrap();
}

/// Test calling `poll` when the same fd is present multiple times in the
/// interest array. This should set the `revents` for both entries in the
/// interest array.
fn test_poll_duplicate_fd_interest() {
    let fd = errno_result(unsafe { libc::eventfd(0, 0) }).unwrap();

    let mut interests = [
        libc::pollfd { fd, events: libc::POLLIN | libc::POLLOUT, revents: 0 },
        libc::pollfd { fd, events: libc::POLLIN | libc::POLLOUT, revents: 0 },
    ];
    let ready = unsafe {
        errno_result(libc::poll(interests.as_mut_ptr(), interests.len() as libc::nfds_t, -1))
            .unwrap()
    };
    assert_eq!(ready, 2);
    // Ensure that both `revents` have been set.
    assert_eq!(interests[0].revents, libc::POLLOUT);
    assert_eq!(interests[1].revents, libc::POLLOUT);
}
