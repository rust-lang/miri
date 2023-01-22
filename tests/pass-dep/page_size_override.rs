//@compile-flags: -Zmiri-force-page-size=8
//@ignore-target-wasm: wasm does not have page_size

fn main() {
    let page_size = page_size::get();

    assert!(page_size == 8 * 1024, "8k page size override not respected: {}", page_size);
}
