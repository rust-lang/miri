//@compile-flags: -Zno-recursion

struct Foo;

fn recurse<T>(depth: u64) {
    //~^ ERROR: recursive call
    eprintln!("recurse @ {depth}");
    if depth > 0 {
        eprintln!("recursion not caught!");
        return;
    }
    recurse::<Foo>(depth + 1);
}

fn main() {
    recurse::<Foo>(0);
}
