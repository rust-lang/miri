//@ignore-target: windows # No libc

#[path = "../../utils/libc.rs"]
mod libc_utils;

use std::ffi::CStr;
use std::{io, ptr};

use libc_utils::*;

fn main() {
    test_ok();
    test_too_small();
    test_null_ptr();
}

fn test_ok() {
    let mut name = [0u8; 5];
    errno_check(unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) });
    assert_eq!(unsafe { CStr::from_ptr(name.as_ptr().cast()) }, c"Miri");
}

fn test_too_small() {
    let mut name = [0u8; 4];
    let result = unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) };
    cfg_select! {
        target_os = "android" => {
            let err = errno_result(result).unwrap_err();
            assert_eq!(&name, &[0u8; 4]);
            assert_eq!(err.raw_os_error(), Some(libc::ENAMETOOLONG));
            assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ENAMETOOLONG));
        }
        any(all(target_os = "linux", target_env = "gnu"), target_os = "freebsd") => {
            let err = errno_result(result).unwrap_err();
            assert_eq!(&name, b"Miri");
            assert_eq!(err.raw_os_error(), Some(libc::ENAMETOOLONG));
            assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ENAMETOOLONG));
        }
        any(
            all(target_os = "linux", not(target_env = "gnu")),
            target_os = "macos",
            target_os = "illumos",
            target_os = "solaris",
        ) => {
            errno_check(result);
            assert_eq!(&name, b"Mir\0");
        }
        _ => {
            compile_error!("unsupported target");
        }
    }
}

fn test_null_ptr() {
    let err = errno_result(unsafe { libc::gethostname(ptr::null_mut(), 5) }).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(libc::EFAULT));
    assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EFAULT));
}
