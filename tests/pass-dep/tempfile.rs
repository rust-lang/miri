//@ignore-target-windows: no libc on Windows
//@compile-flags: -Zmiri-disable-isolation

use std::ffi::{CStr, CString};
use std::path::PathBuf;

/// Test that the [`tempfile`] crate is compatible with miri.
fn main() {
    test_tempfile();
    test_tempfile_in();
}

fn tmp() -> PathBuf {
    let path = std::env::var("MIRI_TEMP")
        .unwrap_or_else(|_| std::env::temp_dir().into_os_string().into_string().unwrap());
    // These are host paths. We need to convert them to the target.
    let path = CString::new(path).unwrap();
    let mut out = Vec::with_capacity(1024);

    unsafe {
        extern "Rust" {
            fn miri_host_to_target_path(path: *const i8, out: *mut i8, out_size: usize) -> usize;
        }
        let ret = miri_host_to_target_path(path.as_ptr(), out.as_mut_ptr(), out.capacity());
        assert_eq!(ret, 0);
        let out = CStr::from_ptr(out.as_ptr()).to_str().unwrap();
        PathBuf::from(out)
    }
}

fn test_tempfile() {
    tempfile::tempfile().unwrap();
}

fn test_tempfile_in() {
    let dir_path = tmp();
    tempfile::tempfile_in(dir_path).unwrap();
}
