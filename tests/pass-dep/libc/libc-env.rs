//@ignore-target: windows
//@compile-flags: -Zmiri-disable-isolation

use std::ffi::CStr;

#[path = "../../utils/mod.rs"]
mod utils;

fn test_getenv() {
    unsafe {
        // PATH should exist and not be null
        let s = libc::getenv(b"PATH\0".as_ptr().cast());
        assert!(!s.is_null());

        // Get a non-existing environment variable
        let s = libc::getenv(b"MIRI_TEST_NONEXISTENT_VAR\0".as_ptr().cast());
        assert!(s.is_null());

        // Empty string should not crash
        let s = libc::getenv(b"\0".as_ptr().cast());
        assert!(s.is_null());
    }
}

fn test_setenv() {
    unsafe {
        // Set a new environment variable
        let result =
            libc::setenv(b"MIRI_TEST_VAR\0".as_ptr().cast(), b"test_value\0".as_ptr().cast(), 1);
        assert_eq!(result, 0);

        // Verify it was set
        let s = libc::getenv(b"MIRI_TEST_VAR\0".as_ptr().cast());
        assert!(!s.is_null());
        let value = CStr::from_ptr(s).to_str().unwrap();
        assert_eq!(value, "test_value");

        // Test overwriting an existing variable
        let result =
            libc::setenv(b"MIRI_TEST_VAR\0".as_ptr().cast(), b"new_value\0".as_ptr().cast(), 1);
        assert_eq!(result, 0);

        // Verify it was updated
        let s = libc::getenv(b"MIRI_TEST_VAR\0".as_ptr().cast());
        assert!(!s.is_null());
        let value = CStr::from_ptr(s).to_str().unwrap();
        assert_eq!(value, "new_value");

        // Test invalid parameters
        let result = libc::setenv(std::ptr::null(), b"value\0".as_ptr().cast(), 1);
        assert_eq!(result, -1);

        let result = libc::setenv(b"\0".as_ptr().cast(), b"value\0".as_ptr().cast(), 1);
        assert_eq!(result, -1);

        let result = libc::setenv(b"INVALID=NAME\0".as_ptr().cast(), b"value\0".as_ptr().cast(), 1);
        assert_eq!(result, -1);
    }
}

fn test_unsetenv() {
    unsafe {
        // Set a variable
        let result = libc::setenv(
            b"MIRI_TEST_UNSET_VAR\0".as_ptr().cast(),
            b"to_be_unset\0".as_ptr().cast(),
            1,
        );
        assert_eq!(result, 0);

        // Verify it exists
        let s = libc::getenv(b"MIRI_TEST_UNSET_VAR\0".as_ptr().cast());
        assert!(!s.is_null());

        // Unset it
        let result = libc::unsetenv(b"MIRI_TEST_UNSET_VAR\0".as_ptr().cast());
        assert_eq!(result, 0);

        // Verify it was unset
        let s = libc::getenv(b"MIRI_TEST_UNSET_VAR\0".as_ptr().cast());
        assert!(s.is_null());

        // Test unsetting a non-existing variable (should succeed)
        let result = libc::unsetenv(b"MIRI_TEST_NONEXISTENT_VAR\0".as_ptr().cast());
        assert_eq!(result, 0);

        // Test invalid parameters
        let result = libc::unsetenv(std::ptr::null());
        assert_eq!(result, -1);

        let result = libc::unsetenv(b"\0".as_ptr().cast());
        assert_eq!(result, -1);

        let result = libc::unsetenv(b"INVALID=NAME\0".as_ptr().cast());
        assert_eq!(result, -1);
    }
}

fn main() {
    test_getenv();
    test_setenv();
    test_unsetenv();
}
