// Copyright 2012 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// FIXME: We handle uninitialized storage here, which currently makes validation fail.
// compile-flags: -Zmir-emit-validate=0

//ignore-msvc

#![feature(libc)]

#![allow(dead_code)]

extern crate libc;
use std::mem;

struct Arena(());

struct Bcx<'a> {
    fcx: &'a Fcx<'a>
}

struct Fcx<'a> {
    arena: &'a Arena,
    ccx: &'a Ccx
}

struct Ccx {
    x: isize
}

fn alloc<'a>(_bcx : &'a Arena) -> &'a Bcx<'a> {
    unsafe {
        mem::transmute(libc::malloc(mem::size_of::<Bcx<'a>>()
            as libc::size_t))
    }
}

fn h<'a>(bcx : &'a Bcx<'a>) -> &'a Bcx<'a> {
    return alloc(bcx.fcx.arena);
}

fn g(fcx : &Fcx) {
    let bcx = Bcx { fcx: fcx };
    let bcx2 = h(&bcx);
    unsafe {
        libc::free(mem::transmute(bcx2));
    }
}

fn f(ccx : &Ccx) {
    let a = Arena(());
    let fcx = Fcx { arena: &a, ccx: ccx };
    return g(&fcx);
}

pub fn main() {
    let ccx = Ccx { x: 0 };
    f(&ccx);
}
