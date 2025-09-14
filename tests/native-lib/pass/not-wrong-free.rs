//@only-target: x86_64-unknown-linux-gnu i686-unknown-linux-gnu
//@compile-flags: -Zmiri-native-lib-enable-tracing

#[global_allocator]
static SOME_STATIC: std::alloc::System = std::alloc::System;

extern "C" {
    fn free_ptr(p: *mut u8);
}

fn main() {
    let box_ptr = Box::into_raw(Box::new(0u8));
    unsafe { free_ptr(box_ptr.cast()) }; // This is fine now :3
}
