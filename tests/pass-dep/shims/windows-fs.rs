//@only-target: windows # this directly tests windows-only functions
//@compile-flags: -Zmiri-disable-isolation
#![allow(nonstandard_style)]

use std::os::windows::ffi::OsStrExt;
use std::ptr;

#[path = "../../utils/mod.rs"]
mod utils;

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, GetLastError};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_NORMAL, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetFileInformationByHandle, OPEN_EXISTING,
};

fn main() {
    unsafe {
        test_create_dir_file();
        test_create_normal_file();
    }
}

unsafe fn test_create_dir_file() {
    let temp = utils::tmp();
    let mut raw_path = temp.as_os_str().encode_wide().collect::<Vec<_>>();
    // encode_wide doesn't add a null-terminator
    raw_path.push(0);
    raw_path.push(0);
    let handle = CreateFileW(
        raw_path.as_ptr(),
        GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        ptr::null_mut(),
        OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS,
        0,
    );
    assert_ne!(handle, -1, "CreateNewW Failed: {}", GetLastError());
    let mut info = std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>();
    if GetFileInformationByHandle(handle, &mut info) == 0 {
        panic!("Failed to get file information")
    };
    assert!(info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0);
    if CloseHandle(handle) == 0 {
        panic!("Failed to close file")
    };
}

unsafe fn test_create_normal_file() {
    let temp = utils::tmp().join("test.txt");
    let mut raw_path = temp.as_os_str().encode_wide().collect::<Vec<_>>();
    // encode_wide doesn't add a null-terminator
    raw_path.push(0);
    raw_path.push(0);
    let handle = CreateFileW(
        raw_path.as_ptr(),
        GENERIC_READ | GENERIC_WRITE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        ptr::null_mut(),
        CREATE_NEW,
        0,
        0,
    );
    assert_ne!(handle, -1, "CreateNewW Failed: {}", GetLastError());
    let mut info = std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>();
    if GetFileInformationByHandle(handle, &mut info) == 0 {
        panic!("Failed to get file information: {}", GetLastError())
    };
    assert!(info.dwFileAttributes & FILE_ATTRIBUTE_NORMAL != 0);
    if CloseHandle(handle) == 0 {
        panic!("Failed to close file")
    };
}