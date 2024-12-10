//@ignore-target: windows # File handling is not implemented yet
//@compile-flags: -Zmiri-disable-isolation

#![feature(io_error_more)]
#![feature(io_error_uncategorized)]

use std::ffi::{CStr, CString, OsString};
use std::fs::{File, canonicalize, remove_file};
use std::io::{Error, ErrorKind, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

#[path = "../../utils/mod.rs"]
mod utils;

/// Platform-specific offset type for file seeking operations.
/// the lseek system call typically expects an off_t type, which can be 64 bits
/// even on some 32-bit systems.
#[cfg(any(
    target_os = "illumos",
    target_os = "solaris",
    target_os = "android",
    all(target_os = "linux", target_pointer_width = "64"),
    all(target_os = "macos", target_arch = "aarch64")
))]
type LseekOffset = i64;

#[cfg(any(
    target_os = "freebsd",
    all(target_os = "macos", not(target_arch = "aarch64")),
    all(target_os = "linux", target_pointer_width = "32")
))]
type LseekOffset = i32;

/// Seeks to a specific position in the file
fn seek(fd: i32, offset: LseekOffset) -> LseekOffset {
    let result = unsafe {
        // the lseek64 function is not part of the POSIX standard and
        // may not be available on all systems.
        #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
        let result = libc::lseek64(fd, offset.into(), libc::SEEK_SET);

        #[cfg(not(all(target_os = "linux", target_pointer_width = "64")))]
        let result = libc::lseek(fd, offset.into(), libc::SEEK_SET);

        result
    };

    LseekOffset::try_from(result).expect("seek operation failed")
}

fn main() {
    test_dup();
    test_dup_stdout_stderr();
    test_canonicalize_too_long();
    test_rename();
    test_ftruncate::<libc::off_t>(libc::ftruncate);
    #[cfg(target_os = "linux")]
    test_ftruncate::<libc::off64_t>(libc::ftruncate64);
    test_file_open_unix_allow_two_args();
    test_file_open_unix_needs_three_args();
    test_file_open_unix_extra_third_arg();
    #[cfg(target_os = "linux")]
    test_o_tmpfile_flag();
    test_posix_mkstemp();
    test_posix_realpath_alloc();
    test_posix_realpath_noalloc();
    test_posix_realpath_errors();
    #[cfg(target_os = "linux")]
    test_posix_fadvise();
    #[cfg(target_os = "linux")]
    test_sync_file_range();
    test_isatty();
    test_read_and_uninit();
    test_nofollow_not_symlink();
    test_readv_basic();
    test_readv_large_buffers();
    test_readv_partial_and_eof();
    test_readv_error_conditions();
}

fn test_file_open_unix_allow_two_args() {
    let path = utils::prepare_with_content("test_file_open_unix_allow_two_args.txt", &[]);

    let mut name = path.into_os_string();
    name.push("\0");
    let name_ptr = name.as_bytes().as_ptr().cast::<libc::c_char>();
    let _fd = unsafe { libc::open(name_ptr, libc::O_RDONLY) };
}

fn test_file_open_unix_needs_three_args() {
    let path = utils::prepare_with_content("test_file_open_unix_needs_three_args.txt", &[]);

    let mut name = path.into_os_string();
    name.push("\0");
    let name_ptr = name.as_bytes().as_ptr().cast::<libc::c_char>();
    let _fd = unsafe { libc::open(name_ptr, libc::O_CREAT, 0o666) };
}

fn test_file_open_unix_extra_third_arg() {
    let path = utils::prepare_with_content("test_file_open_unix_extra_third_arg.txt", &[]);

    let mut name = path.into_os_string();
    name.push("\0");
    let name_ptr = name.as_bytes().as_ptr().cast::<libc::c_char>();
    let _fd = unsafe { libc::open(name_ptr, libc::O_RDONLY, 42) };
}

fn test_dup_stdout_stderr() {
    let bytes = b"hello dup fd\n";
    unsafe {
        let new_stdout = libc::fcntl(1, libc::F_DUPFD, 0);
        let new_stderr = libc::fcntl(2, libc::F_DUPFD, 0);
        libc::write(new_stdout, bytes.as_ptr() as *const libc::c_void, bytes.len());
        libc::write(new_stderr, bytes.as_ptr() as *const libc::c_void, bytes.len());
    }
}

