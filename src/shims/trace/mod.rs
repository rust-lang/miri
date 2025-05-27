mod child;
mod parent;

use std::ops::Range;

pub use self::child::{Supervisor, init_sv, register_retcode_sv};

/// The size used for the array into which we can move the stack pointer.
const FAKE_STACK_SIZE: usize = 1024;

/// Information needed to begin tracing.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct StartFfiInfo {
    /// A vector of page addresses. These should have been automatically obtained
    /// with `IsolatedAlloc::pages` and prepared with `IsolatedAlloc::prepare_ffi`.
    page_ptrs: Vec<u64>,
    /// The address of an allocation that can serve as a temporary stack.
    /// This should be a leaked `Box<[u8; FAKE_STACK_SIZE]>` cast to an int.
    stack_ptr: usize,
    //pid: i32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
enum TraceRequest {
    StartFfi(StartFfiInfo),
    OverrideRetcode(i32),
}

/// A single memory access, conservatively overestimated
/// in case of ambiguity.
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub enum AccessEvent {
    /// A read may have occurred on no more than the specified address range.
    Read(Range<u64>),
    /// A write may have occurred on no more than the specified address range.
    Write(Range<u64>),
}

/// The final results of an FFI trace, containing every relevant event detected
/// by the tracer.
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct MemEvents {
    /// An ordered list of memory accesses that occurred.
    pub acc_events: Vec<AccessEvent>,
    /// A value modulo which `AccessEvent` ranges stay the same length. Makes
    /// parsing the events a lot easier. Should likely just be the page size.
    pub alloc_cutoff: usize,
}
