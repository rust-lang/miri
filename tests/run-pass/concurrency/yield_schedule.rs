// ignore-windows: Concurrency on Windows is not supported yet.

#![feature(once_cell)]

use std::sync::{Arc, Barrier};
use std::lazy::SyncOnceCell as OnceCell;
use std::thread::spawn;


//Variant of once_cell_does_not_leak_partially_constructed_boxes from matklad/once_cell
// with scope replaced with manual thread joins.
fn once_cell_test1() {
    let n_tries = 1;
    let n_readers = 1;
    let n_writers = 1;
    const MSG: &str = "Hello, World";

    for _ in 0..n_tries {
        let cell: Arc<OnceCell<String>> = Arc::new(OnceCell::new());
        let mut joins = Vec::new();
        for _ in 0..n_readers {
            let cell = cell.clone();
            joins.push(spawn(move || loop {
                if let Some(msg) = cell.get() {
                    assert_eq!(msg, MSG);
                    break;
                }
                //Spin loop - add thread yield for liveness
                std::thread::yield_now();
            }));
        }
        for _ in 0..n_writers {
            let cell = cell.clone();
            joins.push(spawn(move || {
                let _ = cell.set(MSG.to_owned());
            }));
        }
        for join in joins {
            join.join().unwrap();
        }
    }
}


// Variant of get_does_not_block from matklad/once_cell
// with scope replaced with manual thread joins.
fn once_cell_test2() {
    let cell: Arc<OnceCell<String>> = Arc::new(OnceCell::new());
    let barrier = Arc::new(Barrier::new(2));
    let join = {
        let cell = cell.clone();
        let barrier = barrier.clone();
        spawn(move || {
            cell.get_or_init(|| {
                barrier.wait();
                barrier.wait();
                "hello".to_string()
            });
        })
    };
    barrier.wait();
    assert_eq!(cell.get(), None);
    barrier.wait();
    join.join().unwrap();
    assert_eq!(cell.get(), Some(&"hello".to_string()));
}



fn main() {
    once_cell_test1();
    once_cell_test2();
}
