//@only-target: linux
//@compile-flags: -Zmiri-preemption-rate=0

use std::thread;

/// Test the behaviour of a thread being blocked on an eventfd, unblocked, and then
/// get blocked again.

// The expected execution is
// 1. Thread 1 blocks.
// 2. Thread 2 blocks.
// 3. Main thread unblocks thread 1 and thread 2.
// 4. Either thread 1 or thread 2 writes u64::MAX.
// 5. The next `write` deadlock.

// TODO: better synchronisation instead of depending on thread::yield now.
// TODO: write similar test for eventfd read block.
fn main() {
    // eventfd write will block when EFD_NONBLOCK flag is clear
    // and the addition caused counter to exceed u64::MAX - 1.
    let flags = libc::EFD_CLOEXEC;
    let fd = unsafe { libc::eventfd(0, flags) };
    // Write u64 - 1, so the all subsequent write will block.
    let sized_8_data: [u8; 8] = (u64::MAX - 1).to_ne_bytes();
    let res: i64 = unsafe {
        libc::write(fd, sized_8_data.as_ptr() as *const libc::c_void, 8).try_into().unwrap()
    };
    assert_eq!(res, 8);

    let thread1 = thread::spawn(move || {
        let sized_8_data = (u64::MAX - 1).to_ne_bytes();
        // Write u64::MAX - 1, so the all subsequent write will block.
        let res: i64 = unsafe {
            libc::write(fd, sized_8_data.as_ptr() as *const libc::c_void, 8).try_into().unwrap()
        };
        // Make sure that write is successful.
        assert_eq!(res, 8);
    });

    let thread2 = thread::spawn(move || {
        let sized_8_data = (u64::MAX - 1).to_ne_bytes();
        // Write 1 to the counter, this will block.
        let res: i64 = unsafe {
            libc::write(fd, sized_8_data.as_ptr() as *const libc::c_void, 8).try_into().unwrap()
        };
        // Make sure that write is successful.
        assert_eq!(res, 8);
    });
    let mut buf: [u8; 8] = [0; 8];
    thread::yield_now();
    // This will unblock previously blocked eventfd read.
    let res: i64 = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), 8).try_into().unwrap() };
    // read returns number of bytes has been read, which is always 8.
    assert_eq!(res, 8);
    let counter = u64::from_ne_bytes(buf);
    assert_eq!(counter, (u64::MAX - 1));
    thread1.join().unwrap();
    thread2.join().unwrap();
}
