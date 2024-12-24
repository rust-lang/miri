//@ignore-target: windows # No libc socketpair on Windows
//@compile-flags: -Zmiri-preemption-rate=0

use std::thread;

fn main() {
    let mut fds = [-1, -1];
    let res = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) };
    assert_eq!(res, 0);
    let arr1: [u8; 212992] = [1; 212992];
    // Exhaust the space in the buffer so the subsequent write will block.
    let res = unsafe { libc::write(fds[0], arr1.as_ptr() as *const libc::c_void, 212992) };
    assert_eq!(res, 212992);
    let thread1 = thread::spawn(move || {
        let data = "abc".as_bytes().as_ptr();
        // The write below will be blocked because the buffer is already full.
        let res = unsafe { libc::write(fds[0], data as *const libc::c_void, 3) };
        assert_eq!(res, 3);
    });
    let thread2 = thread::spawn(move || {
        // Close the socketpair fd while thread1 is blocking on it.
        assert_eq!(unsafe { libc::close(fds[0]) }, 0);
        // Unblock thread1 by freeing up some space.
        let mut buf: [u8; 3] = [0; 3];
        let res = unsafe { libc::read(fds[1], buf.as_mut_ptr().cast(), buf.len() as libc::size_t) };
        assert_eq!(res, 3);
        assert_eq!(buf, [1, 1, 1]);
    });
    thread1.join().unwrap();
    thread2.join().unwrap();
}
