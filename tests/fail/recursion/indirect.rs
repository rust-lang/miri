//@compile-flags: -Zno-recursion

fn indirection(depth: u64) {
    indirectly_recurse(depth + 1);
}

fn indirectly_recurse(depth: u64) {
    //~^ ERROR: recursive call
    eprintln!("recurse @ {depth}");
    if depth > 0 {
        panic!("recursion not caught!");
    }
    indirection(depth);
}

fn main() {
    indirectly_recurse(0);
}
