//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

#![no_main]

#[path = "../utils/genmc.rs"]
mod genmc;

use std::ffi::c_void;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering::SeqCst;

use crate::genmc::*;

const T: usize = 2;
const N: usize = 2000;

static V: AtomicI32 = AtomicI32::new(0);
static LOCAL: [AtomicI32; T] = [const { AtomicI32::new(0) }; T];

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    for i in 0..T {
        let arg = i as *mut c_void;
        let _id = unsafe { spawn_pthread(thread_func, arg) };
    }

    // let arg = 0usize as *mut c_void;
    // let _id0 = unsafe { spawn_pthread(thread_func, arg) };
    // let arg = 1usize as *mut c_void;
    // let _id1 = unsafe { spawn_pthread(thread_func, arg) };
    // unsafe { join_pthreads([_id0, _id1])};

    0
}

extern "C" fn thread_func(value: *mut c_void) -> *mut c_void {
    let tid = value as usize;

    for i in 0..N {
        LOCAL[tid].store(i.try_into().unwrap(), SeqCst);
        let _ = LOCAL[tid].load(SeqCst);
    }

    V.store(tid.try_into().unwrap(), SeqCst);
    let _ = V.load(SeqCst);

    std::ptr::null_mut()
}
