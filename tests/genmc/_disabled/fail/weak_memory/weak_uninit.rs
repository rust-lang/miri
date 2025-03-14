//@compile-flags: -Zmiri-genmc

// NOTE: Disabled due to incomplete uninitialized memory support in Miri-GenMC mode.

// Tests showing weak memory behaviours are exhibited. All tests
// return true when the desired behaviour is seen.
// This is scheduler and pseudo-RNG dependent, so each test is
// run multiple times until one try returns true.
// Spurious failure is possible, if you are really unlucky with
// the RNG and always read the latest value from the store buffer.

#![no_main]

#[path = "../../../utils-dep/mod.rs"]
mod utils_dep;

use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::sync::atomic::*;

use crate::utils_dep::*;

#[allow(dead_code)]
#[derive(Copy, Clone)]
struct EvilSend<T>(pub T);

unsafe impl<T> Send for EvilSend<T> {}
unsafe impl<T> Sync for EvilSend<T> {}

static mut F: MaybeUninit<usize> = MaybeUninit::uninit();

extern "C" fn thread_1(_value: *mut c_void) -> *mut c_void {
    unsafe {
        let x = AtomicUsize::from_ptr(&raw mut F as *mut usize);
        x.store(1, Ordering::Relaxed);
        std::ptr::null_mut()
    }
}

extern "C" fn thread_2(_value: *mut c_void) -> *mut c_void {
    unsafe {
        let x = AtomicUsize::from_ptr(&raw mut F as *mut usize);
        x.load(Ordering::Relaxed); //~ERROR: using uninitialized data
        std::ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    // Unlike with the non-GenMC version of this test, we should only need 1 iteration to detect the bug:
    unsafe {
        let ids = create_pthreads_no_params([thread_1, thread_2]);
        join_pthreads(ids);
    }

    0
}
