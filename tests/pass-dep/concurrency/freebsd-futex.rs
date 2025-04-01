//@only-target: freebsd
//@compile-flags: -Zmiri-preemption-rate=0 -Zmiri-disable-isolation

use std::mem::{self, MaybeUninit};
use std::ptr::{self, addr_of};
use std::sync::atomic::AtomicU32;
use std::time::{Duration, Instant};
use std::{io, thread};

fn wake_nobody() {
    // Current thread waits on futex
    // New thread wakes up 0 threads waiting on that futex
    // Current thread should time out
    static mut FUTEX: u32 = 0;

    let waker = thread::spawn(|| {
        thread::sleep(Duration::from_millis(200));

        unsafe {
            assert_eq!(
                libc::_umtx_op(
                    addr_of!(FUTEX) as *mut _,
                    libc::UMTX_OP_WAKE_PRIVATE,
                    0, // wake up 0 waiters
                    ptr::null_mut::<libc::c_void>(),
                    ptr::null_mut::<libc::c_void>(),
                ),
                0
            );
        }
    });
    let mut timeout = libc::timespec { tv_sec: 0, tv_nsec: 400_000_000 };
    let timeout_size_arg =
        ptr::without_provenance_mut::<libc::c_void>(mem::size_of::<libc::timespec>());
    unsafe {
        assert_eq!(
            libc::_umtx_op(
                addr_of!(FUTEX) as *mut _,
                libc::UMTX_OP_WAIT_UINT_PRIVATE,
                0,
                timeout_size_arg,
                &mut timeout as *mut _ as _,
            ),
            -1
        );
        // main thread did not get woken up, so it timed out
        assert_eq!(io::Error::last_os_error().raw_os_error().unwrap(), libc::ETIMEDOUT);
    }

    waker.join().unwrap();
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

    // Wait with a wrong value just returns 0
    unsafe {
        assert_eq!(
            libc::_umtx_op(
                ptr::from_ref(&futex).cast_mut().cast(),
                libc::UMTX_OP_WAIT_UINT_PRIVATE,
                456,
                ptr::null_mut::<libc::c_void>(),
                ptr::null_mut::<libc::c_void>(),
            ),
            0
        );
    }
}

fn wait_relative_timeout() {
    fn without_timespec() {
        let start = Instant::now();

        let futex: u32 = 123;

        let mut timeout = libc::timespec { tv_sec: 0, tv_nsec: 200_000_000 };
        let timeout_size_arg =
            ptr::without_provenance_mut::<libc::c_void>(mem::size_of::<libc::timespec>());
        // Wait for 200ms, with nobody waking us up early
        unsafe {
            assert_eq!(
                libc::_umtx_op(
                    ptr::from_ref(&futex).cast_mut().cast(),
                    libc::UMTX_OP_WAIT_UINT_PRIVATE,
                    123,
                    timeout_size_arg,
                    &mut timeout as *mut _ as _,
                ),
                -1
            );
            assert_eq!(io::Error::last_os_error().raw_os_error().unwrap(), libc::ETIMEDOUT);
        }

        assert!((200..1000).contains(&start.elapsed().as_millis()));
    }

    fn with_timespec() {
        let futex: u32 = 123;
        let mut timeout = libc::_umtx_time {
            _timeout: libc::timespec { tv_sec: 0, tv_nsec: 200_000_000 },
            _flags: 0,
            _clockid: libc::CLOCK_MONOTONIC as u32,
        };
        let timeout_size_arg =
            ptr::without_provenance_mut::<libc::c_void>(mem::size_of::<libc::_umtx_time>());

        let start = Instant::now();

        // Wait for 200ms, with nobody waking us up early
        unsafe {
            assert_eq!(
                libc::_umtx_op(
                    ptr::from_ref(&futex).cast_mut().cast(),
                    libc::UMTX_OP_WAIT_UINT_PRIVATE,
                    123,
                    timeout_size_arg,
                    &mut timeout as *mut _ as _,
                ),
                -1
            );
            assert_eq!(io::Error::last_os_error().raw_os_error().unwrap(), libc::ETIMEDOUT);
        }
        assert!((200..1000).contains(&start.elapsed().as_millis()));
    }

    without_timespec();
    with_timespec();
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

    // Create umtx_timeout struct with that absolute timeout.
    let umtx_timeout = libc::_umtx_time {
        _timeout: timeout,
        _flags: libc::UMTX_ABSTIME,
        _clockid: libc::CLOCK_MONOTONIC as u32,
    };
    let umtx_timeout_ptr = &umtx_timeout as *const _;
    let umtx_timeout_size = ptr::without_provenance_mut(mem::size_of_val(&umtx_timeout));

    let futex: u32 = 123;

    // Wait for 200ms from now, with nobody waking us up early.
    unsafe {
        assert_eq!(
            libc::_umtx_op(
                ptr::from_ref(&futex).cast_mut().cast(),
                libc::UMTX_OP_WAIT_UINT_PRIVATE,
                123,
                umtx_timeout_size,
                umtx_timeout_ptr as *mut _,
            ),
            -1
        );
        assert_eq!(io::Error::last_os_error().raw_os_error().unwrap(), libc::ETIMEDOUT);
    }
    assert!((200..1000).contains(&start.elapsed().as_millis()));
}

fn wait_wake() {
    static mut FUTEX: u32 = 0;

    let t1 = thread::spawn(move || {
        let mut timeout = libc::timespec { tv_sec: 0, tv_nsec: 500_000_000 };
        let timeout_size_arg =
            ptr::without_provenance_mut::<libc::c_void>(mem::size_of::<libc::timespec>());
        unsafe {
            libc::_umtx_op(
                addr_of!(FUTEX) as *mut _,
                libc::UMTX_OP_WAIT_UINT_PRIVATE,
                0, // FUTEX is 0
                timeout_size_arg,
                &mut timeout as *mut _ as _,
            );
            io::Error::last_os_error().raw_os_error().unwrap() == libc::ETIMEDOUT
        }
    });
    let t2 = thread::spawn(move || {
        let mut timeout = libc::timespec { tv_sec: 0, tv_nsec: 500_000_000 };
        let timeout_size_arg =
            ptr::without_provenance_mut::<libc::c_void>(mem::size_of::<libc::timespec>());
        unsafe {
            libc::_umtx_op(
                addr_of!(FUTEX) as *mut _,
                libc::UMTX_OP_WAIT_UINT_PRIVATE,
                0, // FUTEX is 0
                // make sure the threads still exit
                timeout_size_arg,
                &mut timeout as *mut _ as _,
            );
            io::Error::last_os_error().raw_os_error().unwrap() == libc::ETIMEDOUT
        }
    });

    // Wake up 1 thread and make sure the other is still waiting
    thread::sleep(Duration::from_millis(200));
    unsafe {
        assert_eq!(
            libc::_umtx_op(
                addr_of!(FUTEX) as *mut _,
                libc::UMTX_OP_WAKE_PRIVATE,
                1,
                ptr::null_mut::<libc::c_void>(),
                ptr::null_mut::<libc::c_void>(),
            ),
            0
        );
    }
    // Wait a bit more for good measure.
    thread::sleep(Duration::from_millis(100));
    let t1_woke_up = t1.join().unwrap();
    let t2_woke_up = t2.join().unwrap();
    assert!(!(t1_woke_up && t2_woke_up), "Expected only 1 thread to wake up");
}

fn main() {
    wake_nobody();
    wake_dangling();
    wait_wrong_val();
    wait_relative_timeout();
    wait_absolute_timeout();
    wait_wake();
}
