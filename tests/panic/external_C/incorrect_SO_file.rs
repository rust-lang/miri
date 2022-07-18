//@rustc-env: RUST_BACKTRACE=0
//@only-target-linux
//@only-on-host
//@compile-flags: -Zmiri-external_c_so_file=tests/external_C/badpath.so
//@normalize-stderr-test: "note: rustc.*running on.*" -> ""

extern "C" {
    fn printer();
}

fn main() {
    unsafe {
        // calling a function from a shared object file that doesn't exist
        printer();
    }
}