fn test_dup() {
    let bytes = b"dup and dup2";
    let path = utils::prepare_with_content("miri_test_libc_dup.txt", bytes);

    let mut name = path.into_os_string();
    name.push("\0");
    let name_ptr = name.as_bytes().as_ptr().cast::<libc::c_char>();
    unsafe {
        let fd = libc::open(name_ptr, libc::O_RDONLY);
        let mut first_buf = [0u8; 4];
        libc::read(fd, first_buf.as_mut_ptr() as *mut libc::c_void, 4);
        assert_eq!(&first_buf, b"dup ");

        let new_fd = libc::dup(fd);
        let mut second_buf = [0u8; 4];
        libc::read(new_fd, second_buf.as_mut_ptr() as *mut libc::c_void, 4);
        assert_eq!(&second_buf, b"and ");

        let new_fd2 = libc::dup2(fd, 8);
        let mut third_buf = [0u8; 4];
        libc::read(new_fd2, third_buf.as_mut_ptr() as *mut libc::c_void, 4);
        assert_eq!(&third_buf, b"dup2");
    }
}

fn test_canonicalize_too_long() {
    // Make sure we get an error for long paths.
    let too_long = "x/".repeat(libc::PATH_MAX.try_into().unwrap());
    assert!(canonicalize(too_long).is_err());
}

fn test_rename() {
    let path1 = utils::prepare("miri_test_libc_fs_source.txt");
    let path2 = utils::prepare("miri_test_libc_fs_rename_destination.txt");

    let file = File::create(&path1).unwrap();
    drop(file);

    let c_path1 = CString::new(path1.as_os_str().as_bytes()).expect("CString::new failed");
    let c_path2 = CString::new(path2.as_os_str().as_bytes()).expect("CString::new failed");

    // Renaming should succeed
    unsafe { libc::rename(c_path1.as_ptr(), c_path2.as_ptr()) };
    // Check that old file path isn't present
    assert_eq!(ErrorKind::NotFound, path1.metadata().unwrap_err().kind());
    // Check that the file has moved successfully
    assert!(path2.metadata().unwrap().is_file());

    // Renaming a nonexistent file should fail
    let res = unsafe { libc::rename(c_path1.as_ptr(), c_path2.as_ptr()) };
    assert_eq!(res, -1);
    assert_eq!(Error::last_os_error().kind(), ErrorKind::NotFound);

    remove_file(&path2).unwrap();
}

fn test_ftruncate<T: From<i32>>(
    ftruncate: unsafe extern "C" fn(fd: libc::c_int, length: T) -> libc::c_int,
) {
    // libc::off_t is i32 in target i686-unknown-linux-gnu
    // https://docs.rs/libc/latest/i686-unknown-linux-gnu/libc/type.off_t.html

    let bytes = b"hello";
    let path = utils::prepare("miri_test_libc_fs_ftruncate.txt");
    let mut file = File::create(&path).unwrap();
    file.write(bytes).unwrap();
    file.sync_all().unwrap();
    assert_eq!(file.metadata().unwrap().len(), 5);

    let c_path = CString::new(path.as_os_str().as_bytes()).expect("CString::new failed");
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };

    // Truncate to a bigger size
    let mut res = unsafe { ftruncate(fd, T::from(10)) };
    assert_eq!(res, 0);
    assert_eq!(file.metadata().unwrap().len(), 10);

    // Write after truncate
    file.write(b"dup").unwrap();
    file.sync_all().unwrap();
    assert_eq!(file.metadata().unwrap().len(), 10);

    // Truncate to smaller size
    res = unsafe { ftruncate(fd, T::from(2)) };
    assert_eq!(res, 0);
    assert_eq!(file.metadata().unwrap().len(), 2);

    remove_file(&path).unwrap();
}

