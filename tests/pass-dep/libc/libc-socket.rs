//@ignore-target: windows # No libc socket on Windows
//@ignore-target: solaris # Does socket is a macro for __xnet7_socket which has no shim
//@ignore-target: illumos # Does socket is a macro for __xnet7_socket which has no shim
//@compile-flags: -Zmiri-disable-isolation

fn main() {
    unsafe {
        let sockfd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        libc::close(sockfd);
    }
}
