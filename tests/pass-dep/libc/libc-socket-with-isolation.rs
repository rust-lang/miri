//@ignore-target: windows # No libc socket on Windows
//@compile-flags: -Zmiri-isolation-error=warn-nobacktrace

fn main() {
    unsafe {
        let sockfd = libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        libc::close(sockfd);
    }
}
