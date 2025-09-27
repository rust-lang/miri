//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

// Test that we can detect data races between `alloc` and `dealloc`.
//
// FIXME(genmc): The error message is currently suboptimal, since it mentions accessing freed memory, instead of pointing towards the missing synchronization with the allocation event.

#![no_main]

#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::alloc::{Layout, alloc, dealloc};
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::Ordering::*;

use crate::genmc::*;

static X: AtomicPtr<u64> = AtomicPtr::new(std::ptr::null_mut());

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    // FIXME(genmc,HACK): remove this initializing write once Miri-GenMC supports mixed atomic-non-atomic accesses.
    X.store(std::ptr::null_mut(), SeqCst);

    unsafe {
        let ids = [
            spawn_pthread_closure(|| {
                let a: *mut u64 = alloc(Layout::new::<u64>()) as *mut u64;
                X.store(a, Relaxed); // Relaxed ordering does not synchronize the `alloc` with the other thread.
            }),
            spawn_pthread_closure(|| {
                let b = X.load(Relaxed);
                if !b.is_null() {
                    dealloc(b as *mut u8, Layout::new::<u64>()) //~ ERROR: Undefined Behavior
                }
            }),
        ];
        join_pthreads(ids);
        0
    }
}
