//@ignore-target-windows: home_dir is not supported on Windows
//@ignore-target-wasi: home_dir is not supported on wasi
//@compile-flags: -Zmiri-disable-isolation
use std::env;

fn main() {
    env::remove_var("HOME"); // make sure we enter the interesting codepath
    #[allow(deprecated)]
    env::home_dir().unwrap();
}
