//@ignore-target-windows: only very limited libc on Windows
//@compile-flags: -Zmiri-disable-isolation
#![feature(io_error_more)]
#![feature(pointer_is_aligned_to)]
#![feature(strict_provenance)]

use std::mem::transmute;

/// Tests whether each thread has its own `__errno_location`.
fn test_thread_local_errno() {
    #[cfg(any(target_os = "illumos", target_os = "solaris"))]
    use libc::___errno as __errno_location;
    #[cfg(target_os = "android")]
    use libc::__errno as __errno_location;
    #[cfg(target_os = "linux")]
    use libc::__errno_location;
    #[cfg(any(target_os = "freebsd", target_os = "macos"))]
    use libc::__error as __errno_location;

    unsafe {
        *__errno_location() = 0xBEEF;
        std::thread::spawn(|| {
            assert_eq!(*__errno_location(), 0);
            *__errno_location() = 0xBAD1DEA;
            assert_eq!(*__errno_location(), 0xBAD1DEA);
        })
        .join()
        .unwrap();
        assert_eq!(*__errno_location(), 0xBEEF);
    }
}

fn test_environ() {
    // Just a smoke test for now, checking that the extern static exists.
    extern "C" {
        static mut environ: *const *const libc::c_char;
    }

    unsafe {
        let mut e = environ;
        // Iterate to the end.
        while !(*e).is_null() {
            e = e.add(1);
        }
    }
}

#[cfg(target_os = "linux")]
fn test_sigrt() {
    let min = libc::SIGRTMIN();
    let max = libc::SIGRTMAX();

    // "The Linux kernel supports a range of 33 different real-time
    // signals, numbered 32 to 64"
    assert!(min >= 32);
    assert!(max >= 32);
    assert!(min <= 64);
    assert!(max <= 64);

    // "POSIX.1-2001 requires that an implementation support at least
    // _POSIX_RTSIG_MAX (8) real-time signals."
    assert!(min < max);
    assert!(max - min >= 8)
}

#[cfg(target_os = "linux")]
fn test_affinity() {
    use libc::{cpu_set_t, sched_getaffinity, sched_setaffinity};

    // If pid is zero, then the calling thread is used.
    let pid = 0;

    // Safety: valid value for this type
    let mut cpuset: cpu_set_t = unsafe { core::mem::MaybeUninit::zeroed().assume_init() };

    // now let's properly query the cpuset
    let err = unsafe { sched_getaffinity(pid, core::mem::size_of::<cpu_set_t>(), &mut cpuset) };
    assert_eq!(err, 0);

    assert!(unsafe { libc::CPU_ISSET(0, &cpuset) });

    // assumes `-Zmiri-num-cpus` is the default of 1
    assert!(unsafe { !libc::CPU_ISSET(1, &cpuset) });
    assert!(unsafe { !libc::CPU_ISSET(42, &cpuset) });

    // configure cpu 1
    unsafe { libc::CPU_SET(1, &mut cpuset) };

    let err = unsafe { sched_setaffinity(pid, core::mem::size_of::<cpu_set_t>(), &mut cpuset) };
    assert_eq!(err, 0);

    let err = unsafe { sched_getaffinity(pid, core::mem::size_of::<cpu_set_t>(), &mut cpuset) };
    assert_eq!(err, 0);

    // cpu one should now be set
    assert!(unsafe { libc::CPU_ISSET(1, &cpuset) });

    std::thread::scope(|spawner| {
        spawner.spawn(|| {
            // Safety: valid value for this type
            let mut cpuset: cpu_set_t = unsafe { core::mem::MaybeUninit::zeroed().assume_init() };

            let err =
                unsafe { sched_getaffinity(pid, core::mem::size_of::<cpu_set_t>(), &mut cpuset) };
            assert_eq!(err, 0);

            // the child inherits its parent's set
            assert!(unsafe { libc::CPU_ISSET(0, &cpuset) });
            assert!(unsafe { libc::CPU_ISSET(1, &cpuset) });

            // configure cpu 42 for the child
            unsafe { libc::CPU_SET(42, &mut cpuset) };
        });
    });

    // the parent's set should be unaffected
    assert!(unsafe { !libc::CPU_ISSET(42, &cpuset) });
}

fn test_dlsym() {
    let addr = unsafe { libc::dlsym(libc::RTLD_DEFAULT, b"notasymbol\0".as_ptr().cast()) };
    assert!(addr as usize == 0);

    let addr = unsafe { libc::dlsym(libc::RTLD_DEFAULT, b"isatty\0".as_ptr().cast()) };
    assert!(addr as usize != 0);
    let isatty: extern "C" fn(i32) -> i32 = unsafe { transmute(addr) };
    assert_eq!(isatty(999), 0);
    let errno = std::io::Error::last_os_error().raw_os_error().unwrap();
    assert_eq!(errno, libc::EBADF);
}

fn main() {
    test_thread_local_errno();
    test_environ();

    test_dlsym();

    #[cfg(target_os = "linux")]
    {
        test_sigrt();
        test_affinity();
    }
}
