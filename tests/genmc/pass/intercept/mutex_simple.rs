//@revisions: reps1 reps2 reps3
//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows -Zmiri-genmc-verbose
//@normalize-stderr-test: "Verification took .*s" -> "Verification took [TIME]s"

// Test various features of the `std::sync::Mutex` API with GenMC.
// The test variants use a different number of iterations for the part that increments the counter protected by the mutex.
// More repetitions leads to more possible executions, representing all ways that the threads entering the critical sections can be ordered.
//
// FIXME(genmc): Once the actual implementation of mutexes can be used in GenMC mode and there is a setting to disable Mutex interception: Add test revision without interception.
//
// Miri provides annotations to GenMC for the condition required to unblock a thread blocked on a Mutex lock call.
// This allows massively reduces the number of blocked executions we need to explore (in this test to zero blocked execution).
// We use verbose output to test that there are no blocked executions, only completed executions.

#![no_main]
#![feature(abort_unwind)]

#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::sync::Mutex;

use crate::genmc::*;

const REPS: u64 = if cfg!(reps3) {
    3
} else if cfg!(reps2) {
    2
} else {
    1
};

static LOCK: Mutex<[u64; 32]> = Mutex::new([1234; 32]);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    std::panic::abort_unwind(main_);
    0
}

fn main_() {
    let mut guard = LOCK.lock().unwrap();
    for &v in guard.iter() {
        assert!(v == 1234); // Check that mutex values are initialized correctly
    }
    guard[0] = 0;
    guard[1] = 10;
    assert!(guard[0] == 0 && guard[1] == 10); // Check if changes are accepted

    assert!(LOCK.try_lock().is_err()); // Trying to lock should fail if the lock is already held

    drop(guard); // Dropping the guard should unlock the mutex correctly.
    {
        assert!(LOCK.try_lock().is_ok()); // Trying to lock now should *not* fail since the lock is not held.
    }

    let ids = [
        spawn_pthread_closure(|| {
            for _ in 0..REPS {
                let mut guard = LOCK.lock().unwrap();
                guard[0] += 2;
            }
        }),
        spawn_pthread_closure(|| {
            for _ in 0..REPS {
                let mut guard = LOCK.lock().unwrap();
                guard[0] += 4;
            }
        }),
    ];
    unsafe { join_pthreads(ids) };

    let guard = LOCK.lock().unwrap();
    assert!(guard[0] == REPS * 6); // Due to locking, all increments should be visible in every execution.
    assert!(guard[1] == 10); // All other values should be unchanged.
    for &v in guard.iter().skip(2) {
        assert!(v == 1234);
    }
    drop(guard);
}
