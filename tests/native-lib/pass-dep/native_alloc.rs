//@only-target: x86_64-unknown-linux-gnu i686-unknown-linux-gnu
//@compile-flags: -Zmiri-native-lib-enable-tracing

fn main() {
    test_access_alloc();
    test_free_foreign_natively();
    test_free_native_foreignly();
}

fn test_access_alloc() {
    extern "C" {
        fn allocate_bytes(count: u8) -> *mut libc::c_void;
        fn free_ptr(p: *mut libc::c_void);
    }

    let ptr = unsafe { allocate_bytes(12) }.cast::<u8>();
    for ofs in 0u8..12 {
        unsafe {
            *(ptr.add(ofs.into())) = ofs;
            assert_eq!(*(ptr.add(ofs.into())), ofs);
        }
    }
    unsafe {
        free_ptr(ptr.cast());
    }
}

fn test_free_foreign_natively() {
    extern "C" {
        fn allocate_bytes(count: u8) -> *mut libc::c_void;
    }

    unsafe {
        let ptr = allocate_bytes(4);
        libc::free(ptr);
    }
}

fn test_free_native_foreignly() {
    extern "C" {
        fn free_ptr(p: *mut libc::c_void);
    }

    unsafe {
        let ptr = libc::malloc(64);
        // An invalid free won't deterministically crash, but it's likely enough
        // that it's worth testing for.
        free_ptr(ptr);
    }
}
