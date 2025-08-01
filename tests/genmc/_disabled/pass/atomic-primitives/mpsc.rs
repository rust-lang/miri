//@compile-flags: -Zmiri-genmc -Zmiri-disable-stacked-borrows
//@revisions: threads1 threads2 threads3

// NOTE: Test disabled due to missing mixed-size access support in GenMC.

// Check that various operations on `std::sync::mpsc` are handled properly in GenMC mode.
// This test is a slightly changed version of the "Shared usage" example in the `mpsc` documentation.

#![no_main]

use std::sync::mpsc::channel;
use std::thread;

const NUM_THREADS: usize = if cfg!(threads3) {
    3
} else if cfg!(threads2) {
    2
} else {
    1
};

#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    // Create a shared channel that can be sent along from many threads
    // where tx is the sending half (tx for transmission), and rx is the receiving
    // half (rx for receiving).
    let (tx, rx) = channel();
    for i in 0..NUM_THREADS {
        let tx = tx.clone();
        thread::spawn(move || {
            tx.send(i).unwrap();
        });
    }

    for _ in 0..NUM_THREADS {
        let j = rx.recv().unwrap();
        assert!(j < NUM_THREADS);
    }

    0
}
