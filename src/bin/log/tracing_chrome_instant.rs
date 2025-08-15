//! Code in this class was in part inspired by
//! <https://github.com/tikv/minstant/blob/27c9ec5ec90b5b67113a748a4defee0d2519518c/src/tsc_now.rs>.
//! A useful resource is also
//! <https://www.pingcap.com/blog/how-we-trace-a-kv-database-with-less-than-5-percent-performance-impact/>,
//! although this file does not implement TSC synchronization but insteads pins threads to CPUs,
//! since the former is not reliable (i.e. it might lead to non-monotonic time measurements).
//! Another useful resource for future improvements might be measureme's time measurement utils:
//! <https://github.com/rust-lang/measureme/blob/master/measureme/src/counters.rs>.
#![cfg(feature = "tracing")]

/// This alternative `TracingChromeInstant` implementation was made entirely to suit the needs of
/// [crate::log::tracing_chrome], and shouldn't be used for anything else. It featues two functions:
/// - [TracingChromeInstant::setup_for_thread_and_start], which sets up the current thread to do
///   proper time tracking and returns a point in time to use as "t=0", and
/// - [TracingChromeInstant::with_elapsed_micros_subtracting_tracing], which allows
///   obtaining how much time elapsed since [TracingChromeInstant::setup_for_thread_and_start] was
///   called while accounting for (and subtracting) the time spent inside tracing-related functions.
///
/// This measures time using [std::time::Instant], except for x86/x86_64 Linux machines, where
/// [std::time::Instant] is too slow (~1.5us) and thus `rdtsc` is used instead (~5ns).
pub enum TracingChromeInstant {
    WallTime {
        start_instant: std::time::Instant,
    },
    #[cfg(all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64")))]
    Tsc {
        start_tsc: u64,
        tsc_to_microseconds: f64,
    },
}

impl TracingChromeInstant {
    /// Can be thought of as the same as [std::time::Instant::now()], but also does some setup to
    /// make TSC stable in case TSC is available. This is supposed to be called (at most) once per
    /// thread since the thread setup takes a few milliseconds.
    ///
    /// WARNING: If TSC is available, `incremental_thread_id` is used to pick to which CPU to pin
    /// the current thread. It should be an incremental number that starts from 0. Be aware that
    /// the current thread will be restricted to one CPU for the rest of the execution!
    pub fn setup_for_thread_and_start(incremental_thread_id: usize) -> TracingChromeInstant {
        #[cfg(all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64")))]
        if *tsc::IS_TSC_AVAILABLE.get_or_init(|| tsc::is_tsc_available().unwrap_or(false)) {
            // We need to lock this thread to a specific CPU, because CPUs' TSC timers might be out
            // of sync.
            tsc::set_cpu_affinity(incremental_thread_id);

            // Can only use tsc_to_microseconds() and rdtsc() after having set the CPU affinity!
            // We compute tsc_to_microseconds anew for every new thread just in case some CPU core
            // has a different TSC frequency.
            let tsc_to_microseconds = tsc::tsc_to_microseconds();
            let start_tsc = tsc::rdtsc();
            return TracingChromeInstant::Tsc { start_tsc, tsc_to_microseconds };
        }

        let _ = incremental_thread_id; // otherwise we get a warning when the TSC branch is disabled
        TracingChromeInstant::WallTime { start_instant: std::time::Instant::now() }
    }

