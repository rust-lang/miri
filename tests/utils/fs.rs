use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use super::miri_extern;

pub fn host_to_target_path(path: OsString) -> PathBuf {
    use std::ffi::{CStr, CString};

    // Once into_encoded_bytes is stable we can use it here.
    // (Unstable features would need feature flags in each test...)
    let path = CString::new(path.into_string().unwrap()).unwrap();
    let mut out = Vec::with_capacity(1024);

    unsafe {
        let ret =
            miri_extern::miri_host_to_target_path(path.as_ptr(), out.as_mut_ptr(), out.capacity());
        assert_eq!(ret, 0);
        // Here we panic if it's not UTF-8... but that is hard to avoid with OsStr APIs.
        let out = CStr::from_ptr(out.as_ptr()).to_str().unwrap();
        PathBuf::from(out)
    }
}

pub fn tmp() -> PathBuf {
    let path =
        std::env::var_os("MIRI_TEMP").unwrap_or_else(|| std::env::temp_dir().into_os_string());
    // These are host paths. We need to convert them to the target.
    host_to_target_path(path)
}

/// Prepare: compute filename and make sure the file does not exist.
pub fn prepare(filename: &str) -> PathBuf {
    let path = tmp().join(filename);
    // Clean the paths for robustness.
    fs::remove_file(&path).ok();
    path
}

/// Prepare like above, and also write some initial content to the file.
pub fn prepare_with_content(filename: &str, content: &[u8]) -> PathBuf {
    let path = prepare(filename);
    fs::write(&path, content).unwrap();
    path
}

/// Prepare directory: compute directory name and make sure it does not exist.
pub fn prepare_dir(dirname: &str) -> PathBuf {
    let path = tmp().join(&dirname);
    // Clean the directory for robustness.
    fs::remove_dir_all(&path).ok();
    path
}

/// Generates a test pattern with markers placed at buffer boundaries
///
/// Arguments:
/// * `buffer_sizes` - An array of buffer sizes that will be used in the readv operation
///
/// Returns:
/// * A vector containing the test pattern with markers placed at buffer boundaries
///
/// The function creates a pattern by:
/// 1. Filling the content with repeating "ABCD" sequences
/// 2. Placing markers at each buffer boundary
/// 3. Adding an end pattern to detect overruns
pub fn generate_test_pattern(buffer_sizes: &[usize]) -> Vec<u8> {
    // Calculate total size needed for all buffers.
    let total_size: usize = buffer_sizes.iter().sum();

    // Create our base content vector.
    let mut content = Vec::with_capacity(total_size);

    // Fill with repeating ABCD pattern.
    let base_pattern = b"ABCD";
    while content.len() < total_size {
        content.extend_from_slice(base_pattern);
    }
    content.truncate(total_size);

    // Calculate marker positions at buffer boundaries.
    // We'll accumulate sizes to find boundary positions.
    // Calculate correct marker positions based on cumulative buffer boundaries.
    let mut cumulative_position = 0;
    for (i, &buffer_size) in buffer_sizes.iter().enumerate() {
        let marker = format!("##MARKER{}##", i + 1).into_bytes();
        let marker_len = marker.len();

        // Position marker relative to the current buffer's end
        let marker_position = cumulative_position + buffer_size - marker_len;

        if marker_position + marker_len <= total_size {
            content[marker_position..marker_position + marker_len].copy_from_slice(&marker);
        }

        cumulative_position += buffer_size;
    }

    content
}
