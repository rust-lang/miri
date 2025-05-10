//@compile-flags: -Zmiri-tree-borrows
#[path = "../../utils/mod.rs"]
#[macro_use]
mod utils;

// Counterpart to tests/fail/tree_borrows/cell-inside-struct.rs.
// The difference between them is that here we have a mutable
// reference to the struct instead a shared reference, so both fields
// can be mutated.
use std::cell::Cell;

struct Foo {
    field1: u32,
    field2: Cell<u32>,
}

pub fn main() {
    let mut root = Foo { field1: 42, field2: Cell::new(88) };
    unsafe {
        let a = &mut root;

        name!(a as *const Foo, "a");

        let a: *const Foo = a as *const Foo;
        let a: *mut Foo = a as *mut Foo;

        let alloc_id = alloc_id!(a);
        print_state!(alloc_id);

        // Writing to `field2`, which is interior mutable, should be allowed.
        (*a).field2.set(10);

        // Writing to `field1`, which is reserved, should be allowed.
        (*a).field1 = 88;
    }
}
