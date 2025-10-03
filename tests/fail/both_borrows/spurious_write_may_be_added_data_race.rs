//@revisions: stack tree
//@[tree]compile-flags: -Zmiri-tree-borrows-strong
use std::sync::{Arc, Barrier};

#[derive(Copy, Clone)]
struct SendPtr(*const u32);

unsafe impl Send for SendPtr {}

type IdxBarrier = (usize, Arc<Barrier>);

// Barriers to enforce the interleaving.
// This macro expects `synchronized!(thread, msg)` where `thread` is a `IdxBarrier`,
// and `msg` is the message to be displayed when the thread reaches this point in the execution.
macro_rules! synchronized {
    ($thread:expr, $msg:expr) => {{
        let (thread_id, barrier) = &$thread;
        eprintln!("Thread {} executing: {}", thread_id, $msg);
        barrier.wait();
    }};
}

fn may_insert_spurious_write(_x: &mut u32, b: IdxBarrier) {
    synchronized!(b, "after enter");
    synchronized!(b, "before exit");
}

fn main() {
    let target = &mut 42;
    let target_alias = &*target;
    let target_alias_ptr = SendPtr(target_alias as *const _);

    let barrier = Arc::new(Barrier::new(2));
    let bx = (1, Arc::clone(&barrier));
    let by = (2, Arc::clone(&barrier));

    let join_handle = std::thread::spawn(move || {
        synchronized!(bx, "before read");
        let ptr = target_alias_ptr;
        // now `target_alias` is invalid
        let _val = unsafe { *ptr.0 };
        //~[stack]^ ERROR: /read access .* tag does not exist in the borrow stack/
        //~[tree]| ERROR: /read access through .* is forbidden/
        synchronized!(bx, "after read");
    });

    may_insert_spurious_write(target, by);
    let _ = join_handle.join();
}
