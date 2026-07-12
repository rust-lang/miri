//@check-pass
// Note: -Zno-recursion is *NOT* passed.

fn recurse(depth: u64) {
    eprintln!("recurse @ {depth}");
    if depth > 0 {
        eprintln!("recursion not caught!");
        return;
    }
    recurse(depth + 1);
}

fn main() {
    recurse(0);
}
