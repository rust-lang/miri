// Only works on Unix targets
//@ignore-target: windows wasm
//@only-on-host

fn main() {
    get_c_ptr();
}

fn get_c_ptr() {
    extern "C" {
        fn get_c_ptr() -> *const ();
    }
    unsafe {
        let ptr: *mut i64 = get_c_ptr() as _;
        *ptr = 20;
    }
}
