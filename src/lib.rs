#![feature(rustc_private)]
#![feature(map_try_insert)]
#![feature(never_type)]
#![feature(try_blocks)]
#![feature(io_error_more)]
#![feature(variant_count)]
#![feature(yeet_expr)]
#![feature(is_some_and)]
#![feature(nonzero_ops)]
#![feature(local_key_cell_methods)]
#![feature(is_terminal)]
// Configure clippy and other lints
#![allow(
    clippy::collapsible_else_if,
    clippy::collapsible_if,
    clippy::comparison_chain,
    clippy::enum_variant_names,
    clippy::field_reassign_with_default,
    clippy::manual_map,
    clippy::new_without_default,
    clippy::single_match,
    clippy::useless_format,
    clippy::derive_partial_eq_without_eq,
    clippy::derive_hash_xor_eq,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::single_element_loop,
    clippy::needless_return,
    clippy::bool_to_int_with_if,
    // We are not implementing queries here so it's fine
    rustc::potential_query_instability
)]
#![warn(
    rust_2018_idioms,
    clippy::cast_possible_wrap, // unsigned -> signed
    clippy::cast_sign_loss, // signed -> unsigned
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
)]
// Needed for rustdoc from bootstrap (with `-Znormalize-docs`).
#![recursion_limit = "256"]

extern crate rustc_apfloat;
extern crate rustc_ast;
#[macro_use]
extern crate rustc_middle;
extern crate rustc_const_eval;
extern crate rustc_data_structures;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_target;

mod borrow_tracker;
mod clock;
mod concurrency;
mod diagnostics;
mod eval;
mod helpers;
mod intptrcast;
mod machine;
mod mono_hash_map;
mod operator;
mod range_map;
mod shims;
mod tag_gc;

// Establish a "crate-wide prelude": we often import `crate::*`.

// Make all those symbols available in the same place as our own.
pub use rustc_const_eval::interpret::*;
// Resolve ambiguity.
pub use rustc_const_eval::interpret::{self, AllocMap, PlaceTy, Provenance as _};

pub use crate::shims::dlsym::{Dlsym, EvalContextExt as _};
pub use crate::shims::env::{EnvVars, EvalContextExt as _};
pub use crate::shims::foreign_items::EvalContextExt as _;
pub use crate::shims::intrinsics::EvalContextExt as _;
pub use crate::shims::os_str::EvalContextExt as _;
pub use crate::shims::panic::{CatchUnwindData, EvalContextExt as _};
pub use crate::shims::time::EvalContextExt as _;
pub use crate::shims::tls::TlsData;
pub use crate::shims::EvalContextExt as _;

pub use crate::borrow_tracker::stacked_borrows::{
    EvalContextExt as _, Item, Permission, Stack, Stacks,
};
pub use crate::borrow_tracker::{
    BorTag, BorrowTrackerMethod, CallId, EvalContextExt as _, RetagFields,
};
pub use crate::clock::{Clock, Instant};
pub use crate::concurrency::{
    data_race::{AtomicFenceOrd, AtomicReadOrd, AtomicRwOrd, AtomicWriteOrd, EvalContextExt as _},
    init_once::{EvalContextExt as _, InitOnceId},
    sync::{CondvarId, EvalContextExt as _, MutexId, RwLockId, SyncId},
    thread::{EvalContextExt as _, StackEmptyCallback, ThreadId, ThreadManager, Time},
};
pub use crate::diagnostics::{
    report_error, EvalContextExt as _, NonHaltingDiagnostic, TerminationInfo,
};
pub use crate::eval::{
    create_ecx, eval_entry, AlignmentCheck, BacktraceStyle, IsolatedOp, MiriConfig, RejectOpWith,
};
pub use crate::helpers::EvalContextExt as _;
pub use crate::intptrcast::ProvenanceMode;
pub use crate::machine::{
    AllocExtra, FrameExtra, MiriInterpCx, MiriInterpCxExt, MiriMachine, MiriMemoryKind,
    PrimitiveLayouts, Provenance, ProvenanceExtra, page_size, stack_addr, stack_size,
};
pub use crate::mono_hash_map::MonoHashMap;
pub use crate::operator::EvalContextExt as _;
pub use crate::range_map::RangeMap;
pub use crate::tag_gc::{EvalContextExt as _, VisitTags};

/// Insert rustc arguments at the beginning of the argument list that Miri wants to be
/// set per default, for maximal validation power.
pub const MIRI_DEFAULT_ARGS: &[&str] = &[
    "-Zalways-encode-mir",
    "-Zmir-emit-retag",
    "-Zmir-opt-level=0",
    "--cfg=miri",
    "-Cdebug-assertions=on",
    "-Zextra-const-ub-checks",
];