#[cfg(target_os = "linux")]
fn test_o_tmpfile_flag() {
    use std::fs::{OpenOptions, create_dir};
    use std::os::unix::fs::OpenOptionsExt;
    let dir_path = utils::prepare_dir("miri_test_fs_dir");
    create_dir(&dir_path).unwrap();
    // test that the `O_TMPFILE` custom flag gracefully errors instead of stopping execution
    assert_eq!(
        Some(libc::EOPNOTSUPP),
        OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_TMPFILE)
            .open(dir_path)
            .unwrap_err()
            .raw_os_error(),
    );
}

fn test_posix_mkstemp() {
    use std::ffi::OsStr;
    use std::os::unix::io::FromRawFd;
    use std::path::Path;

    let valid_template = "fooXXXXXX";
    // C needs to own this as `mkstemp(3)` says:
    // "Since it will be modified, `template` must not be a string constant, but
    // should be declared as a character array."
    // There seems to be no `as_mut_ptr` on `CString` so we need to use `into_raw`.
    let ptr = CString::new(valid_template).unwrap().into_raw();
    let fd = unsafe { libc::mkstemp(ptr) };
    // Take ownership back in Rust to not leak memory.
    let slice = unsafe { CString::from_raw(ptr) };
    assert!(fd > 0);
    let osstr = OsStr::from_bytes(slice.to_bytes());
    let path: &Path = osstr.as_ref();
    let name = path.file_name().unwrap().to_string_lossy();
    assert!(name.ne("fooXXXXXX"));
    assert!(name.starts_with("foo"));
    assert_eq!(name.len(), 9);
    assert_eq!(
        name.chars().skip(3).filter(char::is_ascii_alphanumeric).collect::<Vec<char>>().len(),
        6
    );
    let file = unsafe { File::from_raw_fd(fd) };
    assert!(file.set_len(0).is_ok());

    let invalid_templates = vec!["foo", "barXX", "XXXXXXbaz", "whatXXXXXXever", "X"];
    for t in invalid_templates {
        let ptr = CString::new(t).unwrap().into_raw();
        let fd = unsafe { libc::mkstemp(ptr) };
        let _ = unsafe { CString::from_raw(ptr) };
        // "On error, -1 is returned, and errno is set to
        // indicate the error"
        assert_eq!(fd, -1);
        let e = std::io::Error::last_os_error();
        assert_eq!(e.raw_os_error(), Some(libc::EINVAL));
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
    }
}

/// Test allocating variant of `realpath`.
fn test_posix_realpath_alloc() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let buf;
    let path = utils::tmp().join("miri_test_libc_posix_realpath_alloc");
    let c_path = CString::new(path.as_os_str().as_bytes()).expect("CString::new failed");

    // Cleanup before test.
    remove_file(&path).ok();
    // Create file.
    drop(File::create(&path).unwrap());
    unsafe {
        let r = libc::realpath(c_path.as_ptr(), std::ptr::null_mut());
        assert!(!r.is_null());
        buf = CStr::from_ptr(r).to_bytes().to_vec();
        libc::free(r as *mut _);
    }
    let canonical = PathBuf::from(OsString::from_vec(buf));
    assert_eq!(path.file_name(), canonical.file_name());

    // Cleanup after test.
    remove_file(&path).unwrap();
}

/// Test non-allocating variant of `realpath`.
fn test_posix_realpath_noalloc() {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;

    let path = utils::tmp().join("miri_test_libc_posix_realpath_noalloc");
    let c_path = CString::new(path.as_os_str().as_bytes()).expect("CString::new failed");

    let mut v = vec![0; libc::PATH_MAX as usize];

    // Cleanup before test.
    remove_file(&path).ok();
    // Create file.
    drop(File::create(&path).unwrap());
    unsafe {
        let r = libc::realpath(c_path.as_ptr(), v.as_mut_ptr());
        assert!(!r.is_null());
    }
    let c = unsafe { CStr::from_ptr(v.as_ptr()) };
    let canonical = PathBuf::from(c.to_str().expect("CStr to str"));

    assert_eq!(path.file_name(), canonical.file_name());

    // Cleanup after test.
    remove_file(&path).unwrap();
}

