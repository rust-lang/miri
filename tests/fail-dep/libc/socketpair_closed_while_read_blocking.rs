//@ignore-target: windows # No libc socketpair on Windows
//@compile-flags: -Zmiri-preemption-rate=0

use std::thread;

fn main() {
    let mut fds = [-1, -1];
    let res = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) };
    assert_eq!(res, 0);
    let thread1 = thread::spawn(move || {
        // Let this thread block on read.
        let mut buf: [u8; 3] = [0; 3];
        let res = unsafe { libc::read(fds[1], buf.as_mut_ptr().cast(), buf.len() as libc::size_t) };
        assert_eq!(res, 3);
        assert_eq!(&buf, "abc".as_bytes());
    });
    let thread2 = thread::spawn(move || {
        // Close the socketpair fd while thread1 is blocking on it.
        assert_eq!(unsafe { libc::close(fds[1]) }, 0);
        let data = "abc".as_bytes().as_ptr();
        let res = unsafe { libc::write(fds[0], data as *const libc::c_void, 3) };
        // This will fail because we can't write anything if the peer_fd is closed.
        assert_eq!(res, -1);
    });
    thread1.join().unwrap();
    thread2.join().unwrap();
}
