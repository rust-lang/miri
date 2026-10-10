//@ignore-target: windows # libc bits exist, but we don't support them
//@ignore-host: windows # needs unix PermissionExt
//@compile-flags: -Zmiri-disable-isolation
//@run-native

use std::ffi::CStr;
use std::mem::MaybeUninit;

#[path = "../../utils/mod.rs"]
mod utils;

#[path = "../../utils/libc.rs"]
mod libc_utils;
use libc_utils::{errno_check, errno_result};

fn main() {
    test_chmod();
    test_fchmod();
    test_owner();
}

#[track_caller]
fn getmod(path: &CStr) -> u32 {
    let mut stat = MaybeUninit::<libc::stat>::uninit();
    unsafe { errno_check(libc::stat(path.as_ptr(), stat.as_mut_ptr())) };
    u32::from(unsafe { stat.assume_init_ref().st_mode & !libc::S_IFMT })
}

fn test_chmod() {
    let path = utils::prepare_with_content("miri_test_libc_chmod.txt", b"abcdef");
    let c_path = utils::into_c_string(path);

    unsafe { errno_check(libc::chmod(c_path.as_ptr(), 0o777)) };
    assert_eq!(getmod(&c_path), 0o777);
    unsafe { errno_check(libc::chmod(c_path.as_ptr(), 0o610)) };
    assert_eq!(getmod(&c_path), 0o610);
}

fn test_fchmod() {
    let path = utils::prepare_with_content("miri_test_libc_chmod.txt", b"abcdef");
    let c_path = utils::into_c_string(path);

    let fd = unsafe { errno_result(libc::open(c_path.as_ptr(), libc::O_RDONLY)).unwrap() };
    unsafe { errno_check(libc::fchmod(fd, 0o777)) };
    assert_eq!(getmod(&c_path), 0o777);
    unsafe { errno_check(libc::fchmod(fd, 0o610)) };
    assert_eq!(getmod(&c_path), 0o610);
}

/// Check that the IDs of the process match the owner of the files it creates.
fn test_owner() {
    let path = utils::prepare_with_content("miri_test_libc_owner.txt", b"abcdef");
    let c_parent = utils::into_c_string(path.parent().unwrap());
    let c_path = utils::into_c_string(path);

    let mut stat = MaybeUninit::<libc::stat>::uninit();
    unsafe { errno_check(libc::stat(c_path.as_ptr(), stat.as_mut_ptr())) };
    let stat = unsafe { stat.assume_init_ref() };
    let mut parent_stat = MaybeUninit::<libc::stat>::uninit();
    unsafe { errno_check(libc::stat(c_parent.as_ptr(), parent_stat.as_mut_ptr())) };
    let parent_stat = unsafe { parent_stat.assume_init_ref() };

    assert_eq!(stat.st_uid, unsafe { libc::geteuid() });
    // The group of a new file is either our effective group, or (on BSD-like systems and in
    // setgid directories) the group of the parent directory.
    assert!(stat.st_gid == unsafe { libc::getegid() } || stat.st_gid == parent_stat.st_gid);
}
