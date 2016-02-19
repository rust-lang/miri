#![feature(rustc_private)]

extern crate rustc;
extern crate rustc_mir;
extern crate syntax;
#[macro_use] extern crate log;

pub mod interpreter;
