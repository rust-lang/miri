// Preparing for a rustc behavior change that'll happen soon: (FIXME remove this after the next submodule bump succeeded)
//@normalize-stderr-test: "`(ptr_offset_from_unsigned)`" -> "$1"
#![feature(ptr_sub_ptr)]

fn main() {
    let arr = [0u8; 8];
    let ptr1 = arr.as_ptr();
    let ptr2 = ptr1.wrapping_add(4);
    let _val = unsafe { ptr1.sub_ptr(ptr2) }; //~ERROR: first pointer has smaller offset than second: 0 < 4
}
