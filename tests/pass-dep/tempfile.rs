//@ignore-target: windows # File handling is not implemented yet
//@compile-flags: -Zmiri-disable-isolation

use std::fs;

#[path = "../utils/mod.rs"]
mod utils;

/// Test that the [`tempfile`] crate is compatible with miri for UNIX hosts and targets
fn main() {
    // Only create a file in our own tmp folder; the "host" temp folder
    // can be nonsensical for cross-tests.
    let dir_path = utils::tmp();

    // Create unnamed file.
    tempfile::tempfile_in(&dir_path).unwrap();

    // Create named file.
    let f = tempfile::NamedTempFile::new_in(&dir_path).unwrap();
    // FIXME: Cannot call `reopen` as that triggers UB (https://github.com/bytecodealliance/rustix/issues/1668)
    // f.reopen().unwrap();
    drop(f);

    // Create directory.
    let dir = tempfile::tempdir_in(&dir_path).unwrap();
    drop(fs::File::create(dir.path().join("file")).unwrap());
}
