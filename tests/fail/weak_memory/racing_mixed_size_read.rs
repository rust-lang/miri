// We want to control preemption here.
//@compile-flags: -Zmiri-preemption-rate=0

use std::sync::atomic::Ordering::*;
use std::sync::atomic::{AtomicU16, AtomicU32};
use std::thread::spawn;

fn static_atomic(val: u32) -> &'static AtomicU32 {
    let ret = Box::leak(Box::new(AtomicU32::new(val)));
    ret
}

fn split_u32_ptr(dword: *const u32) -> *const [u16; 2] {
    unsafe { std::mem::transmute::<*const u32, *const [u16; 2]>(dword) }
}

// Racing mixed size reads may cause two loads to read-from
// the same store but observe different values, which doesn't make
// sense under the formal model so we forbade this.
pub fn main() {
    let x = static_atomic(0);

    let j1 = spawn(move || {
        x.load(Relaxed);
    });

    let j2 = spawn(move || {
        let x_ptr = x as *const AtomicU32 as *const u32;
        let x_split = split_u32_ptr(x_ptr);
        unsafe {
            let hi = x_split as *const u16 as *const AtomicU16;
            (*hi).load(Relaxed); //~ ERROR: imperfectly overlapping
        }
    });

    j1.join().unwrap();
    j2.join().unwrap();
}
