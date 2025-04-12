//@ignore-target: windows # File handling is not implemented yet
//@compile-flags: -Zmiri-isolation-error=warn-nobacktrace -Zmiri-preemption-rate=0
//@normalize-stderr-test: "(stat(x)?)" -> "$$STAT"

use std::io::{Error, ErrorKind};
use std::{fs, thread};

fn main() {
    test_fcntl_f_dupfd();
    test_setfl_getfl();
    test_setfl_getfl_threaded();
}

fn test_fcntl_f_dupfd() {
    // test `fcntl(F_DUPFD): should work even with isolation.`
    unsafe {
        assert!(libc::fcntl(1, libc::F_DUPFD, 0) >= 0);
    }

    // test `readlink`
    let mut buf = vec![0; "foo_link.txt".len() + 1];
    unsafe {
        assert_eq!(libc::readlink(c"foo.txt".as_ptr(), buf.as_mut_ptr(), buf.len()), -1);
        assert_eq!(Error::last_os_error().raw_os_error(), Some(libc::EACCES));
    }

    // test `stat`
    let err = fs::metadata("foo.txt").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    // check that it is the right kind of `PermissionDenied`
    assert_eq!(err.raw_os_error(), Some(libc::EACCES));
}

/// Basic test for fcntl's F_SETFL and F_GETFL flag.
fn test_setfl_getfl() {
    // Test fd without O_NONBLOCK flag.
    let mut fds = [-1, -1];
    let res = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) };
    assert_eq!(res, 0);

    let res = unsafe { libc::fcntl(fds[0], libc::F_GETFL) };
    assert_eq!(res, 0);

    // Modify the flag to O_NONBLOCK flag with setfl.
    let new_flag = libc::O_NONBLOCK;
    let res = unsafe { libc::fcntl(fds[0], libc::F_SETFL, new_flag) };
    assert_eq!(res, 0);

    let res = unsafe { libc::fcntl(fds[0], libc::F_GETFL) };
    assert_eq!(res, libc::O_NONBLOCK);
}

/// Test the behaviour of setfl/getfl when a fd is blocking.
/// The expected execution is:
/// 1. Main thread blocks on fds[0] `read`.
/// 2. Thread 1 sets O_NONBLOCK flag on fds[0],
///    checks the value of F_GETFL,
///    then writes to fds[1] to unblock main thread's `read`.
fn test_setfl_getfl_threaded() {
    let mut fds = [-1, -1];
    let res = unsafe { libc::pipe(fds.as_mut_ptr()) };
    assert_eq!(res, 0);
    let mut buf: [u8; 5] = [0; 5];
    let thread1 = thread::spawn(move || {
        // Add O_NONBLOCK flag while pipe is still block on read.
        let new_flag = libc::O_NONBLOCK;
        let res = unsafe { libc::fcntl(fds[0], libc::F_SETFL, new_flag) };
        assert_eq!(res, 0);

        // Check the new flag value while the main thread is still blocked on fds[0].
        let res = unsafe { libc::fcntl(fds[0], libc::F_GETFL) };
        assert_eq!(res, libc::O_NONBLOCK);

        // The write below will unblock the `read` in main thread.
        let data = "abcde".as_bytes().as_ptr();
        let res = unsafe { libc::write(fds[1], data as *const libc::c_void, 5) };
        assert_eq!(res, 5);
    });
    // The `read` below will block.
    let res = unsafe { libc::read(fds[0], buf.as_mut_ptr().cast(), buf.len() as libc::size_t) };
    thread1.join().unwrap();
    assert_eq!(res, 5);
}
