//@compile-flags: -Zno-recursion

fn recurse(depth: u64) {
    //~^ ERROR: recursive call
    eprintln!("recurse @ {depth}");
    if depth > 0 {
        panic!("recursion not caught!");
    }
    recurse(depth + 1);
}

fn wrapper() {
    recurse(0);
}

fn main() {
    wrapper();
}