/// Test failure cases for `realpath`.
fn test_posix_realpath_errors() {
    use std::ffi::CString;
    use std::io::ErrorKind;

    // Test nonexistent path returns an error.
    let c_path = CString::new("./nothing_to_see_here").expect("CString::new failed");
    let r = unsafe { libc::realpath(c_path.as_ptr(), std::ptr::null_mut()) };
    assert!(r.is_null());
    let e = std::io::Error::last_os_error();
    assert_eq!(e.raw_os_error(), Some(libc::ENOENT));
    assert_eq!(e.kind(), ErrorKind::NotFound);
}

#[cfg(target_os = "linux")]
fn test_posix_fadvise() {
    use std::io::Write;

    let path = utils::tmp().join("miri_test_libc_posix_fadvise.txt");
    // Cleanup before test
    remove_file(&path).ok();

    // Set up an open file
    let mut file = File::create(&path).unwrap();
    let bytes = b"Hello, World!\n";
    file.write(bytes).unwrap();

    // Test calling posix_fadvise on a file.
    let result = unsafe {
        libc::posix_fadvise(
            file.as_raw_fd(),
            0,
            bytes.len().try_into().unwrap(),
            libc::POSIX_FADV_DONTNEED,
        )
    };
    drop(file);
    remove_file(&path).unwrap();
    assert_eq!(result, 0);
}

#[cfg(target_os = "linux")]
fn test_sync_file_range() {
    use std::io::Write;

    let path = utils::tmp().join("miri_test_libc_sync_file_range.txt");
    // Cleanup before test.
    remove_file(&path).ok();

    // Write to a file.
    let mut file = File::create(&path).unwrap();
    let bytes = b"Hello, World!\n";
    file.write(bytes).unwrap();

    // Test calling sync_file_range on the file.
    let result_1 = unsafe {
        libc::sync_file_range(
            file.as_raw_fd(),
            0,
            0,
            libc::SYNC_FILE_RANGE_WAIT_BEFORE
                | libc::SYNC_FILE_RANGE_WRITE
                | libc::SYNC_FILE_RANGE_WAIT_AFTER,
        )
    };
    drop(file);

    // Test calling sync_file_range on a file opened for reading.
    let file = File::open(&path).unwrap();
    let result_2 = unsafe {
        libc::sync_file_range(
            file.as_raw_fd(),
            0,
            0,
            libc::SYNC_FILE_RANGE_WAIT_BEFORE
                | libc::SYNC_FILE_RANGE_WRITE
                | libc::SYNC_FILE_RANGE_WAIT_AFTER,
        )
    };
    drop(file);

    remove_file(&path).unwrap();
    assert_eq!(result_1, 0);
    assert_eq!(result_2, 0);
}

fn test_isatty() {
    // Testing whether our isatty shim returns the right value would require controlling whether
    // these streams are actually TTYs, which is hard.
    // For now, we just check that these calls are supported at all.
    unsafe {
        libc::isatty(libc::STDIN_FILENO);
        libc::isatty(libc::STDOUT_FILENO);
        libc::isatty(libc::STDERR_FILENO);

        // But when we open a file, it is definitely not a TTY.
        let path = utils::tmp().join("notatty.txt");
        // Cleanup before test.
        remove_file(&path).ok();
        let file = File::create(&path).unwrap();

        assert_eq!(libc::isatty(file.as_raw_fd()), 0);
        assert_eq!(std::io::Error::last_os_error().raw_os_error().unwrap(), libc::ENOTTY);

        // Cleanup after test.
        drop(file);
        remove_file(&path).unwrap();
    }
}

