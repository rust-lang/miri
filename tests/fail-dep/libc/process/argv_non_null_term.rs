//@compile-flags: -Zmiri-disable-isolation
//@ignore-target: windows

use std::ffi::CStr;
use std::ptr;

fn main() {
    test_argv_non_null_term();
}

// some valid default arguments
const PATH: &CStr = c"/bin/echo";
// const A0: &CStr = c"echo";
// const ARGV: [*mut libc::c_char; 2] = [A0.as_ptr() as *mut _, ptr::null_mut()];

fn test_argv_non_null_term() {
    #[repr(C)]
    struct ArgvPremium {
        argv: [*mut libc::c_char; 2],    // not-null terminated argv
        premium: [*mut libc::c_char; 2], // not-intended data
    }

    let mut child_pid: libc::pid_t = 0;
    let path = PATH;

    // argv elements
    let a0 = c"echo";
    let a1 = c"Hello";

    let premium_data = ArgvPremium {
        argv: [a0.as_ptr() as *mut _, a1.as_ptr() as *mut _],
        premium: [c"SECRET-PARTY-BYTES".as_ptr() as *mut _, ptr::null_mut()],
    };
    let argv_ptr = premium_data.argv.as_ptr();

    unsafe {
        // 6 args: pid, path, file_actions, attrp, argv, envp
        let ret = libc::posix_spawn( //~ ERROR: attempting a read access
            &mut child_pid,
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            argv_ptr,
            ptr::null(),
        );
        assert_eq!(ret, 0);
    }
    assert!(child_pid > 0);
}
