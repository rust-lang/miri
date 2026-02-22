//@ignore-target: windows # No libc socket on Windows
//@compile-flags: -Zmiri-disable-isolation

fn main() {
    unsafe {
        let sockfd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        libc::close(sockfd);
    }
}
