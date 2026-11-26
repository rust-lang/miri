//@compile-flags: -Zmiri-disable-isolation

use std::ffi::CStr;
use std::ptr;

fn main() {
    test_envp_non_null_term();
}

// some valid default arguments
const PATH: &CStr = c"/bin/echo";
const A0: &CStr = c"echo";
const ARGV: [*mut libc::c_char; 2] = [A0.as_ptr() as *mut _, ptr::null_mut()];

fn test_envp_non_null_term() {
    #[repr(C)]
    struct EnvpPremium {
        envp: [*mut libc::c_char; 2],    // not-null terminated envp
        premium: [*mut libc::c_char; 2], // not-intended data
    }

    let mut child_pid: libc::pid_t = 0;
    let path = PATH;
    let argv = ARGV;

    // envp elements
    let ev0 = c"k1=v1";
    let ev1 = c"k2=v2";

    let premium_data = EnvpPremium {
        envp: [ev0.as_ptr() as *mut _, ev1.as_ptr() as *mut _],
        premium: [c"SECRET-PARTY-BYTES".as_ptr() as *mut _, ptr::null_mut()],
    };
    let envp_ptr = premium_data.envp.as_ptr();

    unsafe {
        // 6 args: pid, path, file_actions, attrp, argv, envp
        let ret = libc::posix_spawn(
            &mut child_pid,
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            argv.as_ptr(),
            envp_ptr,
        );
        assert_eq!(ret, 0);
    }
    assert!(child_pid > 0);
}