    /// Calls `f` with the time elapsed in microseconds since this [TracingChromeInstant] was built
    /// by [TracingChromeInstant::setup_for_thread_and_start], while subtracting all time previously
    /// spent executing other `f`s passed to this function. This behavior allows subtracting time
    /// spent in functions that log tracing data (which `f` is supposed to be) from the tracing time
    /// measurements.
    #[inline(always)]
    pub fn with_elapsed_micros_subtracting_tracing<T: Fn(f64)>(&mut self, f: T) {
        match self {
            TracingChromeInstant::WallTime { start_instant } => {
                let instant_before_f = std::time::Instant::now();
                let ts = (instant_before_f - *start_instant).as_nanos() as f64 / 1000.0;
                f(ts);
                *start_instant += std::time::Instant::now() - instant_before_f;
            }
            #[cfg(all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64")))]
            TracingChromeInstant::Tsc { start_tsc, tsc_to_microseconds } => {
                let tsc_before_f = tsc::rdtsc();
                let ts = ((tsc_before_f - *start_tsc) as f64) * (*tsc_to_microseconds);
                f(ts);
                *start_tsc += tsc::rdtsc() - tsc_before_f;
            }
        }
    }
}

#[cfg(all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64")))]
mod tsc {

    pub static IS_TSC_AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

    /// Reads the timestamp-counter register. Will give monotonic answers only when called from the
    /// same thread, because the TSC of different CPUs might be out of sync.
    #[inline(always)]
    pub(super) fn rdtsc() -> u64 {
        #[cfg(target_arch = "x86")]
        use core::arch::x86::{_mm_lfence, _rdtsc};
        #[cfg(target_arch = "x86_64")]
        use core::arch::x86_64::{_mm_lfence, _rdtsc};
        use core::sync::atomic::{Ordering, compiler_fence};

        unsafe {
            _mm_lfence();
            compiler_fence(Ordering::SeqCst);
            let tsc = _rdtsc();
            compiler_fence(Ordering::SeqCst);
            _mm_lfence();
            tsc
        }
    }

    /// Estimates the frequency of the TSC counter by waiting 10ms in a busy loop and
    /// looking at how much the TSC increased in the meantime.
    pub(super) fn tsc_to_microseconds() -> f64 {
        const BUSY_WAIT: std::time::Duration = std::time::Duration::from_millis(10);
        let tsc_start = rdtsc();
        let instant_start = std::time::Instant::now();
        while instant_start.elapsed() < BUSY_WAIT {
            // `thread::sleep()` is not very precise at waking up the program at the right time,
            // so use a busy loop instead.
            core::hint::spin_loop();
        }
        let tsc_end = rdtsc();
        (BUSY_WAIT.as_nanos() as f64) / 1000.0 / ((tsc_end - tsc_start) as f64)
    }

    /// Checks whether the TSC counter is available and runs at a constant rate independently
    /// of CPU frequency.
    pub(super) fn is_tsc_available() -> Option<bool> {
        use std::io::{BufRead, BufReader};

        let cpuinfo = std::fs::File::open("/proc/cpuinfo").ok()?;
        let mut cpuinfo = BufReader::new(cpuinfo);

        let mut buf = String::with_capacity(1024);
        while cpuinfo.read_line(&mut buf).ok()? > 0 {
            if buf.starts_with("flags") {
                return Some(buf.contains("constant_tsc"));
            }
            buf.clear();
        }
        None // EOF
    }

    /// Forces the current thread to run on a single CPU, which ensures the TSC counter is monotonic
    /// (since TSCs of different CPUs might be out-of-sync). `incremental_thread_id` is used to pick
    /// to which CPU to pin the current thread, and should be an incremental number that starts from
    /// 0.
    pub(super) fn set_cpu_affinity(incremental_thread_id: usize) {
        let cpu_id = match std::thread::available_parallelism() {
            Ok(available_parallelism) => incremental_thread_id % available_parallelism,
            _ => {
                eprintln!("Could not determine CPU count to properly set CPU affinity");
                incremental_thread_id
            }
        };

        let mut set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
        unsafe { libc::CPU_SET(cpu_id, &mut set) };

        // Set the current thread's core affinity.
        if unsafe {
            libc::sched_setaffinity(
                0, // Defaults to current thread
                size_of::<libc::cpu_set_t>(),
                &set as *const _,
            )
        } != 0
        {
            panic!("Could not set CPU affinity")
        }
    }
}
