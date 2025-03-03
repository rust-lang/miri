//@only-target: freebsd
//@compile-flags: -Zmiri-preemption-rate=0

use std::mem::MaybeUninit;
use std::ptr::{self, addr_of};
use std::sync::atomic::AtomicU32;
use std::time::{Duration, Instant};
use std::{io, thread};

fn wake_nobody() {
    // TODO: _umtx_op does not return how many threads were woken up
    // How do i test this?
}
fn wake_dangling() {
    let futex = Box::new(0);
    let ptr: *const u32 = &*futex;
    drop(futex);

    // Expect error since this is now "unmapped" memory.
    unsafe {
        assert_eq!(
            libc::_umtx_op(
                ptr as *const AtomicU32 as *mut _,
                libc::UMTX_OP_WAKE_PRIVATE,
                0,
                ptr::null_mut::<libc::c_void>(),
                ptr::null_mut::<libc::c_void>(),
            ),
            -1
        );
        assert_eq!(io::Error::last_os_error().raw_os_error().unwrap(), libc::EFAULT);
    }
}

fn wait_wrong_val() {
    let futex: u32 = 123;

    unsafe {
        assert_eq!(
            libc::_umtx_op(
                ptr::from_ref(&futex).cast_mut().cast(),
                libc::UMTX_OP_WAIT_UINT_PRIVATE,
                456,
                ptr::null_mut::<libc::c_void>(),
                ptr::null_mut::<libc::c_void>(),
            ),
            -1
        );
        // man page doesn't document but we set EINVAL for consistency?
        assert_eq!(io::Error::last_os_error().raw_os_error().unwrap(), libc::EINVAL);
    }
}

fn wait_timeout() {
    let start = Instant::now();

    let futex: u32 = 123;

    // Wait for 200ms, with nobody waking us up early
    unsafe {
        assert_eq!(
            libc::_umtx_op(
                ptr::from_ref(&futex).cast_mut().cast(),
                libc::UMTX_OP_WAIT_UINT_PRIVATE,
                123,
                &mut libc::timespec { tv_sec: 0, tv_nsec: 200_000_000 } as *mut _ as *mut _,
                ptr::null_mut::<libc::c_void>(),
            ),
            -1
        );
        // man page doesn't document but we set EINVAL for consistency
        assert_eq!(io::Error::last_os_error().raw_os_error().unwrap(), libc::ETIMEDOUT);
    }

    assert!((200..1000).contains(&start.elapsed().as_millis()));
}
fn wait_absolute_timeout() {
    let start = Instant::now();

    // Get the current monotonic timestamp as timespec.
    let mut timeout = unsafe {
        let mut now: MaybeUninit<libc::timespec> = MaybeUninit::uninit();
        assert_eq!(libc::clock_gettime(libc::CLOCK_MONOTONIC, now.as_mut_ptr()), 0);
        now.assume_init()
    };

    // Add 200ms.
    timeout.tv_nsec += 200_000_000;
    if timeout.tv_nsec > 1_000_000_000 {
        timeout.tv_nsec -= 1_000_000_000;
        timeout.tv_sec += 1;
    }

    // Create umtx_timeout struct
    let umtx_timeout = libc::_umtx_time {
        _timeout: timeout,
        _flags: libc::UMTX_ABSTIME,
        _clockid: libc::CLOCK_MONOTONIC as u32,
    };
    let umtx_timeout_ptr = &umtx_timeout as *const _;
    let umtx_timeout_size = std::mem::size_of_val(&umtx_timeout);

    let futex: u32 = 123;

    // Wait for 200ms from now, with nobody waking us up early.
    unsafe {
        assert_eq!(
            libc::_umtx_op(
                ptr::from_ref(&futex).cast_mut().cast(),
                libc::UMTX_OP_WAIT_UINT_PRIVATE,
                123,
                ptr::without_provenance_mut(umtx_timeout_size),
                umtx_timeout_ptr as *mut _,
            ),
            -1
        );
        // man page doesn't document but we set EINVAL for consistency
        assert_eq!(io::Error::last_os_error().raw_os_error().unwrap(), libc::ETIMEDOUT);
    }
    assert!((200..1000).contains(&start.elapsed().as_millis()));
}

fn wait_wake() {
    let start = Instant::now();

    static mut FUTEX: u32 = 0;

    let t = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        unsafe {
            assert_eq!(
                libc::_umtx_op(
                    addr_of!(FUTEX) as *mut _,
                    libc::UMTX_OP_WAKE_PRIVATE,
                    10, // Wake up 10 threads, but we can't check that we woken up 1.
                    ptr::null_mut::<libc::c_void>(),
                    ptr::null_mut::<libc::c_void>(),
                ),
                0
            );
        }
    });

    unsafe {
        assert_eq!(
            libc::_umtx_op(
                addr_of!(FUTEX) as *mut _,
                libc::UMTX_OP_WAIT_UINT_PRIVATE,
                0, // FUTEX is 0
                ptr::null_mut::<libc::c_void>(),
                ptr::null_mut::<libc::c_void>(),
            ),
            0
        );
    }

    // When running this in stress-gc mode, things can take quite long.
    // So the timeout is 3000 ms.
    assert!((200..3000).contains(&start.elapsed().as_millis()));
    t.join().unwrap();
}

fn main() {
    wake_nobody();
    wake_dangling();
    wait_wrong_val();
    wait_timeout();
    wait_absolute_timeout();
    wait_wake();
}
