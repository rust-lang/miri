//! This module contains everything needed to instantiate an interpreter.
//! This separation exists to ensure that no fancy miri features like
//! interpreting common C functions leak into CTFE.

use super::{EvalResult, EvalContext, Lvalue, PrimVal};

use rustc::{mir, ty};
use syntax::codemap::Span;

/// Methods of this trait signifies a point where CTFE evaluation would fail
/// and some use case dependent behaviour can instead be applied
pub trait Machine<'tcx>: Sized {
    /// Additional data that can be accessed via the EvalContext
    type Data;

    /// Additional data that can be accessed via the Memory
    type MemoryData;

    /// Additional memory kinds a machine wishes to distinguish from the builtin ones
    type MemoryKinds: ::std::fmt::Debug + PartialEq + Copy + Clone;

    /// Entry point to all function calls.
    ///
    /// Returns Ok(true) when the function was handled completely
    /// e.g. due to missing mir
    ///
    /// Returns Ok(false) if a new stack frame was pushed
    fn eval_fn_call<'a>(
        ecx: &mut EvalContext<'a, 'tcx, Self>,
        instance: ty::Instance<'tcx>,
        destination: Option<(Lvalue<'tcx>, mir::BasicBlock)>,
        arg_operands: &[mir::Operand<'tcx>],
        span: Span,
        sig: ty::FnSig<'tcx>,
    ) -> EvalResult<'tcx, bool>;

    /// directly process an intrinsic without pushing a stack frame.
    fn call_intrinsic<'a>(
        ecx: &mut EvalContext<'a, 'tcx, Self>,
        instance: ty::Instance<'tcx>,
        args: &[mir::Operand<'tcx>],
        dest: Lvalue<'tcx>,
        dest_ty: ty::Ty<'tcx>,
        dest_layout: &'tcx ty::layout::Layout,
        target: mir::BasicBlock,
    ) -> EvalResult<'tcx>;

    /// Called for all binary operations except on float types.
    ///
    /// Returns `None` if the operation should be handled by the integer
    /// op code in order to share more code between machines
    ///
    /// Returns a (value, overflowed) pair if the operation succeeded
    fn try_ptr_op<'a>(
        ecx: &EvalContext<'a, 'tcx, Self>,
        bin_op: mir::BinOp,
        left: PrimVal,
        left_ty: ty::Ty<'tcx>,
        right: PrimVal,
        right_ty: ty::Ty<'tcx>,
    ) -> EvalResult<'tcx, Option<(PrimVal, bool)>>;

    /// Called when trying to mark machine defined `MemoryKinds` as static
    fn mark_static_initialized(m: Self::MemoryKinds) -> EvalResult<'tcx>;

    /// Heap allocations via the `box` keyword
    ///
    /// Returns a pointer to the allocated memory
    fn box_alloc<'a>(
        ecx: &mut EvalContext<'a, 'tcx, Self>,
        ty: ty::Ty<'tcx>,
    ) -> EvalResult<'tcx, PrimVal>;
}
