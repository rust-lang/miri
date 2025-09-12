//@ revisions: order12 order21
//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows -Zmiri-genmc-verbose
//@normalize-stderr-test: "Verification took .*\n" -> "Verification took [TIME]\n"

// Translated from GenMC's test "litmus/assume-ctrl".
// Shortened quote:
// "An example demonstrating why we should treat assume() statements like
// if statements, in the sense that the former also create control dependencies."
//
// Without this treatment of `assume()`, using a different a different scheduling policies or thread orders may make GenMC miss certain executions.
// We test this by switching the order in which we spawn the threads.

#![no_main]

#[path = "../../../utils/genmc.rs"]
mod genmc;
#[path = "../../../utils/mod.rs"]
mod utils;

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::*;

use crate::genmc::*;
use crate::utils::miri_genmc_verifier_assume;

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    // FIXME(genmc,HACK): remove these initializing writes once Miri-GenMC supports mixed atomic-non-atomic accesses.
    X.store(0, Relaxed);
    Y.store(0, Relaxed);

    unsafe {
        let t0 = || {
            miri_genmc_verifier_assume(2 > Y.load(Relaxed) || Y.load(Relaxed) > 3);
            X.store(1, Relaxed);
        };
        let t1 = || {
            miri_genmc_verifier_assume(X.load(Relaxed) < 3);
            Y.store(3, Relaxed);
            std::sync::atomic::fence(SeqCst);
            Y.store(4, Relaxed);
        };
        // Reverse the order for the second test variant.
        #[cfg(order21)]
        let (t1, t0) = (t0, t1);

        spawn_pthread_closure(t0);
        spawn_pthread_closure(t1);
        0
    }
}
