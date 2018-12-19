fn f() -> i32 {
    42
}

fn return_fn_ptr() -> fn() -> i32 {
    f
}

fn main() {
    assert!(return_fn_ptr() != f);
}
