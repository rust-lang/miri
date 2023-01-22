#![feature(extern_types)]
//@ignore-target-wasm: Enable this when we have a better way of turning on permissive provenance per-test.

extern "C" {
    type Foo;
}

fn main() {
    let x: &Foo = unsafe { &*(16 as *const Foo) };
    let _y: &Foo = &*x;
}