fn test_read_and_uninit() {
    use std::mem::MaybeUninit;
    {
        // We test that libc::read initializes its buffer.
        let path = utils::prepare_with_content("pass-libc-read-and-uninit.txt", &[1u8, 2, 3]);
        let cpath = CString::new(path.clone().into_os_string().into_encoded_bytes()).unwrap();
        unsafe {
            let fd = libc::open(cpath.as_ptr(), libc::O_RDONLY);
            assert_ne!(fd, -1);
            let mut buf: MaybeUninit<[u8; 2]> = std::mem::MaybeUninit::uninit();
            assert_eq!(libc::read(fd, buf.as_mut_ptr().cast::<std::ffi::c_void>(), 2), 2);
            let buf = buf.assume_init();
            assert_eq!(buf, [1, 2]);
            assert_eq!(libc::close(fd), 0);
        }
        remove_file(&path).unwrap();
    }
    {
        // We test that if we requested to read 4 bytes, but actually read 3 bytes, then
        // 3 bytes (not 4) will be overwritten, and remaining byte will be left as-is.
        let path = utils::prepare_with_content("pass-libc-read-and-uninit-2.txt", &[1u8, 2, 3]);
        let cpath = CString::new(path.clone().into_os_string().into_encoded_bytes()).unwrap();
        unsafe {
            let fd = libc::open(cpath.as_ptr(), libc::O_RDONLY);
            assert_ne!(fd, -1);
            let mut buf = [42u8; 5];
            assert_eq!(libc::read(fd, buf.as_mut_ptr().cast::<std::ffi::c_void>(), 4), 3);
            assert_eq!(buf, [1, 2, 3, 42, 42]);
            assert_eq!(libc::close(fd), 0);
        }
        remove_file(&path).unwrap();
    }
}

