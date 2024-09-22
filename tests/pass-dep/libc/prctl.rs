//@only-target: android # prctl is currently supported for Android only
use std::ffi::CStr;
use std::ffi::CString;

fn main() {
    let long_name = CString::new(
        std::iter::once("test_named_thread_truncation")
            .chain(std::iter::repeat(" yada").take(100))
            .collect::<String>(),
    )
    .unwrap();

    // prctl supports thread names up to 16 characters includding nul.
    // And since the input string is longer it should fail.
    assert_ne!(set_thread_name(&long_name), 0);

    let cstr = CString::new(&long_name.as_bytes()[..15]).unwrap();
    assert_eq!(set_thread_name(&cstr), 0);

    let mut buf = vec![0u8; 16];
    assert_eq!(get_thread_name(&mut buf), 0);
}

fn set_thread_name(name: &CStr) -> i32 {
    cfg_if::cfg_if! {
        if #[cfg(any(target_os = "android"))] {
            unsafe { libc::prctl(libc::PR_SET_NAME, name.as_ptr().cast()) }
        } else {
            compile_error!("set_thread_name not supported for this OS")
        }
    }
}

fn get_thread_name(name: &mut [u8]) -> i32 {
    cfg_if::cfg_if! {
        if #[cfg(any(
            target_os = "android",
        ))] {
            unsafe {
                libc::prctl(libc::PR_GET_NAME, name.as_mut_ptr().cast())
            }
        } else {
            compile_error!("get_thread_name not supported for this OS")
        }
    }
}
