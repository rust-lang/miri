//@ignore-target-windows: only very limited libc on Windows
//@compile-flags: -Zmiri-disable-isolation -Zmiri-num-cpus=4
#![feature(io_error_more)]
#![feature(pointer_is_aligned_to)]
#![feature(strict_provenance)]

use libc::{cpu_set_t, sched_getaffinity, sched_setaffinity};

use std::mem::size_of;

// If pid is zero, then the calling thread is used.
const PID: i32 = 0;

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "android"))]
fn configure_no_cpus() {
    let cpu_count = std::thread::available_parallelism().unwrap().get();

    let mut cpuset: cpu_set_t = unsafe { core::mem::MaybeUninit::zeroed().assume_init() };

    // configuring no CPUs will fail
    let err = unsafe { sched_setaffinity(PID, size_of::<cpu_set_t>(), &cpuset) };
    assert_eq!(err, -1);
    assert_eq!(std::io::Error::last_os_error().kind(), std::io::ErrorKind::InvalidInput);

    // configuring no (physically available) CPUs will fail
    unsafe { libc::CPU_SET(cpu_count, &mut cpuset) };
    let err = unsafe { sched_setaffinity(PID, size_of::<cpu_set_t>(), &cpuset) };
    assert_eq!(err, -1);
    assert_eq!(std::io::Error::last_os_error().kind(), std::io::ErrorKind::InvalidInput);
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "android"))]
fn configure_unavailable_cpu() {
    let cpu_count = std::thread::available_parallelism().unwrap().get();

    // Safety: valid value for this type
    let mut cpuset: cpu_set_t = unsafe { core::mem::MaybeUninit::zeroed().assume_init() };

    let err = unsafe { sched_getaffinity(PID, size_of::<cpu_set_t>(), &mut cpuset) };
    assert_eq!(err, 0);

    // by default, only available CPUs are configured
    for i in 0..cpu_count {
        assert!(unsafe { libc::CPU_ISSET(i, &cpuset) });
    }
    assert!(unsafe { !libc::CPU_ISSET(cpu_count, &cpuset) });

    // configure CPU that we don't have
    unsafe { libc::CPU_SET(cpu_count, &mut cpuset) };

    let err = unsafe { sched_setaffinity(PID, size_of::<cpu_set_t>(), &cpuset) };
    assert_eq!(err, 0);

    let err = unsafe { sched_getaffinity(PID, size_of::<cpu_set_t>(), &mut cpuset) };
    assert_eq!(err, 0);

    // the CPU is not set because it is not available
    assert!(!unsafe { libc::CPU_ISSET(cpu_count, &cpuset) });
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "android"))]
fn lying_about_size() {
    let cpu_count = std::thread::available_parallelism().unwrap().get();

    assert!(cpu_count > 1, "this test cannot do anything interesting with just one thread");

    let mut cpuset: cpu_set_t = unsafe { core::mem::MaybeUninit::zeroed().assume_init() };

    // getting the affinity with insufficient space will fail
    let err = unsafe { sched_getaffinity(PID, 1, &mut cpuset) };
    assert_eq!(err, -1);
    assert_eq!(std::io::Error::last_os_error().kind(), std::io::ErrorKind::InvalidInput);

    // at the start, thread 1 should be set
    let err = unsafe { sched_getaffinity(PID, size_of::<cpu_set_t>(), &mut cpuset) };
    assert_eq!(err, 0);
    assert!(unsafe { libc::CPU_ISSET(1, &cpuset) });

    // make a valid mask
    unsafe { libc::CPU_ZERO(&mut cpuset) };
    unsafe { libc::CPU_SET(0, &mut cpuset) };

    // giving a smaller mask is fine
    let err = unsafe { sched_setaffinity(PID, 8, &cpuset) };
    assert_eq!(err, 0);

    // and actually disables other threads
    let err = unsafe { sched_getaffinity(PID, size_of::<cpu_set_t>(), &mut cpuset) };
    assert_eq!(err, 0);
    assert!(unsafe { !libc::CPU_ISSET(1, &cpuset) });

    // it is important that we reset the cpu mask now for future tests
    for i in 0..cpu_count {
        unsafe { libc::CPU_SET(i, &mut cpuset) };
    }

    // exaggerating the length is also fine (and will not go out of bounds)
    let err = unsafe { sched_setaffinity(PID, size_of::<cpu_set_t>() + 8, &cpuset) };
    assert_eq!(err, 0);
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "android"))]
fn parent_child() {
    let cpu_count = std::thread::available_parallelism().unwrap().get();

    assert!(cpu_count > 1, "this test cannot do anything interesting with just one thread");

    // configure the parent thread to only run only on CPU 0
    let mut parent_cpuset: cpu_set_t = unsafe { core::mem::MaybeUninit::zeroed().assume_init() };
    unsafe { libc::CPU_SET(0, &mut parent_cpuset) };

    let err = unsafe { sched_setaffinity(PID, size_of::<cpu_set_t>(), &parent_cpuset) };
    assert_eq!(err, 0);

    std::thread::scope(|spawner| {
        spawner.spawn(|| {
            let mut cpuset: cpu_set_t = unsafe { core::mem::MaybeUninit::zeroed().assume_init() };

            let err = unsafe { sched_getaffinity(PID, size_of::<cpu_set_t>(), &mut cpuset) };
            assert_eq!(err, 0);

            // the child inherits its parent's set
            assert!(unsafe { libc::CPU_ISSET(0, &cpuset) });
            assert!(unsafe { !libc::CPU_ISSET(1, &cpuset) });

            // configure cpu 1 for the child
            unsafe { libc::CPU_SET(1, &mut cpuset) };
        });
    });

    let err = unsafe { sched_getaffinity(PID, size_of::<cpu_set_t>(), &mut parent_cpuset) };
    assert_eq!(err, 0);

    // the parent's set should be unaffected
    assert!(unsafe { !libc::CPU_ISSET(1, &parent_cpuset) });
}

fn main() {
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "android"))]
    {
        configure_no_cpus();
        configure_unavailable_cpu();
        lying_about_size();
        parent_child();
    }
}
