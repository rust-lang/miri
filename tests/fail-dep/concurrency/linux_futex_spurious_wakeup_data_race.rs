//@only-target: linux android
// We are making scheduler assumptions here, see the "Scheduling" paragraph below.
// The rate must come after `-Zmiri-deterministic-concurrency`, which resets it to `0`.
//@compile-flags: -Zmiri-deterministic-concurrency -Zmiri-spurious-wakeup-rate=0.01

// Test that a spurious wakeup does not synchronize with a `FUTEX_WAKE` that did not wake us, i.e.,
// that it does not acquire the clock of the futex.
//
// The waiter blocks with bitset `0b01`, and nobody ever wakes that bitset, so the only way it can
// return from `FUTEX_WAIT_BITSET` is a spurious wakeup. Meanwhile, the main thread writes `DATA`
// and then calls `FUTEX_WAKE_BITSET` with bitset `0b10`. That stores the main thread's clock in the
// futex, but wakes nobody. A spurious wakeup is not a synchronization event, so the write to `DATA`
// does not happen-before the waiter's read of `DATA`, and Miri must report a data race. If the
// spurious wakeup were wrongly treated like a real `FUTEX_WAKE` and acquired the clock of the
// futex, the race would be hidden and Miri would accept this program.
//
// Scheduling: the waiter must block before the main thread's `FUTEX_WAKE`, as otherwise the SeqCst
// fences in the futex shims already create a happens-before edge and there is no race; the fixed
// schedule guarantees that. The low rate makes the spurious wakeup almost always come after the
// `FUTEX_WAKE`, so that a wrong clock acquire would actually hide the race. If it comes earlier,
// the write still precedes the read without synchronization, so the race is reported for every
// seed.

use std::ptr::{self, addr_of};
use std::sync::atomic::AtomicU32;
use std::thread;

static FUTEX: AtomicU32 = AtomicU32::new(0);
static mut DATA: i32 = 0;

fn main() {
    let t = thread::spawn(|| unsafe {
        // Nobody wakes this bitset, so we only return on a spurious wakeup.
        assert_eq!(
            libc::syscall(
                libc::SYS_futex,
                addr_of!(FUTEX).cast::<u32>(), // uaddr
                libc::FUTEX_WAIT_BITSET,       // op
                0,                             // val
                ptr::null::<libc::timespec>(),
                ptr::null::<u32>(), // uaddr2
                0b01,               // val3 (bitset)
            ),
            0,
        );
        // The main thread's write does not happen-before this read.
        let _val = DATA; //~ ERROR: Data race detected
    });

    // Let the other thread run until it blocks on the futex.
    thread::yield_now();

    unsafe {
        DATA = 1;
        // This releases our clock into the futex, but wakes nobody, as the bitset does not match
        // the waiter. If the spurious wakeup acquired this clock, there would be no data race.
        assert_eq!(
            libc::syscall(
                libc::SYS_futex,
                addr_of!(FUTEX).cast::<u32>(), // uaddr
                libc::FUTEX_WAKE_BITSET,       // op
                1,                             // val (max wake up count)
                ptr::null::<libc::timespec>(),
                ptr::null::<u32>(), // uaddr2
                0b10,               // val3 (bitset)
            ),
            0,
        );
    }

    // Keep yielding (without synchronizing) until the other thread has done its read.
    while !t.is_finished() {
        thread::yield_now();
    }
    t.join().unwrap();
}
