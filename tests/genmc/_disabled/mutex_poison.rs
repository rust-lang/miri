//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows

#![no_main]
#![feature(abort_unwind)]

// #[path = "../../../utils/genmc.rs"]
// mod genmc;

// use std::ffi::c_void;
use std::sync::{LockResult, Mutex};

// use crate::genmc::*;

static LOCK: Mutex<u64> = Mutex::new(0);

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    std::panic::abort_unwind(main_);
    0
}

fn main_() {
    // let _ids = unsafe { create_pthreads_no_params( [thread_1, thread_2]) };
    // unsafe { join_pthreads(ids) };

    let handle1 = std::thread::spawn(|| {
        // let _err = std::panic::catch_unwind(|| {
        let mut guard = LOCK.lock().unwrap();
        *guard = 0xDEADBEEF;
        panic!();
        // });
        // drop(_err);
    });

    let handle2 = std::thread::spawn(|| {
        // std::thread::sleep(std::time::Duration::from_millis(10)); // Let thread1 run first
        match LOCK.lock() {
            LockResult::Ok(mut value) => *value = 1234,
            LockResult::Err(mut poison) => {
                **poison.get_mut() = 42;
            }
        }
    });

    handle1.join().unwrap();
    handle2.join().unwrap();

    // // Depending on the thread interleaving, the mutex might be poisoned.
    // match LOCK.lock() {
    //     LockResult::Ok(value) => assert!(*value == 1234 || *value == 42),
    //     LockResult::Err(_poison) => {}
    // }
}

// // extern "C" fn thread_1(_: *mut c_void) -> *mut c_void {
// fn thread_1() {
//     let _err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
//         let mut guard = LOCK.lock().unwrap();
//         // Pretend whatever causes the crash fills the mutex with garbage values.
//         *guard = 0xDEADBEEF;
//         panic!();
//     }));
//     Box::leak(Box::new(_err));
//     std::ptr::null_mut()
// }

// // extern "C" fn thread_2(_: *mut c_void) -> *mut c_void {
// fn thread_2() {
//     match LOCK.lock() {
//         LockResult::Ok(mut value) => *value = 1234,
//         LockResult::Err(mut poison) => {
//             **poison.get_mut() = 42;
//         }
//     }
//     std::ptr::null_mut()
// }
