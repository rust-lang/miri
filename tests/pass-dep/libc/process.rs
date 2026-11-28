//@ignore-target: windows
//@ignore-host: windows
//@compile-flags: -Zmiri-disable-isolation
//@run-native

use std::ffi::CStr;
use std::ptr;

fn main() {
    test_pid_null();
    test_path_null();
    test_path_empty();
    test_path_wrong1();
    test_argv_empty();
    test_argv_unconventional();
    test_envp_null();
    test_envp_empty();
}

// some valid default arguments
const PATH: &CStr = c"/bin/echo";
const A0: &CStr = c"/bin/echo";
const ARGV: [*mut libc::c_char; 2] = [A0.as_ptr() as *mut _, ptr::null_mut()];

fn test_pid_null() {
    let path = PATH;
    let argv = ARGV;

    unsafe {
        // 6 args: pid, path, file_actions, attrp, argv, envp
        let ret = libc::posix_spawn(
            ptr::null_mut(),
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            argv.as_ptr(),
            ptr::null(),
        );
        assert_eq!(ret, 0);
    }
}

fn test_path_null() {
    let argv = ARGV;

    unsafe {
        // 6 args: pid, path, file_actions, attrp, argv, envp
        let ret = libc::posix_spawn(
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            argv.as_ptr(),
            ptr::null(),
        );
        assert_eq!(ret, libc::EFAULT);
    }
}

fn test_path_empty() {
    let path = c"";
    let argv = ARGV;

    unsafe {
        // 6 args: pid, path, file_actions, attrp, argv, envp
        let ret = libc::posix_spawn(
            ptr::null_mut(),
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            argv.as_ptr(),
            ptr::null(),
        );
        assert_eq!(ret, libc::ENOENT);
    }
}

fn test_path_wrong1() {
    let path = c"/bin/echo432804";
    let argv = ARGV;

    unsafe {
        // 6 args: pid, path, file_actions, attrp, argv, envp
        let ret = libc::posix_spawn(
            ptr::null_mut(),
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            argv.as_ptr(),
            ptr::null(),
        );
        assert_eq!(ret, libc::ENOENT);
    }
}

fn test_argv_empty() {
    let argv: [*mut libc::c_char; 1] = [ptr::null_mut()];

    unsafe {
        // 6 args: pid, path, file_actions, attrp, argv, envp
        let ret = libc::posix_spawn(
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            argv.as_ptr(),
            ptr::null(),
        );
        assert_eq!(ret, libc::EFAULT);
    }
}

fn test_argv_unconventional() {
    let mut child_pid: libc::pid_t = 0;
    let path = PATH;

    // argv elements
    let a0 = c"echo4738219047";

    // argv: null-terminated C array
    let argv: [*mut libc::c_char; 2] = [a0.as_ptr() as *mut _, ptr::null_mut()];

    unsafe {
        // 6 args: pid, path, file_actions, attrp, argv, envp
        let ret = libc::posix_spawn(
            &mut child_pid,
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            argv.as_ptr(),
            ptr::null(),
        );
        assert_eq!(ret, 0);
    }
    assert!(child_pid > 0);
}

fn test_envp_null() {
    let mut child_pid: libc::pid_t = 0;
    let path = PATH;
    let argv = ARGV;

    unsafe {
        // 6 args: pid, path, file_actions, attrp, argv, envp
        let ret = libc::posix_spawn(
            &mut child_pid,
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            argv.as_ptr(),
            ptr::null(),
        );
        assert_eq!(ret, 0);
    }
    assert!(child_pid > 0);
}

fn test_envp_empty() {
    let mut child_pid: libc::pid_t = 0;
    let path = PATH;
    let argv = ARGV;
    let envp: [*mut libc::c_char; 1] = [ptr::null_mut()];

    unsafe {
        // 6 args: pid, path, file_actions, attrp, argv, envp
        let ret = libc::posix_spawn(
            &mut child_pid,
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            argv.as_ptr(),
            envp.as_ptr(),
        );
        assert_eq!(ret, 0);
    }
    assert!(child_pid > 0);
}
