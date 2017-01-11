// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//error-pattern: no mir for `std::

use std::sync::Mutex;

fn par_for<I, F>(iter: I, f: F)
    where I: Iterator,
          I::Item: Send,
          F: Fn(I::Item) + Sync
{
    for item in iter {
        f(item)
    }
}

fn sum(x: &[i32]) {
    let sum_lengths = Mutex::new(0);
    par_for(x.windows(4), |x| {
        *sum_lengths.lock().unwrap() += x.len()
    });

    assert_eq!(*sum_lengths.lock().unwrap(), (x.len() - 3) * 4);
}

fn main() {
    let mut elements = [0; 20];

    // iterators over references into this stack frame
    par_for(elements.iter_mut().enumerate(), |(i, x)| {
        *x = i as i32
    });

    sum(&elements)
}
