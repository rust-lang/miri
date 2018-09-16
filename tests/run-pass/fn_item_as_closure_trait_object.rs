fn foo() {}

fn main1() {
    let f: &Fn() = &foo;
    f();
}

fn main2() {
  fn magic<F: FnOnce() -> i32>(f: F) -> F { f }
  let f = magic(|| 42) as fn() -> i32;
  assert_eq!(f(), 42);
}

fn main() {
    main1();
    main2();
}
