//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// This tests is the test `checks_fail` from loom/test/smoke.rs adapted for Miri-GenMC.
// https://github.com/tokio-rs/loom/blob/dbf32b04bae821c64be44405a0bb72ca08741558/tests/smoke.rs

// This test checks that an incorrect implementation of an incrementing counter is detected.
// The counter behaves wrong if two threads try to increment at the same time (increments can be lost).

#![no_main]

#[cfg(not(any(non_genmc_std, genmc_std)))]
#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::ffi::c_void;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use crate::genmc::{create_pthreads_no_params, join_pthreads};

struct BuggyInc {
    num: AtomicUsize,
}

impl BuggyInc {
    const fn new() -> BuggyInc {
        BuggyInc { num: AtomicUsize::new(0) }
    }

    fn inc(&self) {
        // The bug is here:
        // Another thread can increment `self.num` between the next two lines, which is then overridden by this thread.
        let curr = self.num.load(Acquire);
        self.num.store(curr + 1, Release);
    }
}

static BUGGY_INC: BuggyInc = BuggyInc::new();

extern "C" fn thread_func(_value: *mut c_void) -> *mut c_void {
    BUGGY_INC.inc();
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    let thread_ids = unsafe { create_pthreads_no_params([thread_func; 2]) };

    unsafe { join_pthreads(thread_ids) };

    if 2 != BUGGY_INC.num.load(Relaxed) {
        unsafe { std::hint::unreachable_unchecked() }; //~ ERROR: entering unreachable code
    }

    0
}
