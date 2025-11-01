//@compile-flags: -Zmiri-tree-borrows -Zmiri-permissive-provenance

#[path = "../../../utils/mod.rs"]
mod utils;

#[allow(unused_variables, unused_assignments)]
fn main() {
    let mut x: u32 = 4;
    let int = {
        let y = &x;
        y as *const u32 as usize
    };
    // If y wasn't exposed, this would gc it.
    utils::run_provenance_gc();
    // This should disable y.
    x = 5;
    let wild = int as *const u32;

    let fail = unsafe { *wild }; //~ ERROR: /read access through wildcard at .* is forbidden/
}
