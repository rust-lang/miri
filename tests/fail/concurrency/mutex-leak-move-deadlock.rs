//@error-in-other-file: deadlock
//@normalize-stderr-test: "src/sys/.*\.rs" -> "$$FILE"
//@normalize-stderr-test: "LL \| .*" -> "LL | $$CODE"
//@normalize-stderr-test: "\| +\^+" -> "| ^"
//@normalize-stderr-test: "\n *= note:.*" -> ""
//@normalize-stderr-test: "\n *\d+:.*\n *at .*" -> ""
// On macOS we use chekced pthread mutexes which changes the error
//@normalize-stderr-test: "a thread got stuck here" -> "thread `main` got stuck here"
//@normalize-stderr-test: "a thread deadlocked" -> "the evaluated program deadlocked"
// A spurious wakeup lets the deadlocked thread wake up and block again, which delays the
// deadlock detection (and with rate 1.0 defeats it), so keep spurious wakeups disabled here.
//@compile-flags: -Zmiri-spurious-wakeup-rate=0
use std::mem;
use std::sync::Mutex;

fn main() {
    let m = Mutex::new(0);
    mem::forget(m.lock());
    // Move the lock while it is "held" (really: leaked)
    let m2 = m;
    // Now try to acquire the lock again.
    let _guard = m2.lock();
}
