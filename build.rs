use std::env::var;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(trace)");
    if var("CARGO_CFG_TARGET_OS").unwrap() == "linux"
        && var("CARGO_CFG_TARGET_ENV").unwrap() == "gnu"
        && (var("CARGO_CFG_TARGET_ARCH").unwrap() == "x86_64"
            || var("CARGO_CFG_TARGET_ARCH").unwrap() == "x86"
            || var("CARGO_CFG_TARGET_ARCH").unwrap() == "aarch64")
    {
        println!("cargo::rustc-cfg=trace");
    }
}
