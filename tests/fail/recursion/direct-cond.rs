//@compile-flags: -Zno-recursion

fn recurse(depth: u64) {
    //~^ ERROR: recursive call
    eprintln!("recurse @ {depth}");
    if depth > 0 {
        panic!("recursion not caught!");
    }
    if std::env::args().len() == 1 {
        recurse(depth + 1);
    }
}

fn main() {
    recurse(0);
}
