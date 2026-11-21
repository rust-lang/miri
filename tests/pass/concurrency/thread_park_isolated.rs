//@ignore-target: apple # park_timeout on macOS uses the system clock
// This test checks the exact sleep time, so Miri's spurious wakeups must stay disabled: they
// would make `park_timeout` return early.
//@compile-flags: -Zmiri-spurious-wakeup-rate=0
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    let start = Instant::now();

    thread::park_timeout(Duration::from_millis(100));

    // Thanks to deterministic execution, this will wait *exactly* 100ms, plus the time for the surrounding code.
    assert!((100..110).contains(&start.elapsed().as_millis()), "{}", start.elapsed().as_millis());
}
