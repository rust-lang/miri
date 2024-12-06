//@only-target: windows # this directly tests windows-only functions
//@compile-flags: -Zmiri-disable-isolation
#![allow(nonstandard_style)]

use std::os::windows::ffi::OsStrExt;
use std::ptr;

#[path = "../../utils/mod.rs"]
mod utils;

// Windows API definitions.
type HANDLE = isize;
type BOOL = i32;
type DWORD = u32;
type LPCWSTR = *const u16;

const GENERIC_READ: u32 = 2147483648u32;
const GENERIC_WRITE: u32 = 1073741824u32;
pub const FILE_SHARE_NONE: u32 = 0u32;
pub const FILE_SHARE_READ: u32 = 1u32;
pub const FILE_SHARE_WRITE: u32 = 2u32;
pub const OPEN_ALWAYS: u32 = 4u32;
pub const OPEN_EXISTING: u32 = 3u32;
pub const CREATE_NEW: u32 = 1u32;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 16u32;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 128u32;
pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x02000000u32;

#[repr(C)]
struct FILETIME {
    dwLowDateTime: DWORD,
    dwHighDateTime: DWORD,
}

#[repr(C)]
struct BY_HANDLE_FILE_INFORMATION {
    dwFileAttributes: DWORD,
    ftCreationTime: FILETIME,
    ftLastAccessTime: FILETIME,
    ftLastWriteTime: FILETIME,
    dwVolumeSerialNumber: DWORD,
    nFileSizeHigh: DWORD,
    nFileSizeLow: DWORD,
    nNumberOfLinks: DWORD,
    nFileIndexHigh: DWORD,
    nFileIndexLow: DWORD,
}

#[repr(C)]
struct SECURITY_ATTRIBUTES {
    nLength: DWORD,
    lpSecurityDescriptor: *mut std::ffi::c_void,
    bInheritHandle: BOOL,
}

extern "system" {
    fn CreateFileW(
        file_name: LPCWSTR,
        dwDesiredAccess: DWORD,
        dwShareMode: DWORD,
        lpSecurityAttributes: *mut SECURITY_ATTRIBUTES,
        dwCreationDisposition: DWORD,
        dwFlagsAndAttributes: DWORD,
        hTemplateFile: HANDLE,
    ) -> HANDLE;
    fn GetFileInformationByHandle(
        handle: HANDLE,
        file_info: *mut BY_HANDLE_FILE_INFORMATION,
    ) -> BOOL;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn GetLastError() -> DWORD;
}

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
