//@check-pass
//@compile-flags: -Zno-recursion

struct Foo;
struct Bar;

fn recurse<T>(depth: u64) {
    eprintln!("recurse @ {depth}");
    if depth > 0 {
        eprintln!("recursion not caught!");
        return;
    }
    recurse::<Bar>(depth + 1);
}

fn main() {
    recurse::<Foo>(0);
}
