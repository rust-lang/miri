mod child;
pub mod messages;
mod parent;

use std::cell::RefCell;
use std::rc::Rc;

use rustc_const_eval::interpret::InterpResult;

pub use self::child::{Supervisor, init_sv, register_retcode_sv};
use crate::alloc::isolated_alloc::IsolatedAlloc;

/// The size of the temporary stack we use for callbacks that the server executes in the client.
/// This should be big enough that `mempr_on` and `mempr_off` can safely be jumped into with the
/// stack pointer pointing to a "stack" of this size without overflowing it.
const CALLBACK_STACK_SIZE: usize = 1024;

/// Wrapper type for what we get back from an FFI call; the former is its actual
/// return value, and the latter is the list of memory accesses that occurred during
/// this call.
pub type CallResult<'tcx> = InterpResult<'tcx, (crate::ImmTy<'tcx>, Option<messages::MemEvents>)>;

/// Performs an arbitrary FFI call, enabling tracing from the supervisor.
pub fn do_ffi<'tcx>(
    alloc: &Rc<RefCell<IsolatedAlloc>>,
    f: impl FnOnce() -> InterpResult<'tcx, crate::ImmTy<'tcx>>,
) -> CallResult<'tcx> {
    // SAFETY: We don't touch the machine memory past this point.
    let guard = unsafe { Supervisor::start_ffi(alloc) };

    f().map(|v| {
        let memevents = unsafe { Supervisor::end_ffi(guard) };
        (v, memevents)
    })
}
