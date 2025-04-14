//@compile-flags: -Zmiri-tree-borrows

// Perform foreign access to a Cell and then local access.
// Takes advantage of two-phase borrows.

use std::cell::Cell;

pub trait Foo {
    fn lw_fr(&mut self, val: i32);
    fn lw_fw(&mut self, val: ());
    fn lr_fw(&mut self, val: ());
    fn lr_fr(&mut self, val: i32);
}

impl Foo for Cell<i32> {
    // local write after foreign read
    fn lw_fr(&mut self, val: i32) {
        self.set(val);
    }

    // local write after foreign write
    fn lw_fw(&mut self, _val: ()) {
        self.set(100);
    }

    // local read after foreign write
    fn lr_fw(&mut self, _val: ()) {
        self.get();
    }

    // local read after foreign read
    #[allow(path_statements)]
    fn lr_fr(&mut self, val: i32) {
        self.get();
        val;
    }
}

fn main() {
    let mut x: Cell<i32> = Cell::new(42);

    // Expands to something like
    //
    // let y = &twophase x;
    // let y1 = &*y;
    // let z = &x;
    // Cell::set(y1, Cell::get(z) + 1);
    //
    // Cell::get(z) is a foreign read for y1
    x.lw_fr(x.get() + 1);
    assert_eq!(x.get(), 43);
    x.lw_fw(x.set(1));
    assert_eq!(x.get(), 100);

    x.lr_fw(x.set(1));
    assert_eq!(x.get(), 1);

    x.lr_fr(x.get() + 1);
    assert_eq!(x.get(), 1);
}
