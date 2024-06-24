//@ignore-target-windows: only very limited libc on Windows
//@compile-flags: -Zmiri-disable-isolation -Zmiri-num-cpus=4

fn main() {
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "android"))]
    {
        use libc::{cpu_set_t, sched_setaffinity};

        use std::mem::size_of;

        // If pid is zero, then the calling thread is used.
        const PID: i32 = 0;

        let mut cpuset: cpu_set_t = unsafe { core::mem::MaybeUninit::zeroed().assume_init() };

        let err = unsafe { sched_setaffinity(PID, size_of::<cpu_set_t>() + 1, &mut cpuset) }; //~ ERROR: memory access failed
        assert_eq!(err, 0);
    }
}
