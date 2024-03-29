//@ignore-target-windows: no libc on Windows
//@compile-flags: -Zmiri-disable-isolation

#![feature(io_error_more)]
#![feature(io_error_uncategorized)]

#[path = "../../utils/mod.rs"]
mod utils;

fn test_socket(
    socket: unsafe extern "C" fn(
        domain: libc::c_int,
        type_: libc::c_int,
        protocol: libc::c_int,
    ) -> libc::c_int,
) {
    // libc::c_int is i32 in target i686-unknown-linux-gnu
    // https://docs.rs/libc/latest/i686-unknown-linux-gnu/libc/type.c_int.html

    let domain = 0;
    let type_ = 0;
    let protocol = 0;

    let fd = unsafe { libc::socket(domain, type_, protocol) };

    // 0 - stdin, 1 - stdout, 2 - stderr; and all fd's are sequential upon first issue
    assert_eq!(fd, 3);
    let res = unsafe { socket(domain, type_, protocol) };
    assert_eq!(res, 4);
}
fn main() {
    #[cfg(target_os = "linux")]
    test_socket(libc::socket);
}
