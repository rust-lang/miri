//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows -Zmiri-ignore-leaks

// Test that we can detect writing to an allocation, where the allocation event is not synchronized with the write event.
// We never deallocate the memory, so leak-checks must be disabled.
//
// FIXME(genmc): The error message is currently suboptimal, since it mentions non-allocated memory, instead of pointing towards the missing synchronization with the allocation.

#![no_main]

#[path = "../../../utils/genmc.rs"]
mod genmc;

use std::alloc::{Layout, alloc};
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
                X.store(a, Relaxed); // Relaxed ordering does not synchronize the alloc with the other thread.
            }),
            spawn_pthread_closure(|| {
                let b = X.load(Relaxed);
                if !b.is_null() {
                    *b = 42; //~ ERROR: Undefined Behavior
                }
            }),
        ];
        join_pthreads(ids);
        0
    }
}
