use std::ffi::CStr;
use std::{io, ptr};

fn main() {
    test_ok();
    test_null_ptr();
}

fn test_ok() {
    // SAFETY: all zeros for `utsname` is valid.
    let mut uname: libc::utsname = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::uname(&mut uname) };
    if result != 0 {
        panic!("failed to call uname");
    }

    // These values are only correct when running isolated.
    assert_eq!(unsafe { CStr::from_ptr(&uname.sysname as *const _) }, c"Linux");
    assert_eq!(unsafe { CStr::from_ptr(&uname.nodename as *const _) }, c"Miri");
    assert_eq!(unsafe { CStr::from_ptr(&uname.release as *const _) }, c"6.18.1-arch1-2");
    assert_eq!(
        unsafe { CStr::from_ptr(&uname.version as *const _) },
        c"#1 SMP PREEMPT_DYNAMIC Sat, 13 Dec 2025 18:23:21 +0000"
    );
    assert_eq!(unsafe { CStr::from_ptr(&uname.machine as *const _) }, c"x86_64");
    #[cfg(any(target_os = "linux", target_os = "android"))]
    assert_eq!(unsafe { CStr::from_ptr(&uname.domainname as *const _) }, c"(none)");
}

fn test_null_ptr() {
    let result = unsafe { libc::uname(ptr::null_mut()) };
    assert_eq!(result, -1);
    assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EFAULT));
}
