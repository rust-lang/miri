//@ignore-target: windows # No libc socket on Windows
//@compile-flags: -Zmiri-disable-isolation

use std::net::TcpListener;

fn main() {
    test_create_ipv4_listener();
    test_create_ipv6_listener();
}

fn test_create_ipv4_listener() {
    let _listener_ipv4 = TcpListener::bind("127.0.0.1:0").unwrap();
}

fn test_create_ipv6_listener() {
    let _listener_ipv6 = TcpListener::bind("[::1]:0").unwrap();
}
