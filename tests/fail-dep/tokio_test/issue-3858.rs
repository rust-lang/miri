//@only-target-linux: We only support tokio on Linux

// Test for ICE caused by weak epoll interest upgrade succeed, but attempt to retrieve
// epoll instance failed.
// https://github.com/rust-lang/miri/issues/3858
use tokio::fs;
use tokio::runtime::Handle;
use tokio::time::Duration;

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let _enter = rt.enter();
    rt.shutdown_timeout(Duration::from_secs(1000));

    let err: std::io::Error =
        Handle::current().block_on(fs::read_to_string("Cargo.toml")).unwrap_err();

    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    let inner_err = err.get_ref().expect("no inner error");
    assert_eq!(inner_err.to_string(), "background task failed");
}
