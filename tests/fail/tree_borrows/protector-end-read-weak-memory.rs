//@compile-flags:-Zmiri-deterministic-concurrency -Zmiri-tree-borrows
//@revisions: access noaccess
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;

// A way to send raw pointers across threads.
// Note that when using this in closures will require explicit copying
// `let ptr = ptr;` to force the borrow checker to copy the `Send` wrapper
// instead of just copying the inner `!Send` field.
#[derive(Copy, Clone)]
struct SendPtr(*mut u8);
unsafe impl Send for SendPtr {}

unsafe extern "Rust" {
    safe fn miri_write_to_stdout(bytes: &[u8]);
}

fn miriwrite(data: &str) {
    miri_write_to_stdout(data.as_bytes())
}

fn thread1_inner(x: &mut u8, comm: &Arc<AtomicI32>) {
    miriwrite("Thread 1: protector active!\n");
    *x = 42;
    miriwrite("Thread 1: past write!\n");
    comm.store(1, Ordering::Release);
    miriwrite("Thread 1: past release!\n");
    thread::yield_now();
    if cfg!(access) {
        *x = 43;
    }
    miriwrite("Thread 1: pretend write just happened!\n");
}

fn thread1(x: SendPtr, comm: Arc<AtomicI32>) {
    miriwrite("Thread 1: started!\n");
    let x = x.0;
    thread1_inner(unsafe { &mut *x }, &comm);
    miriwrite("Thread 1: protector finished!\n");
    comm.fetch_add(1, Ordering::Relaxed);
    miriwrite("Thread 1: past relaxed store!\n");
    thread::yield_now();
    miriwrite("Thread 1: terminating!\n");
}

fn thread2(x: SendPtr, comm: Arc<AtomicI32>) {
    miriwrite("Thread 2: started!\n");
    let x = x.0;
    loop {
        miriwrite("Thread 2: about to load_acquire!\n");
        let v = comm.load(Ordering::Acquire);
        miriwrite(&format!("Thread 2: load_acquire -> {v}!\n"));
        if v >= 2 {
            break;
        }
        thread::yield_now();
    }
    miriwrite("Thread 2: about to load non-atomically!\n");
    let xv = unsafe { *x }; //~ ERROR: /(Data race detected between \(1\) non-atomic write on thread `unnamed-[0-9]+` and \(2\) non-atomic read on thread `unnamed-[0-9]+`|read access through .* is forbidden)/
    miriwrite(&format!("Thread 2: read {xv}!\n"));
    miriwrite("Thread 2: terminating!\n");
}

fn main() {
    let commr1 = Arc::new(AtomicI32::new(0));
    let mut x = 0;
    let xptr = SendPtr(&raw mut x);
    let xptr2 = xptr;
    let commr2 = commr1.clone();
    let t2 = thread::spawn(move || thread2(xptr2, commr2));
    let t1 = thread::spawn(move || thread1(xptr, commr1));
    t1.join().unwrap();
    t2.join().unwrap();
}
