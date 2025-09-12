//@revisions: stack tree
//@[tree]compile-flags: -Zmiri-tree-borrows-strong

fn may_insert_spurious_write(_x: &mut u32) {}

fn main() {
    let target = &mut 42;
    let target_alias = &*target;
    let target_alias_ptr = target_alias as *const _;
    may_insert_spurious_write(target);
    // now `target_alias` is invalid
    let _val = unsafe { *target_alias_ptr };
    //~[stack]^ ERROR: /read access .* tag does not exist in the borrow stack/
    //~[tree]| ERROR: /read access through .* is forbidden/
}
