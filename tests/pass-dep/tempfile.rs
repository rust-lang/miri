//@ignore-target-windows: no libc on Windows
//@compile-flags: -Zmiri-disable-isolation

//! Test that the [`tempfile`] crate is compatible with miri.
fn main() {
    test_tempfile();
    test_tempfile_in();
}

fn tmp() -> PathBuf {
    std::env::var("MIRI_TEMP")
        .map(|tmp| {
            // MIRI_TEMP is set outside of our emulated
            // program, so it may have path separators that don't
            // correspond to our target platform. We normalize them here
            // before constructing a `PathBuf`

            #[cfg(windows)]
            return PathBuf::from(tmp.replace("/", "\\"));

            #[cfg(not(windows))]
            return PathBuf::from(tmp.replace("\\", "/"));
        })
        .unwrap_or_else(|_| std::env::temp_dir())
}

fn test_tempfile() {
    tempfile::tempfile().unwrap();
}

fn test_tempfile_in() {
    let dir_path = tmp();
    tempfile::tempfile_in(dir_path).unwrap();
}
