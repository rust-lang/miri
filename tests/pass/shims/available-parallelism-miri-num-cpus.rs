//@revisions: num_1024 num_256
//@[num_1024]compile-flags: -Zmiri-num-cpus=1024
//@[num_256]compile-flags: -Zmiri-num-cpus=256

use std::num::NonZero;
use std::thread::available_parallelism;

fn main() {
    check();
}

#[cfg(num_1024)]
fn check() {
    #[cfg(not(target_os = "freebsd"))]
    assert_eq!(available_parallelism().unwrap(), NonZero::new(1024).unwrap());

    // FIXME: When the stdlib compat version of FreeBSD is bumped to 14 or higher, change this test back to only testing for
    // 1024 CPUs. This way we don't need these revisions and cfgs.
    #[cfg(target_os = "freebsd")]
    assert_eq!(available_parallelism().unwrap(), NonZero::new(256).unwrap());
}

#[cfg(num_256)]
fn check() {
    assert_eq!(available_parallelism().unwrap(), NonZero::new(256).unwrap());
}
