//@ignore-target: windows # File handling is not implemented yet
//@compile-flags: -Zmiri-disable-isolation -Zmiri-preemption-rate=0
use std::thread;

/// By setting O_NONBLOCK flag, the blocked fd will not be woken up.
fn main() {
    let mut fds = [-1, -1];
    let res = unsafe { libc::pipe(fds.as_mut_ptr()) };
    assert_eq!(res, 0);
    let mut buf: [u8; 5] = [0; 5];
    // This will block.
    let _res = unsafe { libc::read(fds[0], buf.as_mut_ptr().cast(), buf.len() as libc::size_t) };
    //~^ ERROR: deadlock: the evaluated program deadlocked
    let _thread1 = thread::spawn(move || {
        // Add O_NONBLOCK flag while pipe is still block on read.
        let new_flag = libc::O_NONBLOCK;
        let res = unsafe { libc::fcntl(fds[0], libc::F_SETFL, new_flag) };
        assert_eq!(res, 0);
    });
}
