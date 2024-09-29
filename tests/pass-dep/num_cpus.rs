//@compile-flags: -Zmiri-disable-isolation
//@only-miri: fake cpu number

fn main() {
    assert_eq!(num_cpus::get(), 1);
}
