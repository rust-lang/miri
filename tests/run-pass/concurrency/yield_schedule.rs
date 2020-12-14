// ignore-windows: Concurrency on Windows is not supported yet.
// compile-flags: -Zmiri-disable-isolation

#![feature(once_cell)]

use std::sync::{Arc, Barrier, Mutex, RwLock, Condvar};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::lazy::SyncOnceCell as OnceCell;
use std::thread::spawn;
use std::time::Duration;

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

fn yield_with_mutex() {
    let shared = Arc::new(Mutex::new(0usize));
    let s1 = shared.clone();
    let s_guard = shared.lock().unwrap();
    let j1 = spawn(move || {
        let mut a_guard = loop {
            // yield loop for try-lock.
            if let Ok(guard) = s1.try_lock() {
                break guard
            }else{
                std::thread::yield_now();
            }
        };
        *a_guard = 2;
    });

    // Dropping after yield will only terminate
    // if wake from blocking is implemented.
    std::thread::yield_now();
    drop(s_guard);
    j1.join().unwrap();
}

fn yield_with_rwlock_write() {
    let shared = Arc::new(RwLock::new(0usize));
    let s1 = shared.clone();
    let s_guard = shared.write().unwrap();
    let j1 = spawn(move || {
        let mut a_guard = loop {
            // yield loop for try-lock.
            if let Ok(guard) = s1.try_write() {
                break guard
            }else{
                std::thread::yield_now();
            }
        };
        *a_guard = 2;
    });

    // Dropping after yield will only terminate
    // if wake from blocking is implemented.
    std::thread::yield_now();
    drop(s_guard);
    j1.join().unwrap();
}

fn yield_with_rwlock_read() {
    let shared = Arc::new(RwLock::new(0usize));
    let s1 = shared.clone();
    let s_guard = shared.write().unwrap();
    let j1 = spawn(move || {
        let _a_guard = loop {
            // yield loop for try-lock.
            if let Ok(guard) = s1.try_read() {
                break guard
            }else{
                std::thread::yield_now();
            }
        };
    });

    // Dropping after yield will only terminate
    // if wake from blocking is implemented.
    std::thread::yield_now();
    drop(s_guard);
    j1.join().unwrap();
}

fn yield_with_condvar() {
    let shared = Arc::new((Condvar::new(), Mutex::new(())));
    let s1 = shared.clone();
    let j1 = spawn(move || {
        let mut lock = s1.1.lock().unwrap();
        loop {
            match s1.0.wait_timeout(lock, Duration::from_secs(0)) {
                Ok(_) => break,
                Err(err) => {
                    lock = err.into_inner().0;
                    std::thread::yield_now();
                }
            }
        }
    });

    // Signal after yield yield will only terminate
    // if wake from blocking is implemented.
    std::thread::yield_now();
    shared.0.notify_one();
    j1.join().unwrap();
}


fn print_yield_counters() {
    let shared = Arc::new(AtomicUsize::new(0usize));
    let make_new = || {
        let shared = shared.clone();
        move || {
            let mut array = [0; 10];
            for i in 0..10 {
                array[i] = shared.fetch_add(1, Ordering::SeqCst);
                std::thread::yield_now();
            }
            array
        }
    };
    let j1 = spawn(make_new());
    let j2 = spawn(make_new());
    let j3 = spawn(make_new());
    let j4 = spawn(make_new());
    println!("Interleave Yield");
    println!("Thread 1 = {:?}", j1.join().unwrap());
    println!("Thread 2 = {:?}", j2.join().unwrap());
    println!("Thread 3 = {:?}", j3.join().unwrap());
    println!("Thread 4 = {:?}", j4.join().unwrap());
}

fn spin_loop() {
    static FLAG: AtomicUsize = AtomicUsize::new(0);
    let fun = || {
        while FLAG.load(Ordering::Acquire) == 0 {
            // spin and wait
            // Note: the thread yield or spin loop hint
            // is required for termination, otherwise
            // this will run forever.
            std::sync::atomic::spin_loop_hint();
        }
    };
    let j1 = spawn(fun);
    let j2 = spawn(fun);
    let j3 = spawn(fun);
    let j4 = spawn(fun);
    std::thread::yield_now();
    FLAG.store(1, Ordering::Release);
    j1.join().unwrap();
    j2.join().unwrap();
    j3.join().unwrap();
    j4.join().unwrap();
}

fn main() {
    once_cell_test1();
    once_cell_test2();
    yield_with_mutex();
    yield_with_rwlock_write();
    yield_with_rwlock_read();
    yield_with_condvar();
    print_yield_counters();
    spin_loop();
}
