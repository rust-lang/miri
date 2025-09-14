//@only-target: x86_64-unknown-linux-gnu i686-unknown-linux-gnu
//@compile-flags: -Zmiri-native-lib-enable-tracing

extern "C" {
    fn free_ptr(p: *mut u8);
}

fn main() {
    let box_ptr = Box::into_raw(Box::new(0u8));
    unsafe { free_ptr(box_ptr.cast()) }; //~ERROR: Undefined Behavior: deallocating
}
