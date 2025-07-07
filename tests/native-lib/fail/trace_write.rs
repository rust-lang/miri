//@only-target: linux
//@only-target: gnu
//@only-target: x86
//@only-on-host
//@compile-flags: -Zmiri-native-lib-enable-tracing

extern "C" {
    fn init_n(n: i32, ptr: *mut u8);
}

fn main() {
    partial_init();
}

fn partial_init() {
    let mut slice: [u8; 10];
    unsafe {
        init_n(5, (&raw mut slice).cast());
        assert!(slice[3] == 0);
        println!("{}", slice[6]);
    }
}
