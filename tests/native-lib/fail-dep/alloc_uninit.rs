//@only-target: x86_64-unknown-linux-gnu i686-unknown-linux-gnu
//@compile-flags: -Zmiri-native-lib-enable-tracing

extern "C" {
    fn allocate_bytes(count: u8) -> *mut libc::c_void;
    fn write_byte_with_ofs(ptr: *mut libc::c_void, ofs: usize, byte: u8);
}

fn main() {
    let bytes = unsafe { allocate_bytes(2) };
    unsafe { write_byte_with_ofs(bytes, 0, 12) };
    assert_eq!(unsafe { *(bytes.cast::<u8>()) }, 12);
    // The error message doesn't neatly fit into the pattern the tests demand,
    // but the error is due to uninit memory as per the .stderr file.
    let _val = unsafe { *(bytes.cast::<u8>().offset(1)) }; //~ERROR: Undefined Behavior
}
