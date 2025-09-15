//@only-target: x86_64-unknown-linux-gnu i686-unknown-linux-gnu
//@compile-flags: -Zmiri-native-lib-enable-tracing -Zmiri-permissive-provenance

fn main() {
    test_write_to_mapped();
}

fn test_write_to_mapped() {
    extern "C" {
        fn map_page() -> *mut std::ffi::c_void;
        fn unmap_page(pg: *mut std::ffi::c_void);
    }

    unsafe {
        let pg = map_page().cast::<u64>();
        *pg = 64;
        *pg.offset(10) = 1312;
        unmap_page(pg.cast());
    }
}
