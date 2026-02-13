//@compile-flags: -Zmiri-disable-isolation
//@only-target: linux android illumos
use std::io::Read;

fn main() {
    let _ = match std::fs::File::open("/proc/self/cmdline") {
        Ok(mut f) => {
            let mut buf = Vec::new();
            let _ = f.read_to_end(&mut buf);
        }
        Err(_) => {}
    };
    ();
}