fn test_nofollow_not_symlink() {
    let bytes = b"Hello, World!\n";
    let path = utils::prepare_with_content("test_nofollow_not_symlink.txt", bytes);
    let cpath = CString::new(path.as_os_str().as_bytes()).unwrap();
    let ret = unsafe { libc::open(cpath.as_ptr(), libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    assert!(ret >= 0);
}

/// Tests basic functionality of the readv() system call by reading a small file
/// with multiple buffers.
///
/// Verifies that:
/// - File contents are read correctly into provided buffers
/// - The total number of bytes read matches file size
/// - Buffer boundaries are respected
/// - Return values match expected behavior
fn test_readv_basic() {
    let bytes = b"abcdefgh";
    let path = utils::prepare_with_content("miri_test_libc_readv.txt", bytes);

    // Convert path to a null-terminated CString.
    let name = CString::new(path.into_os_string().into_string().unwrap()).unwrap();
    let name_ptr = name.as_ptr();

    let mut first_buf = [0u8; 4];
    let mut second_buf = [0u8; 8];

    unsafe {
        // Define iovec structures.
        let iov: [libc::iovec; 2] = [
            libc::iovec {
                iov_len: first_buf.len() as usize,
                iov_base: first_buf.as_mut_ptr() as *mut libc::c_void,
            },
            libc::iovec {
                iov_len: second_buf.len() as usize,
                iov_base: second_buf.as_mut_ptr() as *mut libc::c_void,
            },
        ];

        // Open file.
        let fd = libc::open(name_ptr, libc::O_RDONLY);
        if fd < 0 {
            eprintln!("Failed to open file: {}", Error::last_os_error().to_string());
            return;
        }

        // Call readv with proper type conversions.
        let iovcnt = libc::c_int::try_from(iov.len()).expect("iovec count too large for platform");

        // Call readv with proper type handling for the count.
        let res = libc::readv(fd, iov.as_ptr() as *const libc::iovec, iovcnt);

        if res < 0 {
            eprintln!("Failed to readv: {}", Error::last_os_error());
            libc::close(fd);
            return;
        }

        // Close the file descriptor.
        libc::close(fd);
    }

    // Validate buffers.
    if first_buf != *b"abcd" {
        eprintln!("First buffer mismatch: {:?}", first_buf);
    }

    if second_buf != *b"efgh\0\0\0\0" {
        eprintln!("Second buffer mismatch: {:?}", second_buf);
    }
}

/// Tests readv() system call with large buffer sizes and pattern verification.
/// Uses multiple buffers (16KB, 16KB, 32KB) to read a 64KB file containing
/// a repeating 'ABCD' pattern with markers at buffer boundaries.
///
/// Verifies that:
/// - Large file contents are read correctly
/// - Markers at buffer boundaries are preserved
/// - Pattern integrity is maintained between markers
/// - Memory safety with large allocations
/// - Buffer boundary handling for larger sizes
fn test_readv_large_buffers() {
    const BUFFER_SIZE_1: usize = 16384; // 16KB
    const BUFFER_SIZE_2: usize = 16384; // 16KB
    const BUFFER_SIZE_3: usize = 32768; // 32KB

    // Define our buffer sizes
    let buffer_sizes = &[
        BUFFER_SIZE_1, // 16KB
        BUFFER_SIZE_2, // 16KB
        BUFFER_SIZE_3, // 32KB
    ];

    // Create large test file with patterns and markers.
    // Generate pattern with awareness of buffer boundaries.
    let large_content = utils::generate_test_pattern(buffer_sizes);

    let path = utils::prepare_with_content("large_readv_test.txt", &large_content);

    // Create buffers based on our defined sizes.
    let mut buffers: Vec<Vec<u8>> = buffer_sizes.iter().map(|&size| vec![0u8; size]).collect();

    // Convert path to CString for libc interface.
    let path_cstr = CString::new(path.into_os_string().into_string().unwrap()).unwrap();

    let bytes_read: usize = unsafe {
        let fd = libc::open(path_cstr.as_ptr(), libc::O_RDONLY);
        assert!(fd > 0, "Failed to open test file");

        // Create iovec array using our buffers.
        let iov = buffers
            .iter_mut()
            .map(|buf| {
                libc::iovec { iov_base: buf.as_mut_ptr() as *mut libc::c_void, iov_len: buf.len() }
            })
            .collect::<Vec<_>>();

        // Perform readv operation.
        let read_result = libc::readv(fd, iov.as_ptr(), iov.len() as i32);

        libc::close(fd);
        read_result.try_into().unwrap()
    };

    // Verify total bytes read.
    let expected_total: usize = buffer_sizes.iter().sum();
    assert_eq!(
        bytes_read, expected_total,
        "Unexpected bytes read. Expected {}, got {}",
        expected_total, bytes_read
    );

    // Verify markers in each buffer with correct positioning.
    let mut current_pos = 0;
    for (i, buf) in buffers.iter().enumerate() {
        let marker = format!("##MARKER{}##", i + 1);
        let marker_len = marker.len();

        // Calculate correct position for this buffer
        let buffer_size = buf.len();
        let marker_pos = buffer_size - marker_len;

        // Read the exact number of bytes needed for the marker.
        let content = std::str::from_utf8(&buf[marker_pos..marker_pos + marker_len])
            .unwrap_or("Invalid UTF-8");

        assert_eq!(
            content,
            marker,
            "Marker {} mismatch at position {}. Expected '{}', found '{}'",
            i + 1,
            current_pos + marker_pos,
            marker,
            content
        );

        // Update position for next buffer
        current_pos += buffer_size;
    }

    // Helper function to verify the repeating ABCD pattern.
    let verify_pattern = |buf: &[u8], start: usize, end: usize, buffer_num: usize| {
        // Safety check for range validity
        if start >= end || end > buf.len() {
            println!(
                "Invalid range for buffer {}: start={}, end={}, len={}",
                buffer_num,
                start,
                end,
                buf.len()
            );
            return false;
        }

        let chunk = &buf[start..end];

        // Calculate the pattern offset for alignment.
        let pattern_offset = start % 4;
        let expected_pattern = [b'A', b'B', b'C', b'D'];

        // Verify each byte against the expected pattern at the correct offset.
        chunk.iter().enumerate().all(|(i, &byte)| {
            let expected = expected_pattern[(i + pattern_offset) % 4];
            if byte != expected {
                println!(
                    "Mismatch at position {}: expected {}, found {}",
                    start + i,
                    expected as char,
                    byte as char
                );
                false
            } else {
                true
            }
        })
    };

    // Adjust verification ranges and pattern alignment.
    for (i, buf) in buffers.iter().enumerate() {
        let buffer_num = i + 1;
        let buffer_size = buf.len();
        let marker_len = 11;

        // Calculate correct start position based on marker alignment.
        let start = if buffer_num == 1 { 0 } else { marker_len };
        let end = buffer_size - marker_len;

        assert!(
            verify_pattern(buf, start, end, buffer_num),
            "Pattern corruption detected in buffer {}. Expected aligned 'ABCD' pattern \
             in range {}..{}",
            buffer_num,
            start,
            end
        );
    }
}

/// Tests readv() system call behavior with EOF conditions and partial reads.
/// Uses a test file smaller than total buffer size to verify correct handling
/// of file boundaries and partial data transfers.
///
/// Verifies that:
/// - Partial reads near EOF work correctly
/// - Reading exactly at EOF returns 0
/// - Buffer contents match expected data
/// - Total bytes read matches available data
/// - Remaining buffer space is unmodified
fn test_readv_partial_and_eof() {
    // Let's create a file smaller than our total buffer sizes.
    // We'll use a structured pattern to make validation easier.
    let test_data = b"HEADER_DATA_SECTION_ONE_DATA_SECTION_TWO_END"; // 41 bytes
    let path = utils::prepare_with_content("partial_read_test.txt", test_data);

    // Test Case 1: Normal buffers larger than file size.
    {
        let mut first_buf = vec![0u8; 20]; // Should be filled completely
        let mut second_buf = vec![0u8; 20]; // Should be filled completely
        let mut third_buf = vec![0u8; 20]; // Should be partially filled

        let path_cstr = CString::new(path.to_str().unwrap()).unwrap();

        let bytes_read: usize = unsafe {
            let fd = libc::open(path_cstr.as_ptr(), libc::O_RDONLY);
            assert!(fd > 0, "Failed to open test file");

            let iov = [
                libc::iovec {
                    iov_base: first_buf.as_mut_ptr() as *mut libc::c_void,
                    iov_len: first_buf.len(),
                },
                libc::iovec {
                    iov_base: second_buf.as_mut_ptr() as *mut libc::c_void,
                    iov_len: second_buf.len(),
                },
                libc::iovec {
                    iov_base: third_buf.as_mut_ptr() as *mut libc::c_void,
                    iov_len: third_buf.len(),
                },
            ];

            let result = libc::readv(fd, iov.as_ptr(), iov.len() as i32);
            libc::close(fd);
            result.try_into().unwrap()
        };

        // Verify total bytes read matches file size.
        assert_eq!(
            bytes_read,
            test_data.len(),
            "Expected {} bytes read, got {}",
            test_data.len(),
            bytes_read
        );

        // Verify buffer contents
        assert_eq!(&first_buf[..20], &test_data[..20], "First buffer content mismatch");
        assert_eq!(&second_buf[..20], &test_data[20..40], "Second buffer content mismatch");
        assert_eq!(&third_buf[..1], &test_data[40..41], "Third buffer partial content mismatch");
    }

    // Test Case 2: Reading from an offset near EOF.
    {
        let mut first_buf = vec![0u8; 10];
        let mut second_buf = vec![0u8; 10];

        let path_cstr = CString::new(path.to_str().unwrap()).unwrap();

        let bytes_read: usize = unsafe {
            let fd = libc::open(path_cstr.as_ptr(), libc::O_RDONLY);
            assert!(fd > 0, "Failed to open test file");

            // Seek to near end of file
            // Use the platform-specific offset type directly
            let offset = LseekOffset::try_from(test_data.len() - 15).unwrap();
            let seek_result = seek(fd, offset);

            // Compare using the same types
            assert_eq!(LseekOffset::try_from(seek_result).unwrap(), offset);

            let iov = [
                libc::iovec {
                    iov_base: first_buf.as_mut_ptr() as *mut libc::c_void,
                    iov_len: first_buf.len(),
                },
                libc::iovec {
                    iov_base: second_buf.as_mut_ptr() as *mut libc::c_void,
                    iov_len: second_buf.len(),
                },
            ];

            let result = libc::readv(fd, iov.as_ptr(), iov.len() as i32);
            libc::close(fd);
            result.try_into().unwrap()
        };

        // Should read remaining 15 bytes
        assert_eq!(bytes_read, 15, "Expected 15 bytes read from offset, got {}", bytes_read);
    }

    // Test Case 3: Reading at EOF.
    {
        let mut buf = vec![0u8; 10];

        let path_cstr = CString::new(path.to_str().unwrap()).unwrap();

        let bytes_read: usize = unsafe {
            let fd = libc::open(path_cstr.as_ptr(), libc::O_RDONLY);
            assert!(fd > 0, "Failed to open test file");

            // Seek to EOF
            // Cast the offset to the appropriate type for the platform
            let offset = LseekOffset::try_from(test_data.len()).unwrap();
            let seek_result = seek(fd, offset);
            assert_eq!(
                LseekOffset::try_from(seek_result).unwrap(),
                LseekOffset::try_from(test_data.len()).unwrap()
            );

            let iov = [libc::iovec {
                iov_base: buf.as_mut_ptr() as *mut libc::c_void,
                iov_len: buf.len(),
            }];

            let result = libc::readv(fd, iov.as_ptr(), iov.len() as i32);
            libc::close(fd);
            result.try_into().unwrap()
        };

        // Should read 0 bytes at EOF
        assert_eq!(bytes_read, 0, "Expected 0 bytes read at EOF, got {}", bytes_read);
    }

    // Test Case 4: Small buffers with exact boundaries.
    {
        let mut first_buf = vec![0u8; 7]; // "HEADER_"
        let mut second_buf = vec![0u8; 5]; // "DATA_"
        let mut third_buf = vec![0u8; 7]; // "SECTION"

        let path_cstr = CString::new(path.to_str().unwrap()).unwrap();

        let bytes_read: usize = unsafe {
            let fd = libc::open(path_cstr.as_ptr(), libc::O_RDONLY);
            assert!(fd > 0, "Failed to open test file");

            let iov = [
                libc::iovec {
                    iov_base: first_buf.as_mut_ptr() as *mut libc::c_void,
                    iov_len: first_buf.len(),
                },
                libc::iovec {
                    iov_base: second_buf.as_mut_ptr() as *mut libc::c_void,
                    iov_len: second_buf.len(),
                },
                libc::iovec {
                    iov_base: third_buf.as_mut_ptr() as *mut libc::c_void,
                    iov_len: third_buf.len(),
                },
            ];

            let result = libc::readv(fd, iov.as_ptr(), iov.len() as i32);
            libc::close(fd);
            result.try_into().unwrap()
        };

        // Verify exact buffer fills.
        assert_eq!(
            bytes_read, 19,
            "Expected 19 bytes read for exact boundaries, got {}",
            bytes_read
        );
        assert_eq!(&first_buf, b"HEADER_", "First buffer exact content mismatch");
        assert_eq!(&second_buf, b"DATA_", "Second buffer exact content mismatch");
        assert_eq!(&third_buf[..7], b"SECTION", "Third buffer exact content mismatch");
    }
}

/// Tests error handling conditions of the readv() system call.
/// Verifies that the implementation properly handles various error scenarios
/// including invalid file descriptors,
///
/// Test coverage includes:
/// - Invalid file descriptor scenarios
fn test_readv_error_conditions() {
    #[cfg(any(target_os = "illumos", target_os = "solaris"))]
    use libc::___errno as __errno_location;
    #[cfg(target_os = "android")]
    use libc::__errno as __errno_location;
    #[cfg(target_os = "linux")]
    use libc::__errno_location;
    #[cfg(any(target_os = "freebsd", target_os = "macos"))]
    use libc::__error as __errno_location;

    // Test Case 1: Invalid File Descriptor Scenarios.
    {
        let mut buffer = vec![0u8; 10];

        // Create a single valid iovec structure for testing.
        let iov = [libc::iovec {
            iov_base: buffer.as_mut_ptr() as *mut libc::c_void,
            iov_len: buffer.len(),
        }];

        unsafe {
            // Test with negative file descriptor.
            let result = libc::readv(-1, iov.as_ptr(), 1);
            assert_eq!(result, -1, "Expected error for negative file descriptor");
            assert_eq!(
                *__errno_location(),
                libc::EBADF,
                "Expected EBADF for negative file descriptor"
            );

            // Test with unopened but potentially valid fd number.
            let result = libc::readv(999999, iov.as_ptr(), 1);
            assert_eq!(result, -1, "Expected error for invalid file descriptor");
            assert_eq!(
                *__errno_location(),
                libc::EBADF,
                "Expected EBADF for invalid file descriptor"
            );
        }
    }
}
