//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows
//@revisions: seqcst_rel acqrel relaxed

// This test is the equivalent to the litmus "pass" test `2w2w_3sc_rel1`.
// Here we test different atomic orderings that should all allow an execution where (X, Y) == (1, 1) at the end.
//
// Miri without GenMC is unable to produce this program execution, even with -Zmiri-many-seeds.

#![no_main]

#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::{self, *};

use crate::genmc::{join_pthreads, spawn_pthread_closure};

static X: AtomicU64 = AtomicU64::new(0);
static Y: AtomicU64 = AtomicU64::new(0);

#[cfg(seqcst_rel)]
const STORE_ORD_3: Ordering = SeqCst;
#[cfg(seqcst_rel)]
const STORE_ORD_1: Ordering = Release;

#[cfg(acqrel)]
const STORE_ORD_3: Ordering = Release;
#[cfg(acqrel)]
const STORE_ORD_1: Ordering = Release;

#[cfg(not(any(acqrel, seqcst_rel)))]
const STORE_ORD_3: Ordering = Relaxed;
#[cfg(not(any(acqrel, seqcst_rel)))]
const STORE_ORD_1: Ordering = Relaxed;

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    unsafe {
        let ids = [
            spawn_pthread_closure(|| {
                X.store(1, STORE_ORD_3);
                Y.store(2, STORE_ORD_3);
            }),
            spawn_pthread_closure(|| {
                Y.store(1, STORE_ORD_1);
                X.store(2, STORE_ORD_3);
            }),
        ];
        // Join so we can read the final values.
        join_pthreads(ids);

        let result = (X.load(Relaxed), Y.load(Relaxed));
        if result == (1, 1) {
            std::hint::unreachable_unchecked(); //~ ERROR: entering unreachable code
        }

        0
    }
}
