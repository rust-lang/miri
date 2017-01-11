#![feature(
    btree_range,
    collections,
    collections_bound,
    pub_restricted,
    rustc_private,
    i128_type,
)]

// From rustc.
#[macro_use]
extern crate log;
extern crate log_settings;
#[macro_use]
extern crate rustc;
extern crate rustc_borrowck;
extern crate rustc_const_math;
extern crate rustc_data_structures;
extern crate rustc_mir;
extern crate syntax;

// From crates.io.
extern crate byteorder;

mod cast;
mod error;
mod eval_context;
mod lvalue;
mod memory;
mod operator;
mod step;
mod terminator;
mod value;
mod vtable;

pub use error::{
    EvalError,
    EvalResult,
};

pub use eval_context::{
    EvalContext,
    Frame,
    ResourceLimits,
    StackPopCleanup,
    eval_main,
    run_mir_passes,
};

pub use lvalue::{
    Lvalue,
    LvalueExtra,
};

pub use memory::{
    AllocId,
    Memory,
    Pointer,
};

pub use value::{
    PrimVal,
    PrimValKind,
    Value,
};
