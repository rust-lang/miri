//@only-target: android # prctl is currently supported for Android only
use std::ffi::{CStr, CString};

use libc::c_char;

fn main() {
    let long_name = CString::new(
        std::iter::once("test_named_thread_truncation")
            .chain(std::iter::repeat(" yada").take(100))
            .collect::<String>(),
    )
    .unwrap();

    // prctl supports thread names up to 16 characters including nul.
    // Since the input string is longer, it should fail.
    assert_ne!(set_thread_name(&long_name), 0);

    let cstr = CString::new(&long_name.as_bytes()[..15]).unwrap();
    assert_eq!(set_thread_name(&cstr), 0);

    let mut buf = vec![0u8; 16];
    assert_eq!(get_thread_name(&mut buf), 0);
}

fn set_thread_name(name: &CStr) -> i32 {
    const PR_SET_NAME: i32 = 15;
    unsafe { libc::prctl(PR_SET_NAME, name.as_ptr().cast::<*const c_char>()) }
}

fn get_thread_name(name: &mut [u8]) -> i32 {
    const PR_GET_NAME: i32 = 16;
    unsafe { libc::prctl(PR_GET_NAME, name.as_mut_ptr().cast::<*mut c_char>()) }
}
