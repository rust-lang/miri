//@only-target-linux
//@only-on-host
//@compile-flags: -Zmiri-external_c_so_file=tests/external_C/libtestlib.so

extern "C" {
    fn printer();
}

fn main() {
    unsafe {
        // test void function that prints from C -- call it twice
        printer();
        printer();
    }
}
