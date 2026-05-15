//@compile-flags: -Zmiri-disable-isolation
//@only-target: linux
//@ignore-host: windows

use std::fs;

#[path = "../../utils/mod.rs"]
mod utils;

fn main() {
    let bytes = b"Hello, copied World!\n";
    let src = utils::prepare_with_content("miri_test_fs_copy_source.txt", bytes);
    let dst = utils::prepare("miri_test_fs_copy_destination.txt");

    let copied = fs::copy(&src, &dst).unwrap();
    assert_eq!(copied, bytes.len() as u64);
    assert_eq!(fs::read(&dst).unwrap(), bytes);

    let new_bytes = b"short";
    fs::write(&src, new_bytes).unwrap();
    let copied = fs::copy(&src, &dst).unwrap();
    assert_eq!(copied, new_bytes.len() as u64);
    assert_eq!(fs::read(&dst).unwrap(), new_bytes);

    fs::remove_file(&src).unwrap();
    fs::remove_file(&dst).unwrap();
}
