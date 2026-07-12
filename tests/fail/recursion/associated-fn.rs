//@compile-flags: -Zno-recursion

struct Foo {
    depth: u64,
}

impl Foo {
    fn recurse(&mut self) {
        //~^ ERROR: recursive call
        eprintln!("recurse @ {}", self.depth);
        if self.depth > 0 {
            eprintln!("recursion not caught!");
            return;
        }
        self.depth += 1;
        self.recurse();
    }
}

fn main() {
    Foo { depth: 0 }.recurse();
}
