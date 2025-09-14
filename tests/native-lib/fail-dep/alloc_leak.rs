//@only-target: x86_64-unknown-linux-gnu i686-unknown-linux-gnu
//@compile-flags: -Zmiri-native-lib-enable-tracing

extern "C" {
    fn allocate_bytes(count: u8) -> *mut libc::c_void;
}

fn main() {
    let _bytes = unsafe { allocate_bytes(4) }.cast::<u8>(); //~ERROR: memory leaked
}
