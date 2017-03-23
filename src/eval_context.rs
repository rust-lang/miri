use std::cell::Ref;
use std::collections::HashMap;
use std::fmt::Write;

use rustc::hir::def_id::DefId;
use rustc::hir::map::definitions::DefPathData;
use rustc::middle::const_val::ConstVal;
use rustc::mir;
use rustc::traits::Reveal;
use rustc::ty::layout::{self, Layout, Size};
use rustc::ty::subst::{self, Subst, Substs};
use rustc::ty::{self, Ty, TyCtxt, TypeFoldable, Binder};
use rustc_data_structures::indexed_vec::Idx;
use rustc_const_math::ConstInt;
use syntax::codemap::{self, DUMMY_SP};

use error::{EvalError, EvalResult};
use lvalue::{Global, GlobalId, Lvalue, LvalueExtra};
use memory::{Memory, Pointer};
use operator;
use value::{PrimVal, PrimValKind, Value, ValueKind};

pub type MirRef<'tcx> = Ref<'tcx, mir::Mir<'tcx>>;

pub struct EvalContext<'a, 'tcx: 'a> {
    /// The results of the type checker, from rustc.
    pub(crate) tcx: TyCtxt<'a, 'tcx, 'tcx>,

    /// The virtual memory system.
    pub(crate) memory: Memory<'a, 'tcx>,

    /// Precomputed statics, constants and promoteds.
    pub(crate) globals: HashMap<GlobalId<'tcx>, Global<'tcx>>,

    /// The virtual call stack.
    pub(crate) stack: Vec<Frame<'tcx>>,

    /// The maximum number of stack frames allowed
    pub(crate) stack_limit: usize,

    /// The maximum number of operations that may be executed.
    /// This prevents infinite loops and huge computations from freezing up const eval.
    /// Remove once halting problem is solved.
    pub(crate) steps_remaining: u64,
}

/// A stack frame.
pub struct Frame<'tcx> {
    ////////////////////////////////////////////////////////////////////////////////
    // Function and callsite information
    ////////////////////////////////////////////////////////////////////////////////

    /// The MIR for the function called on this frame.
    pub mir: MirRef<'tcx>,

    /// The def_id of the current function.
    pub def_id: DefId,

    /// type substitutions for the current function invocation.
    pub substs: &'tcx Substs<'tcx>,

    /// The span of the call site.
    pub span: codemap::Span,

    ////////////////////////////////////////////////////////////////////////////////
    // Return lvalue and locals
    ////////////////////////////////////////////////////////////////////////////////

    /// The block to return to when returning from the current stack frame
    pub return_to_block: StackPopCleanup,

    /// The location where the result of the current stack frame should be written to.
    pub return_lvalue: Lvalue<'tcx>,

    /// The list of locals for this stack frame, stored in order as
    /// `[arguments..., variables..., temporaries...]`. The locals are stored as `Value`s, which
    /// can either directly contain `PrimVal` or refer to some part of an `Allocation`.
    ///
    /// Before being initialized, all locals are `Value::ByVal(PrimVal::Undef)`.
    pub locals: Vec<Value>,

    /// Temporary allocations introduced to save stackframes
    /// This is pure interpreter magic and has nothing to do with how rustc does it
    /// An example is calling an FnMut closure that has been converted to a FnOnce closure
    /// The value's destructor will be called and the memory freed when the stackframe finishes
    pub interpreter_temporaries: Vec<(Pointer, Ty<'tcx>)>,

    ////////////////////////////////////////////////////////////////////////////////
    // Current position within the function
    ////////////////////////////////////////////////////////////////////////////////

    /// The block that is currently executed (or will be executed after the above call stacks
    /// return).
    pub block: mir::BasicBlock,

    /// The index of the currently evaluated statment.
    pub stmt: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum StackPopCleanup {
    /// The stackframe existed to compute the initial value of a static/constant, make sure it
    /// isn't modifyable afterwards in case of constants.
    /// In case of `static mut`, mark the memory to ensure it's never marked as immutable through
    /// references or deallocated
    /// The bool decides whether the value is mutable (true) or not (false)
    MarkStatic(bool),
    /// A regular stackframe added due to a function call will need to get forwarded to the next
    /// block
    Goto(mir::BasicBlock),
    /// The main function and diverging functions have nowhere to return to
    None,
}

#[derive(Copy, Clone, Debug)]
pub struct ResourceLimits {
    pub memory_size: u64,
    pub step_limit: u64,
    pub stack_limit: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        ResourceLimits {
            memory_size: 100 * 1024 * 1024, // 100 MB
            step_limit: 1_000_000,
            stack_limit: 100,
        }
    }
}

impl<'a, 'tcx> EvalContext<'a, 'tcx> {
    pub fn new(tcx: TyCtxt<'a, 'tcx, 'tcx>, limits: ResourceLimits) -> Self {
        EvalContext {
            tcx,
            memory: Memory::new(&tcx.data_layout, limits.memory_size),
            globals: HashMap::new(),
            stack: Vec::new(),
            stack_limit: limits.stack_limit,
            steps_remaining: limits.step_limit,
        }
    }

    pub fn alloc_ptr(&mut self, ty: Ty<'tcx>) -> EvalResult<'tcx, Pointer> {
        let substs = self.substs();
        self.alloc_ptr_with_substs(ty, substs)
    }

    pub fn alloc_ptr_with_substs(
        &mut self,
        ty: Ty<'tcx>,
        substs: &'tcx Substs<'tcx>
    ) -> EvalResult<'tcx, Pointer> {
        let size = self.type_size_with_substs(ty, substs)?.expect("cannot alloc memory for unsized type");
        let align = self.type_align_with_substs(ty, substs)?;
        self.memory.allocate(size, align)
    }

    pub fn memory(&self) -> &Memory<'a, 'tcx> {
        &self.memory
    }

    pub fn memory_mut(&mut self) -> &mut Memory<'a, 'tcx> {
        &mut self.memory
    }

    pub fn stack(&self) -> &[Frame<'tcx>] {
        &self.stack
    }

    pub(crate) fn str_to_value(&mut self, s: &str) -> EvalResult<'tcx, Value> {
        let ptr = self.memory.allocate_cached(s.as_bytes())?;
        Ok(Value::ByValPair(PrimVal::Ptr(ptr), PrimVal::from_u128(s.len() as u128)))
    }

    pub(super) fn const_to_value(&mut self, const_val: &ConstVal) -> EvalResult<'tcx, Value> {
        use rustc::middle::const_val::ConstVal::*;
        use rustc_const_math::ConstFloat;

        let primval = match *const_val {
            Integral(const_int) => PrimVal::Bytes(const_int.to_u128_unchecked()),

            Float(ConstFloat::F32(f)) => PrimVal::from_f32(f),
            Float(ConstFloat::F64(f)) => PrimVal::from_f64(f),

            Bool(b) => PrimVal::from_bool(b),
            Char(c) => PrimVal::from_char(c),

            Str(ref s) => return self.str_to_value(s),

            ByteStr(ref bs) => {
                let ptr = self.memory.allocate_cached(bs)?;
                PrimVal::Ptr(ptr)
            }

            Struct(_)    => unimplemented!(),
            Tuple(_)     => unimplemented!(),
            Function(_, _)  => unimplemented!(),
            Array(_)     => unimplemented!(),
            Repeat(_, _) => unimplemented!(),
        };

        Ok(Value::ByVal(primval))
    }

    pub(super) fn type_is_sized(&self, ty: Ty<'tcx>) -> bool {
        // generics are weird, don't run this function on a generic
        assert!(!ty.needs_subst());
        ty.is_sized(self.tcx, &self.tcx.empty_parameter_environment(), DUMMY_SP)
    }

    pub fn load_mir(&self, def_id: DefId) -> EvalResult<'tcx, MirRef<'tcx>> {
        trace!("load mir {:?}", def_id);
        if def_id.is_local() || self.tcx.sess.cstore.is_item_mir_available(def_id) {
            Ok(self.tcx.item_mir(def_id))
        } else {
            Err(EvalError::NoMirFor(self.tcx.item_path_str(def_id)))
        }
    }

    pub fn monomorphize(&self, ty: Ty<'tcx>, substs: &'tcx Substs<'tcx>) -> Ty<'tcx> {
        // miri doesn't care about lifetimes, and will choke on some crazy ones
        // let's simply get rid of them
        let without_lifetimes = self.tcx.erase_regions(&ty);
        let substituted = without_lifetimes.subst(self.tcx, substs);
        self.tcx.normalize_associated_type(&substituted)
    }

    pub fn erase_lifetimes<T>(&self, value: &Binder<T>) -> T
        where T : TypeFoldable<'tcx>
    {
        let value = self.tcx.erase_late_bound_regions(value);
        self.tcx.erase_regions(&value)
    }

    pub(super) fn type_size(&self, ty: Ty<'tcx>) -> EvalResult<'tcx, Option<u64>> {
        self.type_size_with_substs(ty, self.substs())
    }

    pub(super) fn type_align(&self, ty: Ty<'tcx>) -> EvalResult<'tcx, u64> {
        self.type_align_with_substs(ty, self.substs())
    }

    fn type_size_with_substs(
        &self,
        ty: Ty<'tcx>,
        substs: &'tcx Substs<'tcx>,
    ) -> EvalResult<'tcx, Option<u64>> {
        let layout = self.type_layout_with_substs(ty, substs)?;
        if layout.is_unsized() {
            Ok(None)
        } else {
            Ok(Some(layout.size(&self.tcx.data_layout).bytes()))
        }
    }

    fn type_align_with_substs(&self, ty: Ty<'tcx>, substs: &'tcx Substs<'tcx>) -> EvalResult<'tcx, u64> {
        self.type_layout_with_substs(ty, substs).map(|layout| layout.align(&self.tcx.data_layout).abi())
    }

    pub(super) fn type_layout(&self, ty: Ty<'tcx>) -> EvalResult<'tcx, &'tcx Layout> {
        self.type_layout_with_substs(ty, self.substs())
    }

    fn type_layout_with_substs(&self, ty: Ty<'tcx>, substs: &'tcx Substs<'tcx>) -> EvalResult<'tcx, &'tcx Layout> {
        // TODO(solson): Is this inefficient? Needs investigation.
        let ty = self.monomorphize(ty, substs);

        self.tcx.infer_ctxt((), Reveal::All).enter(|infcx| {
            ty.layout(&infcx).map_err(EvalError::Layout)
        })
    }

    pub fn push_stack_frame(
        &mut self,
        def_id: DefId,
        span: codemap::Span,
        mir: MirRef<'tcx>,
        substs: &'tcx Substs<'tcx>,
        return_lvalue: Lvalue<'tcx>,
        return_to_block: StackPopCleanup,
        temporaries: Vec<(Pointer, Ty<'tcx>)>,
    ) -> EvalResult<'tcx> {
        ::log_settings::settings().indentation += 1;

        // Subtract 1 because `local_decls` includes the ReturnPointer, but we don't store a local
        // `Value` for that.
        let num_locals = mir.local_decls.len() - 1;
        let locals = vec![Value::ByVal(PrimVal::Undef); num_locals];

        self.stack.push(Frame {
            mir,
            block: mir::START_BLOCK,
            return_to_block,
            return_lvalue,
            locals,
            interpreter_temporaries: temporaries,
            span,
            def_id,
            substs,
            stmt: 0,
        });

        if self.stack.len() > self.stack_limit {
            Err(EvalError::StackFrameLimitReached)
        } else {
            Ok(())
        }
    }

    pub(super) fn pop_stack_frame(&mut self) -> EvalResult<'tcx> {
        ::log_settings::settings().indentation -= 1;
        let frame = self.stack.pop().expect("tried to pop a stack frame, but there were none");
        match frame.return_to_block {
            StackPopCleanup::MarkStatic(mutable) => if let Lvalue::Global(id) = frame.return_lvalue {
                let global_value = self.globals.get_mut(&id)
                    .expect("global should have been cached (static)");
                match global_value.value {
                    Value::ByRef(ptr) => self.memory.mark_static_initalized(ptr.alloc_id, mutable)?,
                    Value::ByVal(val) => if let PrimVal::Ptr(ptr) = val {
                        self.memory.mark_inner_allocation(ptr.alloc_id, mutable)?;
                    },
                    Value::ByValPair(val1, val2) => {
                        if let PrimVal::Ptr(ptr) = val1 {
                            self.memory.mark_inner_allocation(ptr.alloc_id, mutable)?;
                        }
                        if let PrimVal::Ptr(ptr) = val2 {
                            self.memory.mark_inner_allocation(ptr.alloc_id, mutable)?;
                        }
                    },
                }
                // see comment on `initialized` field
                assert!(!global_value.initialized);
                global_value.initialized = true;
                assert!(global_value.mutable);
                global_value.mutable = mutable;
            } else {
                bug!("StackPopCleanup::MarkStatic on: {:?}", frame.return_lvalue);
            },
            StackPopCleanup::Goto(target) => self.goto_block(target),
            StackPopCleanup::None => {},
        }
        // deallocate all locals that are backed by an allocation
        for local in frame.locals {
            if let Value::ByRef(ptr) = local {
                trace!("deallocating local");
                self.memory.dump_alloc(ptr.alloc_id);
                match self.memory.deallocate(ptr) {
                    // We could alternatively check whether the alloc_id is static before calling
                    // deallocate, but this is much simpler and is probably the rare case.
                    Ok(()) | Err(EvalError::DeallocatedStaticMemory) => {},
                    other => return other,
                }
            }
        }
        // drop and deallocate all temporary allocations
        for (ptr, ty) in frame.interpreter_temporaries {
            trace!("dropping temporary allocation");
            let mut drops = Vec::new();
            self.drop(Lvalue::from_ptr(ptr), ty, &mut drops)?;
            self.eval_drop_impls(drops, frame.span)?;
        }
        Ok(())
    }

    pub fn assign_discr_and_fields<
        V: IntoValTyPair<'tcx>,
        J: IntoIterator<Item = V>,
    >(
        &mut self,
        dest: Lvalue<'tcx>,
        dest_ty: Ty<'tcx>,
        discr_offset: u64,
        operands: J,
        discr_val: u128,
        variant_idx: usize,
        discr_size: u64,
    ) -> EvalResult<'tcx>
        where J::IntoIter: ExactSizeIterator,
    {
        // FIXME(solson)
        let dest_ptr = self.force_allocation(dest)?.to_ptr();

        let discr_dest = dest_ptr.offset(discr_offset);
        self.memory.write_uint(discr_dest, discr_val, discr_size)?;

        let dest = Lvalue::Ptr {
            ptr: dest_ptr,
            extra: LvalueExtra::DowncastVariant(variant_idx),
        };

        self.assign_fields(dest, dest_ty, operands)
    }

    pub fn assign_fields<
        V: IntoValTyPair<'tcx>,
        J: IntoIterator<Item = V>,
    >(
        &mut self,
        dest: Lvalue<'tcx>,
        dest_ty: Ty<'tcx>,
        operands: J,
    ) -> EvalResult<'tcx>
        where J::IntoIter: ExactSizeIterator,
    {
        if self.type_size(dest_ty)? == Some(0) {
            // zst assigning is a nop
            return Ok(());
        }
        match self.ty_to_value_kind(dest_ty) {
            ValueKind::Ref => {
                trace!("assign fields ref");
                for (field_index, operand) in operands.into_iter().enumerate() {
                    let (value, value_ty) = operand.into_val_ty_pair(self)?;
                    let field_dest = self.lvalue_field(dest, field_index, dest_ty, value_ty)?;
                    self.write_value(value, field_dest, value_ty)?;
                }
                Ok(())
            },
            ValueKind::Val(_) => {
                trace!("assign fields val");
                let mut iter = operands.into_iter();
                assert_eq!(iter.len(), 1);
                let (value, value_ty) = iter.next().unwrap().into_val_ty_pair(self)?;
                self.write_value(value, dest, value_ty)
            },
            ValueKind::ValPair(_, _) => {
                trace!("assign fields pair");
                let mut iter = operands.into_iter();
                let (a, a_ty) = iter.next().unwrap().into_val_ty_pair(self)?;
                match self.ty_to_value_kind(a_ty) {
                    ValueKind::Ref => bug!("ty_to_value_kind broken: field of ValPair is Ref"),
                    ValueKind::Val(_) => {
                        let (b, b_ty) = iter.next().unwrap().into_val_ty_pair(self)?;
                        assert!(iter.is_empty());
                        let a = self.value_to_primval(a, a_ty)?;
                        let b = self.value_to_primval(b, b_ty)?;
                        self.write_value(Value::ByValPair(a, b), dest, dest_ty)
                    },
                    ValueKind::ValPair(_, _) => {
                        assert!(iter.is_empty());
                        self.write_value(a, dest, dest_ty)
                    }
                }
            }
        }
    }

    /// Evaluate an assignment statement.
    ///
    /// There is no separate `eval_rvalue` function. Instead, the code for handling each rvalue
    /// type writes its results directly into the memory specified by the lvalue.
    pub(super) fn eval_rvalue_into_lvalue(
        &mut self,
        rvalue: &mir::Rvalue<'tcx>,
        lvalue: &mir::Lvalue<'tcx>,
    ) -> EvalResult<'tcx> {
        let dest = self.eval_lvalue(lvalue)?;
        let dest_ty = self.lvalue_ty(lvalue);
        let dest_layout = self.type_layout(dest_ty)?;

        use rustc::mir::Rvalue::*;
        match *rvalue {
            Use(ref operand) => {
                let value = self.eval_operand(operand)?;
                self.write_value(value, dest, dest_ty)?;
            }

            BinaryOp(bin_op, ref left, ref right) => {
                // ignore overflow bit, rustc inserts check branches for us
                self.intrinsic_overflowing(bin_op, left, right, dest, dest_ty)?;
            }

            CheckedBinaryOp(bin_op, ref left, ref right) => {
                self.intrinsic_with_overflow(bin_op, left, right, dest, dest_ty)?;
            }

            UnaryOp(un_op, ref operand) => {
                let val = self.eval_operand_to_primval(operand)?;
                let kind = self.ty_to_primval_kind(dest_ty)?;
                self.write_primval(dest, operator::unary_op(un_op, val, kind)?, dest_ty)?;
            }

            // Skip everything for zsts
            Aggregate(..) if self.type_size(dest_ty)? == Some(0) => {}

            Aggregate(ref kind, ref operands) => {
                self.inc_step_counter_and_check_limit(operands.len() as u64)?;
                use rustc::ty::layout::Layout::*;
                match *dest_layout {
                    Univariant { ref variant, .. } => {
                        if variant.packed {
                            let ptr = self.force_allocation(dest)?.to_ptr_and_extra().0;
                            self.memory.mark_packed(ptr, variant.stride().bytes());
                        }
                        self.assign_fields(dest, dest_ty, operands)?;
                    }

                    Array { .. } => {
                        self.assign_fields(dest, dest_ty, operands)?;
                    }

                    General { discr, ref variants, .. } => {
                        if let mir::AggregateKind::Adt(adt_def, variant, _, _) = *kind {
                            let discr_val = adt_def.discriminants(self.tcx)
                                .nth(variant)
                                .expect("broken mir: Adt variant id invalid")
                                .to_u128_unchecked();
                            let discr_size = discr.size().bytes();
                            if variants[variant].packed {
                                let ptr = self.force_allocation(dest)?.to_ptr_and_extra().0;
                                self.memory.mark_packed(ptr, variants[variant].stride().bytes());
                            }

                            self.assign_discr_and_fields(
                                dest,
                                dest_ty,
                                variants[variant].offsets[0].bytes(),
                                operands,
                                discr_val,
                                variant,
                                discr_size,
                            )?;
                        } else {
                            bug!("tried to assign {:?} to Layout::General", kind);
                        }
                    }

                    RawNullablePointer { nndiscr, .. } => {
                        if let mir::AggregateKind::Adt(_, variant, _, _) = *kind {
                            if nndiscr == variant as u64 {
                                assert_eq!(operands.len(), 1);
                                let operand = &operands[0];
                                let value = self.eval_operand(operand)?;
                                let value_ty = self.operand_ty(operand);
                                self.write_value(value, dest, value_ty)?;
                            } else {
                                if let Some(operand) = operands.get(0) {
                                    assert_eq!(operands.len(), 1);
                                    let operand_ty = self.operand_ty(operand);
                                    assert_eq!(self.type_size(operand_ty)?, Some(0));
                                }
                                self.write_primval(dest, PrimVal::Bytes(0), dest_ty)?;
                            }
                        } else {
                            bug!("tried to assign {:?} to Layout::RawNullablePointer", kind);
                        }
                    }

                    StructWrappedNullablePointer { nndiscr, ref nonnull, ref discrfield, .. } => {
                        if let mir::AggregateKind::Adt(_, variant, _, _) = *kind {
                            if nonnull.packed {
                                let ptr = self.force_allocation(dest)?.to_ptr_and_extra().0;
                                self.memory.mark_packed(ptr, nonnull.stride().bytes());
                            }
                            if nndiscr == variant as u64 {
                                self.assign_fields(dest, dest_ty, operands)?;
                            } else {
                                for operand in operands {
                                    let operand_ty = self.operand_ty(operand);
                                    assert_eq!(self.type_size(operand_ty)?, Some(0));
                                }
                                let (offset, ty) = self.nonnull_offset_and_ty(dest_ty, nndiscr, discrfield)?;

                                // FIXME(solson)
                                let dest = self.force_allocation(dest)?.to_ptr();

                                let dest = dest.offset(offset.bytes());
                                let dest_size = self.type_size(ty)?
                                    .expect("bad StructWrappedNullablePointer discrfield");
                                self.memory.write_int(dest, 0, dest_size)?;
                            }
                        } else {
                            bug!("tried to assign {:?} to Layout::RawNullablePointer", kind);
                        }
                    }

                    CEnum { .. } => {
                        assert_eq!(operands.len(), 0);
                        if let mir::AggregateKind::Adt(adt_def, variant, _, _) = *kind {
                            let n = adt_def.discriminants(self.tcx)
                                .nth(variant)
                                .expect("broken mir: Adt variant index invalid")
                                .to_u128_unchecked();
                            self.write_primval(dest, PrimVal::Bytes(n), dest_ty)?;
                        } else {
                            bug!("tried to assign {:?} to Layout::CEnum", kind);
                        }
                    }

                    Vector { count, .. } => {
                        debug_assert_eq!(count, operands.len() as u64);
                        self.assign_fields(dest, dest_ty, operands)?;
                    }

                    UntaggedUnion { .. } => {
                        assert_eq!(operands.len(), 1);
                        let operand = &operands[0];
                        let value = self.eval_operand(operand)?;
                        let value_ty = self.operand_ty(operand);
                        self.write_value(value, dest, value_ty)?;
                    }

                    _ => {
                        return Err(EvalError::Unimplemented(format!(
                            "can't handle destination layout {:?} when assigning {:?}",
                            dest_layout,
                            kind
                        )));
                    }
                }
            }

            Repeat(ref operand, _) => {
                let (elem_ty, length) = match dest_ty.sty {
                    ty::TyArray(elem_ty, n) => (elem_ty, n as u64),
                    _ => bug!("tried to assign array-repeat to non-array type {:?}", dest_ty),
                };
                self.inc_step_counter_and_check_limit(length)?;
                let elem_size = self.type_size(elem_ty)?
                    .expect("repeat element type must be sized");
                let value = self.eval_operand(operand)?;

                // FIXME(solson)
                let dest = self.force_allocation(dest)?.to_ptr();

                for i in 0..length {
                    let elem_dest = dest.offset(i * elem_size);
                    self.write_value_to_ptr(value, elem_dest, elem_ty)?;
                }
            }

            Len(ref lvalue) => {
                let src = self.eval_lvalue(lvalue)?;
                let ty = self.lvalue_ty(lvalue);
                let (_, len) = src.elem_ty_and_len(ty);
                self.write_primval(dest, PrimVal::from_u128(len as u128), dest_ty)?;
            }

            Ref(_, _, ref lvalue) => {
                let src = self.eval_lvalue(lvalue)?;
                let (raw_ptr, extra) = self.force_allocation(src)?.to_ptr_and_extra();
                let ptr = PrimVal::Ptr(raw_ptr);

                let val = match extra {
                    LvalueExtra::None => Value::ByVal(ptr),
                    LvalueExtra::Length(len) => Value::ByValPair(ptr, PrimVal::from_u128(len as u128)),
                    LvalueExtra::Vtable(vtable) => Value::ByValPair(ptr, PrimVal::Ptr(vtable)),
                    LvalueExtra::DowncastVariant(..) =>
                        bug!("attempted to take a reference to an enum downcast lvalue"),
                };

                self.write_value(val, dest, dest_ty)?;
            }

            Box(ty) => {
                let ptr = self.alloc_ptr(ty)?;
                self.write_primval(dest, PrimVal::Ptr(ptr), dest_ty)?;
            }

            Cast(kind, ref operand, cast_ty) => {
                debug_assert_eq!(self.monomorphize(cast_ty, self.substs()), dest_ty);
                use rustc::mir::CastKind::*;
                match kind {
                    Unsize => {
                        let src = self.eval_operand(operand)?;
                        let src_ty = self.operand_ty(operand);
                        self.unsize_into(src, src_ty, dest, dest_ty)?;
                    }

                    Misc => {
                        let src = self.eval_operand(operand)?;
                        let src_ty = self.operand_ty(operand);
                        if self.type_is_fat_ptr(src_ty) {
                            trace!("misc cast: {:?}", src);
                            match (src, self.type_is_fat_ptr(dest_ty)) {
                                (Value::ByRef(_), _) |
                                (Value::ByValPair(..), true) => {
                                    self.write_value(src, dest, dest_ty)?;
                                },
                                (Value::ByValPair(data, _), false) => {
                                    self.write_value(Value::ByVal(data), dest, dest_ty)?;
                                },
                                (Value::ByVal(_), _) => bug!("expected fat ptr"),
                            }
                        } else {
                            let src_val = self.value_to_primval(src, src_ty)?;
                            let dest_val = self.cast_primval(src_val, src_ty, dest_ty)?;
                            self.write_value(Value::ByVal(dest_val), dest, dest_ty)?;
                        }
                    }

                    ReifyFnPointer => match self.operand_ty(operand).sty {
                        ty::TyFnDef(def_id, substs, sig) => {
                            let fn_ptr = self.memory.create_fn_ptr(def_id, substs, sig);
                            self.write_value(Value::ByVal(PrimVal::Ptr(fn_ptr)), dest, dest_ty)?;
                        },
                        ref other => bug!("reify fn pointer on {:?}", other),
                    },

                    UnsafeFnPointer => match dest_ty.sty {
                        ty::TyFnPtr(_) => {
                            let src = self.eval_operand(operand)?;
                            self.write_value(src, dest, dest_ty)?;
                        },
                        ref other => bug!("fn to unsafe fn cast on {:?}", other),
                    },

                    ClosureFnPointer => match self.operand_ty(operand).sty {
                        ty::TyClosure(def_id, substs) => {
                            let fn_ty = self.tcx.closure_type(def_id);
                            let fn_ptr = self.memory.create_fn_ptr_from_noncapture_closure(def_id, substs, fn_ty);
                            self.write_value(Value::ByVal(PrimVal::Ptr(fn_ptr)), dest, dest_ty)?;
                        },
                        ref other => bug!("reify fn pointer on {:?}", other),
                    },
                }
            }

            Discriminant(ref lvalue) => {
                let lval = self.eval_lvalue(lvalue)?;
                let ty = self.lvalue_ty(lvalue);
                let ptr = self.force_allocation(lval)?.to_ptr();
                let discr_val = self.read_discriminant_value(ptr, ty)?;
                if let ty::TyAdt(adt_def, _) = ty.sty {
                    if adt_def.discriminants(self.tcx).all(|v| discr_val != v.to_u128_unchecked()) {
                        return Err(EvalError::InvalidDiscriminant);
                    }
                } else {
                    bug!("rustc only generates Rvalue::Discriminant for enums");
                }
                self.write_primval(dest, PrimVal::Bytes(discr_val), dest_ty)?;
            },
        }

        if log_enabled!(::log::LogLevel::Trace) {
            self.dump_local(dest);
        }

        Ok(())
    }

    fn type_is_fat_ptr(&self, ty: Ty<'tcx>) -> bool {
        match ty.sty {
            ty::TyRawPtr(ref tam) |
            ty::TyRef(_, ref tam) => !self.type_is_sized(tam.ty),
            ty::TyAdt(def, _) if def.is_box() => !self.type_is_sized(ty.boxed_ty()),
            _ => false,
        }
    }

    pub(super) fn nonnull_offset_and_ty(
        &self,
        ty: Ty<'tcx>,
        nndiscr: u64,
        discrfield: &[u32],
    ) -> EvalResult<'tcx, (Size, Ty<'tcx>)> {
        // Skip the constant 0 at the start meant for LLVM GEP and the outer non-null variant
        let path = discrfield.iter().skip(2).map(|&i| i as usize);

        // Handle the field index for the outer non-null variant.
        let (inner_offset, inner_ty) = match ty.sty {
            ty::TyAdt(adt_def, substs) => {
                let variant = &adt_def.variants[nndiscr as usize];
                let index = discrfield[1];
                let field = &variant.fields[index as usize];
                (self.get_field_offset(ty, index as usize)?, self.field_ty(field, substs))
            }
            _ => bug!("non-enum for StructWrappedNullablePointer: {}", ty),
        };

        self.field_path_offset_and_ty(inner_offset, inner_ty, path)
    }

    fn field_path_offset_and_ty<I: Iterator<Item = usize>>(
        &self,
        mut offset: Size,
        mut ty: Ty<'tcx>,
        path: I,
    ) -> EvalResult<'tcx, (Size, Ty<'tcx>)> {
        // Skip the initial 0 intended for LLVM GEP.
        for field_index in path {
            let field_offset = self.get_field_offset(ty, field_index)?;
            trace!("field_path_offset_and_ty: {}, {}, {:?}, {:?}", field_index, ty, field_offset, offset);
            ty = self.get_field_ty(ty, field_index)?;
            offset = offset.checked_add(field_offset, &self.tcx.data_layout).unwrap();
        }

        Ok((offset, ty))
    }
    fn get_fat_field(&self, pointee_ty: Ty<'tcx>, field_index: usize) -> EvalResult<'tcx, Ty<'tcx>> {
        match (field_index, &self.tcx.struct_tail(pointee_ty).sty) {
            (1, &ty::TyStr) |
            (1, &ty::TySlice(_)) => Ok(self.tcx.types.usize),
            (1, &ty::TyDynamic(..)) |
            (0, _) => Ok(self.tcx.mk_imm_ptr(self.tcx.types.u8)),
            _ => bug!("invalid fat pointee type: {}", pointee_ty),
        }
    }

    pub fn get_field_ty(&self, ty: Ty<'tcx>, field_index: usize) -> EvalResult<'tcx, Ty<'tcx>> {
        match ty.sty {
            ty::TyAdt(adt_def, _) if adt_def.is_box() => self.get_fat_field(ty.boxed_ty(), field_index),
            ty::TyAdt(adt_def, substs) => {
                use rustc::ty::layout::Layout::*;
                let variant = match *self.type_layout(ty)? {
                    General { .. } => bug!("get_field_ty on enum"),
                    StructWrappedNullablePointer { nndiscr, .. } |
                    RawNullablePointer { nndiscr, .. } => {
                        let variants = adt_def.variants.iter();
                        let discrs = adt_def.discriminants(self.tcx).map(ConstInt::to_u128_unchecked);
                        let (variant, _) = variants.zip(discrs).find(|&(_, discr)| discr == nndiscr as u128).unwrap();
                        variant
                    },
                    // single variant enums are the same as structs. use `.variants[0]` instead of `struct_variant`
                    Univariant { .. } |
                    UntaggedUnion { .. } |
                    Vector { .. } => &adt_def.variants[0],
                    Array { .. } => bug!("{:?} cannot have Array layout", ty),
                    _ => bug!("get_field_ty on non-product type: {:?}", ty),
                };
                Ok(self.field_ty(&variant.fields[field_index], substs))
            }

            ty::TyTuple(fields, _) => Ok(fields[field_index]),

            ty::TyRef(_, ref tam) |
            ty::TyRawPtr(ref tam) => self.get_fat_field(tam.ty, field_index),
            ty::TyArray(inner, _) => Ok(inner),
            ty::TyClosure(did, substs) => Ok(substs.upvar_tys(did, self.tcx).nth(field_index).unwrap()),
            _ => bug!("can't handle type: {:?}, {:?}", ty, ty.sty),
            //_ => Err(EvalError::Unimplemented(format!("can't handle type: {:?}, {:?}", ty, ty.sty))),
        }
    }

    fn get_field_offset(&self, ty: Ty<'tcx>, field_index: usize) -> EvalResult<'tcx, Size> {
        let layout = self.type_layout(ty)?;

        use rustc::ty::layout::Layout::*;
        match *layout {
            Univariant { ref variant, .. } => {
                Ok(variant.offsets[field_index])
            }
            FatPointer { .. } => {
                let bytes = field_index as u64 * self.memory.pointer_size();
                Ok(Size::from_bytes(bytes))
            }
            StructWrappedNullablePointer { ref nonnull, .. } => {
                Ok(nonnull.offsets[field_index])
            }
            Array{ .. } => {
                let field = field_index as u64;
                let elem_size = match ty.sty {
                    ty::TyArray(elem_ty, n) => {
                        assert!(field < n as u64);
                        self.type_size(elem_ty)?.expect("array elements are sized") as u64
                    },
                    _ => bug!("lvalue_field: got Array layout but non-array type {:?}", ty),
                };
                Ok(Size::from_bytes(field * elem_size))
            }
            Vector { element, count } => {
                let field = field_index as u64;
                assert!(field < count);
                let elem_size = element.size(&self.tcx.data_layout).bytes();
                Ok(Size::from_bytes(field * elem_size))
            }
            RawNullablePointer { .. } |
            UntaggedUnion { .. } => Ok(Size::from_bytes(0)),
            _ => {
                let msg = format!("can't handle type: {:?}, with layout: {:?}", ty, layout);
                Err(EvalError::Unimplemented(msg))
            }
        }
    }

    pub fn get_field_count(&self, ty: Ty<'tcx>) -> EvalResult<'tcx, usize> {
        let layout = self.type_layout(ty)?;

        use rustc::ty::layout::Layout::*;
        match *layout {
            Univariant { ref variant, .. } => Ok(variant.offsets.len()),
            FatPointer { .. } => Ok(2),
            StructWrappedNullablePointer { ref nonnull, .. } => Ok(nonnull.offsets.len()),
            Array { .. } => match ty.sty {
                ty::TyArray(_, n) => Ok(n),
                _ => bug!("array layout expected array type, but got {:?}", ty.sty),
            },
            Vector { count, .. } => Ok(count as usize),
            RawNullablePointer { .. } |
            UntaggedUnion { .. } => Ok(1),
            _ => {
                let msg = format!("can't handle type: {:?}, with layout: {:?}", ty, layout);
                Err(EvalError::Unimplemented(msg))
            }
        }
    }

    pub(super) fn eval_operand_to_primval(&mut self, op: &mir::Operand<'tcx>) -> EvalResult<'tcx, PrimVal> {
        let value = self.eval_operand(op)?;
        let ty = self.operand_ty(op);
        self.value_to_primval(value, ty)
    }

    pub(super) fn eval_operand(&mut self, op: &mir::Operand<'tcx>) -> EvalResult<'tcx, Value> {
        use rustc::mir::Operand::*;
        match *op {
            Consume(ref lvalue) => self.eval_and_read_lvalue(lvalue),

            Constant(mir::Constant { ref literal, ty, .. }) => {
                use rustc::mir::Literal;
                let value = match *literal {
                    Literal::Value { ref value } => self.const_to_value(value)?,

                    Literal::Item { def_id, substs } => {
                        if let ty::TyFnDef(..) = ty.sty {
                            // function items are zero sized
                            Value::ByRef(self.memory.allocate(0, 0)?)
                        } else {
                            let (def_id, substs) = self.resolve_associated_const(def_id, substs);
                            let cid = GlobalId { def_id, substs, promoted: None };
                            self.globals.get(&cid).expect("static/const not cached").value
                        }
                    }

                    Literal::Promoted { index } => {
                        let cid = GlobalId {
                            def_id: self.frame().def_id,
                            substs: self.substs(),
                            promoted: Some(index),
                        };
                        self.globals.get(&cid).expect("promoted not cached").value
                    }
                };

                Ok(value)
            }
        }
    }

    pub(super) fn operand_ty(&self, operand: &mir::Operand<'tcx>) -> Ty<'tcx> {
        self.monomorphize(operand.ty(&self.mir(), self.tcx), self.substs())
    }

    fn copy(&mut self, src: Pointer, dest: Pointer, ty: Ty<'tcx>) -> EvalResult<'tcx> {
        let size = self.type_size(ty)?.expect("cannot copy from an unsized type");
        let align = self.type_align(ty)?;
        self.memory.copy(src, dest, size, align)?;
        Ok(())
    }

    pub(super) fn force_allocation(
        &mut self,
        lvalue: Lvalue<'tcx>,
    ) -> EvalResult<'tcx, Lvalue<'tcx>> {
        let new_lvalue = match lvalue {
            Lvalue::Local { frame, local, field } => {
                // -1 since we don't store the return value
                match self.stack[frame].locals[local.index() - 1] {
                    Value::ByRef(ptr) => {
                        assert!(field.is_none());
                        Lvalue::from_ptr(ptr)
                    },
                    val => {
                        let ty = self.stack[frame].mir.local_decls[local].ty;
                        let ty = self.monomorphize(ty, self.stack[frame].substs);
                        let substs = self.stack[frame].substs;
                        let ptr = self.alloc_ptr_with_substs(ty, substs)?;
                        self.stack[frame].locals[local.index() - 1] = Value::ByRef(ptr);
                        self.write_value_to_ptr(val, ptr, ty)?;
                        let lval = Lvalue::from_ptr(ptr);
                        if let Some((field, field_ty)) = field {
                            self.lvalue_field(lval, field, ty, field_ty)?
                        } else {
                            lval
                        }
                    }
                }
            }
            Lvalue::Ptr { .. } => lvalue,
            Lvalue::Global(cid) => {
                let global_val = *self.globals.get(&cid).expect("global not cached");
                match global_val.value {
                    Value::ByRef(ptr) => Lvalue::from_ptr(ptr),
                    _ => {
                        let ptr = self.alloc_ptr_with_substs(global_val.ty, cid.substs)?;
                        self.memory.mark_static(ptr.alloc_id);
                        self.write_value_to_ptr(global_val.value, ptr, global_val.ty)?;
                        // see comment on `initialized` field
                        if global_val.initialized {
                            self.memory.mark_static_initalized(ptr.alloc_id, global_val.mutable)?;
                        }
                        let lval = self.globals.get_mut(&cid).expect("already checked");
                        *lval = Global {
                            value: Value::ByRef(ptr),
                            .. global_val
                        };
                        Lvalue::from_ptr(ptr)
                    },
                }
            }
        };
        Ok(new_lvalue)
    }

    /// ensures this Value is not a ByRef
    pub(super) fn follow_by_ref_value(&mut self, value: Value, ty: Ty<'tcx>) -> EvalResult<'tcx, Value> {
        match value {
            Value::ByRef(ptr) => self.read_value(ptr, ty),
            other => Ok(other),
        }
    }

    pub(super) fn value_to_primval(&mut self, value: Value, ty: Ty<'tcx>) -> EvalResult<'tcx, PrimVal> {
        match self.follow_by_ref_value(value, ty)? {
            Value::ByRef(_) => bug!("follow_by_ref_value can't result in `ByRef`"),

            Value::ByVal(primval) => {
                self.ensure_valid_value(primval, ty)?;
                Ok(primval)
            }

            Value::ByValPair(..) => bug!("value_to_primval can't work with fat pointers"),
        }
    }

    pub(super) fn write_primval(
        &mut self,
        dest: Lvalue<'tcx>,
        val: PrimVal,
        dest_ty: Ty<'tcx>,
    ) -> EvalResult<'tcx> {
        self.write_value(Value::ByVal(val), dest, dest_ty)
    }

    pub(super) fn write_value(
        &mut self,
        src_val: Value,
        dest: Lvalue<'tcx>,
        dest_ty: Ty<'tcx>,
    ) -> EvalResult<'tcx> {
        match dest {
            Lvalue::Global(cid) => {
                let dest = *self.globals.get_mut(&cid).expect("global should be cached");
                if !dest.mutable {
                    return Err(EvalError::ModifiedConstantMemory);
                }
                let write_dest = |this: &mut Self, val| {
                    *this.globals.get_mut(&cid).expect("already checked") = Global {
                        value: val,
                        ..dest
                    };
                    Ok(())
                };
                self.write_value_possibly_by_val(src_val, write_dest, dest.value, dest_ty)
            },

            Lvalue::Ptr { ptr, extra } => {
                assert_eq!(extra, LvalueExtra::None);
                self.write_value_to_ptr(src_val, ptr, dest_ty)
            }

            Lvalue::Local { frame, local, field } => {
                let dest = self.get_local(frame, local, field.map(|(i, _)| i));
                self.write_value_possibly_by_val(
                    src_val,
                    |this, val| this.set_local(frame, local, field, val),
                    dest,
                    dest_ty,
                )
            }
        }
    }

    // The cases here can be a bit subtle. Read carefully!
    fn write_value_possibly_by_val<F: FnOnce(&mut Self, Value) -> EvalResult<'tcx>>(
        &mut self,
        src_val: Value,
        write_dest: F,
        old_dest_val: Value,
        dest_ty: Ty<'tcx>,
    ) -> EvalResult<'tcx> {
        if let Value::ByRef(dest_ptr) = old_dest_val {
            // If the value is already `ByRef` (that is, backed by an `Allocation`),
            // then we must write the new value into this allocation, because there may be
            // other pointers into the allocation. These other pointers are logically
            // pointers into the local variable, and must be able to observe the change.
            //
            // Thus, it would be an error to replace the `ByRef` with a `ByVal`, unless we
            // knew for certain that there were no outstanding pointers to this allocation.
            self.write_value_to_ptr(src_val, dest_ptr, dest_ty)?;

        } else if let Value::ByRef(src_ptr) = src_val {
            // If the value is not `ByRef`, then we know there are no pointers to it
            // and we can simply overwrite the `Value` in the locals array directly.
            //
            // In this specific case, where the source value is `ByRef`, we must duplicate
            // the allocation, because this is a by-value operation. It would be incorrect
            // if they referred to the same allocation, since then a change to one would
            // implicitly change the other.
            //
            // It is a valid optimization to attempt reading a primitive value out of the
            // source and write that into the destination without making an allocation, so
            // we do so here.
            if let Ok(Some(src_val)) = self.try_read_value(src_ptr, dest_ty) {
                write_dest(self, src_val)?;
            } else {
                let dest_ptr = self.alloc_ptr(dest_ty)?;
                self.copy(src_ptr, dest_ptr, dest_ty)?;
                write_dest(self, Value::ByRef(dest_ptr))?;
            }

        } else {
            // Finally, we have the simple case where neither source nor destination are
            // `ByRef`. We may simply copy the source value over the the destintion.
            write_dest(self, src_val)?;
        }
        Ok(())
    }

    pub(super) fn write_value_to_ptr(
        &mut self,
        value: Value,
        dest: Pointer,
        dest_ty: Ty<'tcx>,
    ) -> EvalResult<'tcx> {
        match value {
            Value::ByRef(ptr) => self.copy(ptr, dest, dest_ty),
            Value::ByVal(primval) => {
                let size = self.type_size(dest_ty)?.expect("dest type must be sized");
                self.memory.write_primval(dest, primval, size)
            }
            Value::ByValPair(a, b) => self.write_pair_to_ptr(a, b, dest, dest_ty),
        }
    }

    pub(super) fn write_pair_to_ptr(
        &mut self,
        a: PrimVal,
        b: PrimVal,
        ptr: Pointer,
        mut ty: Ty<'tcx>
    ) -> EvalResult<'tcx> {
        while self.get_field_count(ty)? == 1 {
            ty = self.get_field_ty(ty, 0)?;
        }
        assert_eq!(self.get_field_count(ty)?, 2);
        let field_0 = self.get_field_offset(ty, 0)?.bytes();
        let field_1 = self.get_field_offset(ty, 1)?.bytes();
        let field_0_ty = self.get_field_ty(ty, 0)?;
        let field_1_ty = self.get_field_ty(ty, 1)?;
        let field_0_size = self.type_size(field_0_ty)?.expect("pair element type must be sized");
        let field_1_size = self.type_size(field_1_ty)?.expect("pair element type must be sized");
        self.memory.write_primval(ptr.offset(field_0), a, field_0_size)?;
        self.memory.write_primval(ptr.offset(field_1), b, field_1_size)?;
        Ok(())
    }

    fn ptr_ty_to_value_kind(&self, ty: Ty<'tcx>) -> ValueKind {
        if self.type_is_sized(ty) {
            ValueKind::Val(PrimValKind::Ptr)
        } else {
            let extra = match self.tcx.struct_tail(ty).sty {
                ty::TyStr | ty::TySlice(_) => PrimValKind::from_uint_size(self.memory.pointer_size()),
                ty::TyDynamic(..) => PrimValKind::Ptr,
                _ => bug!("{:?} is not an unsized type", ty),
            };
            ValueKind::ValPair(PrimValKind::Ptr, extra)
        }
    }

    // keep this function in sync with `try_read_value`
    pub fn ty_to_value_kind(&self, ty: Ty<'tcx>) -> ValueKind {
        match ty.sty {
            ty::TyRef(_, ref tam) |
            ty::TyRawPtr(ref tam) => self.ptr_ty_to_value_kind(tam.ty),
            ty::TyAdt(ref def, _) if def.is_box() => self.ptr_ty_to_value_kind(ty.boxed_ty()),
            ty::TyAdt(..) => {
                match self.get_field_count(ty) {
                    Ok(1) => {
                        let field_ty = self.get_field_ty(ty, 0).expect("has one field");
                        self.ty_to_value_kind(field_ty)
                    },
                    Ok(2) => {
                        let a = self.get_field_ty(ty, 0).expect("has at least one field");
                        let b = self.get_field_ty(ty, 1).expect("has two fields");
                        match (self.ty_to_primval_kind(a), self.ty_to_primval_kind(b)) {
                            (Ok(a), Ok(b)) => ValueKind::ValPair(a, b),
                            _ => ValueKind::Ref,
                        }
                    },
                    _ => ValueKind::Ref,
                }
            },
            // everything else is either a single value primval or must be ByRef
            _ => self.ty_to_primval_kind(ty).map(ValueKind::Val).unwrap_or(ValueKind::Ref),
        }
    }
    pub fn ty_to_primval_kind(&self, ty: Ty<'tcx>) -> EvalResult<'tcx, PrimValKind> {
        use syntax::ast::FloatTy;

        let kind = match ty.sty {
            ty::TyBool => PrimValKind::Bool,
            ty::TyChar => PrimValKind::Char,

            ty::TyInt(int_ty) => {
                use syntax::ast::IntTy::*;
                let size = match int_ty {
                    I8 => 1,
                    I16 => 2,
                    I32 => 4,
                    I64 => 8,
                    I128 => 16,
                    Is => self.memory.pointer_size(),
                };
                PrimValKind::from_int_size(size)
            }

            ty::TyUint(uint_ty) => {
                use syntax::ast::UintTy::*;
                let size = match uint_ty {
                    U8 => 1,
                    U16 => 2,
                    U32 => 4,
                    U64 => 8,
                    U128 => 16,
                    Us => self.memory.pointer_size(),
                };
                PrimValKind::from_uint_size(size)
            }

            ty::TyFloat(FloatTy::F32) => PrimValKind::F32,
            ty::TyFloat(FloatTy::F64) => PrimValKind::F64,

            ty::TyFnPtr(_) => PrimValKind::FnPtr,

            ty::TyRef(_, ref tam) |
            ty::TyRawPtr(ref tam) if self.type_is_sized(tam.ty) => PrimValKind::Ptr,

            ty::TyAdt(ref def, _) if def.is_box() => {
                if self.type_is_sized(ty.boxed_ty()) {
                    PrimValKind::Ptr
                } else {
                    return Err(EvalError::TypeNotPrimitive(ty));
                }
            },

            ty::TyAdt(ref def, substs) => {
                use rustc::ty::layout::Layout::*;
                match *self.type_layout(ty)? {
                    CEnum { discr, signed, .. } => {
                        let size = discr.size().bytes();
                        if signed {
                            PrimValKind::from_int_size(size)
                        } else {
                            PrimValKind::from_uint_size(size)
                        }
                    }

                    RawNullablePointer { value, .. } => {
                        use rustc::ty::layout::Primitive::*;
                        match value {
                            // TODO(solson): Does signedness matter here? What should the sign be?
                            Int(int) => PrimValKind::from_uint_size(int.size().bytes()),
                            F32 => PrimValKind::F32,
                            F64 => PrimValKind::F64,
                            Pointer => PrimValKind::Ptr,
                        }
                    }

                    // represent single field structs as their single field
                    Univariant { .. } => {
                        // enums with just one variant are no different, but `.struct_variant()` doesn't work for enums
                        let variant = &def.variants[0];
                        // FIXME: also allow structs with only a single non zst field
                        if variant.fields.len() == 1 {
                            return self.ty_to_primval_kind(self.field_ty(&variant.fields[0], substs));
                        } else {
                            return Err(EvalError::TypeNotPrimitive(ty));
                        }
                    }

                    _ => return Err(EvalError::TypeNotPrimitive(ty)),
                }
            }

            _ => return Err(EvalError::TypeNotPrimitive(ty)),
        };

        Ok(kind)
    }

    fn ensure_valid_value(&self, val: PrimVal, ty: Ty<'tcx>) -> EvalResult<'tcx> {
        match ty.sty {
            ty::TyBool if val.to_bytes()? > 1 => Err(EvalError::InvalidBool),

            ty::TyChar if ::std::char::from_u32(val.to_bytes()? as u32).is_none()
                => Err(EvalError::InvalidChar(val.to_bytes()? as u32 as u128)),

            _ => Ok(()),
        }
    }

    pub(super) fn read_value(&mut self, ptr: Pointer, ty: Ty<'tcx>) -> EvalResult<'tcx, Value> {
        if let Some(val) = self.try_read_value(ptr, ty)? {
            Ok(val)
        } else {
            bug!("primitive read failed for type: {:?}", ty);
        }
    }

    fn read_ptr(&mut self, ptr: Pointer, pointee_ty: Ty<'tcx>) -> EvalResult<'tcx, Value> {
        let p = self.memory.read_ptr(ptr)?;
        if self.type_is_sized(pointee_ty) {
            Ok(Value::ByVal(PrimVal::Ptr(p)))
        } else {
            trace!("reading fat pointer extra of type {}", pointee_ty);
            let extra = ptr.offset(self.memory.pointer_size());
            let extra = match self.tcx.struct_tail(pointee_ty).sty {
                ty::TyDynamic(..) => PrimVal::Ptr(self.memory.read_ptr(extra)?),
                ty::TySlice(..) |
                ty::TyStr => PrimVal::from_u128(self.memory.read_usize(extra)? as u128),
                _ => bug!("unsized primval ptr read from {:?}", pointee_ty),
            };
            Ok(Value::ByValPair(PrimVal::Ptr(p), extra))
        }
    }

    // keep this function in sync with `ty_to_primval_kind`
    fn try_read_value(&mut self, ptr: Pointer, ty: Ty<'tcx>) -> EvalResult<'tcx, Option<Value>> {
        use syntax::ast::FloatTy;

        let val = match ty.sty {
            ty::TyBool => PrimVal::from_bool(self.memory.read_bool(ptr)?),
            ty::TyChar => {
                let c = self.memory.read_uint(ptr, 4)? as u32;
                match ::std::char::from_u32(c) {
                    Some(ch) => PrimVal::from_char(ch),
                    None => return Err(EvalError::InvalidChar(c as u128)),
                }
            }

            ty::TyInt(int_ty) => {
                use syntax::ast::IntTy::*;
                let size = match int_ty {
                    I8 => 1,
                    I16 => 2,
                    I32 => 4,
                    I64 => 8,
                    I128 => 16,
                    Is => self.memory.pointer_size(),
                };
                PrimVal::from_i128(self.memory.read_int(ptr, size)?)
            }

            ty::TyUint(uint_ty) => {
                use syntax::ast::UintTy::*;
                let size = match uint_ty {
                    U8 => 1,
                    U16 => 2,
                    U32 => 4,
                    U64 => 8,
                    U128 => 16,
                    Us => self.memory.pointer_size(),
                };
                PrimVal::from_u128(self.memory.read_uint(ptr, size)?)
            }

            ty::TyFloat(FloatTy::F32) => PrimVal::from_f32(self.memory.read_f32(ptr)?),
            ty::TyFloat(FloatTy::F64) => PrimVal::from_f64(self.memory.read_f64(ptr)?),

            ty::TyFnPtr(_) => self.memory.read_ptr(ptr).map(PrimVal::Ptr)?,
            ty::TyRef(_, ref tam) |
            ty::TyRawPtr(ref tam) => return self.read_ptr(ptr, tam.ty).map(Some),

            ty::TyAdt(def, substs) => {
                if def.is_box() {
                    return self.read_ptr(ptr, ty.boxed_ty()).map(Some);
                }
                use rustc::ty::layout::Layout::*;
                match *self.type_layout(ty)? {
                    CEnum { discr, signed, .. } => {
                        let size = discr.size().bytes();
                        if signed {
                            PrimVal::from_i128(self.memory.read_int(ptr, size)?)
                        } else {
                            PrimVal::from_u128(self.memory.read_uint(ptr, size)?)
                        }
                    },
                    RawNullablePointer { value, ..} => {
                        use rustc::ty::layout::Primitive::*;
                        match value {
                            // TODO(solson): Does signedness matter here? What should the sign be?
                            Int(int) => PrimVal::from_u128(self.memory.read_uint(ptr, int.size().bytes())?),
                            F32 => PrimVal::from_f32(self.memory.read_f32(ptr)?),
                            F64 => PrimVal::from_f64(self.memory.read_f64(ptr)?),
                            Pointer => self.memory.read_ptr(ptr).map(PrimVal::Ptr)?,
                        }
                    },
                    Univariant { .. } => {
                        // enums with just one variant are no different, but `.struct_variant()` doesn't work for enums
                        let variant = &def.variants[0];
                        // FIXME: also allow structs with only a single non zst field
                        if variant.fields.len() == 1 {
                            let ty = self.field_ty(&variant.fields[0], substs);
                            return self.try_read_value(ptr, ty);
                        } else {
                            debug_assert!(self.ty_to_primval_kind(ty).is_err());
                            return Ok(None);
                        }
                    }
                    _ => {
                        debug_assert!(self.ty_to_primval_kind(ty).is_err());
                        return Ok(None);
                    },
                }
            },

            _ => {
                debug_assert!(self.ty_to_primval_kind(ty).is_err());
                return Ok(None);
            },
        };

        Ok(Some(Value::ByVal(val)))
    }

    pub(super) fn frame(&self) -> &Frame<'tcx> {
        self.stack.last().expect("no call frames exist")
    }

    pub(super) fn frame_mut(&mut self) -> &mut Frame<'tcx> {
        self.stack.last_mut().expect("no call frames exist")
    }

    pub(super) fn mir(&self) -> MirRef<'tcx> {
        Ref::clone(&self.frame().mir)
    }

    pub(super) fn substs(&self) -> &'tcx Substs<'tcx> {
        self.frame().substs
    }

    fn unsize_into_ptr(
        &mut self,
        src: Value,
        src_ty: Ty<'tcx>,
        dest: Lvalue<'tcx>,
        dest_ty: Ty<'tcx>,
        sty: Ty<'tcx>,
        dty: Ty<'tcx>,
    ) -> EvalResult<'tcx> {
        // A<Struct> -> A<Trait> conversion
        let (src_pointee_ty, dest_pointee_ty) = self.tcx.struct_lockstep_tails(sty, dty);

        match (&src_pointee_ty.sty, &dest_pointee_ty.sty) {
            (&ty::TyArray(_, length), &ty::TySlice(_)) => {
                let ptr = src.read_ptr(&self.memory)?;
                let len = PrimVal::from_u128(length as u128);
                let ptr = PrimVal::Ptr(ptr);
                self.write_value(Value::ByValPair(ptr, len), dest, dest_ty)
            }
            (&ty::TyDynamic(..), &ty::TyDynamic(..)) => {
                // For now, upcasts are limited to changes in marker
                // traits, and hence never actually require an actual
                // change to the vtable.
                self.write_value(src, dest, dest_ty)
            },
            (_, &ty::TyDynamic(ref data, _)) => {
                let trait_ref = data.principal().unwrap().with_self_ty(self.tcx, src_pointee_ty);
                let trait_ref = self.tcx.erase_regions(&trait_ref);
                let vtable = self.get_vtable(trait_ref)?;
                let ptr = src.read_ptr(&self.memory)?;
                let ptr = PrimVal::Ptr(ptr);
                let extra = PrimVal::Ptr(vtable);
                self.write_value(Value::ByValPair(ptr, extra), dest, dest_ty)
            },

            _ => bug!("invalid unsizing {:?} -> {:?}", src_ty, dest_ty),
        }
    }

    fn unsize_into(
        &mut self,
        src: Value,
        src_ty: Ty<'tcx>,
        dest: Lvalue<'tcx>,
        dest_ty: Ty<'tcx>,
    ) -> EvalResult<'tcx> {
        match (&src_ty.sty, &dest_ty.sty) {
            (&ty::TyRef(_, ref s), &ty::TyRef(_, ref d)) |
            (&ty::TyRef(_, ref s), &ty::TyRawPtr(ref d)) |
            (&ty::TyRawPtr(ref s), &ty::TyRawPtr(ref d)) => self.unsize_into_ptr(src, src_ty, dest, dest_ty, s.ty, d.ty),
            (&ty::TyAdt(def_a, substs_a), &ty::TyAdt(def_b, substs_b)) => {
                if def_a.is_box() || def_b.is_box() {
                    if !def_a.is_box() || !def_b.is_box() {
                        panic!("invalid unsizing between {:?} -> {:?}", src_ty, dest_ty);
                    }
                    return self.unsize_into_ptr(src, src_ty, dest, dest_ty, src_ty.boxed_ty(), dest_ty.boxed_ty());
                }
                if self.ty_to_primval_kind(src_ty).is_ok() {
                    let sty = self.get_field_ty(src_ty, 0)?;
                    let dty = self.get_field_ty(dest_ty, 0)?;
                    return self.unsize_into(src, sty, dest, dty);
                }
                // unsizing of generic struct with pointer fields
                // Example: `Arc<T>` -> `Arc<Trait>`
                // here we need to increase the size of every &T thin ptr field to a fat ptr

                assert_eq!(def_a, def_b);

                let src_fields = def_a.variants[0].fields.iter();
                let dst_fields = def_b.variants[0].fields.iter();

                //let src = adt::MaybeSizedValue::sized(src);
                //let dst = adt::MaybeSizedValue::sized(dst);
                let src_ptr = match src {
                    Value::ByRef(ptr) => ptr,
                    _ => bug!("expected pointer, got {:?}", src),
                };

                // FIXME(solson)
                let dest = self.force_allocation(dest)?.to_ptr();
                let iter = src_fields.zip(dst_fields).enumerate();
                for (i, (src_f, dst_f)) in iter {
                    let src_fty = self.field_ty(src_f, substs_a);
                    let dst_fty = self.field_ty(dst_f, substs_b);
                    if self.type_size(dst_fty)? == Some(0) {
                        continue;
                    }
                    let src_field_offset = self.get_field_offset(src_ty, i)?.bytes();
                    let dst_field_offset = self.get_field_offset(dest_ty, i)?.bytes();
                    let src_f_ptr = src_ptr.offset(src_field_offset);
                    let dst_f_ptr = dest.offset(dst_field_offset);
                    if src_fty == dst_fty {
                        self.copy(src_f_ptr, dst_f_ptr, src_fty)?;
                    } else {
                        self.unsize_into(Value::ByRef(src_f_ptr), src_fty, Lvalue::from_ptr(dst_f_ptr), dst_fty)?;
                    }
                }
                Ok(())
            }
            _ => bug!("unsize_into: invalid conversion: {:?} -> {:?}", src_ty, dest_ty),
        }
    }

    pub(super) fn dump_local(&self, lvalue: Lvalue<'tcx>) {
        if let Lvalue::Local { frame, local, field } = lvalue {
            let mut allocs = Vec::new();
            let mut msg = format!("{:?}", local);
            if let Some((field, _)) = field {
                write!(msg, ".{}", field).unwrap();
            }
            let last_frame = self.stack.len() - 1;
            if frame != last_frame {
                write!(msg, " ({} frames up)", last_frame - frame).unwrap();
            }
            write!(msg, ":").unwrap();

            match self.get_local(frame, local, field.map(|(i, _)| i)) {
                Value::ByRef(ptr) => {
                    allocs.push(ptr.alloc_id);
                }
                Value::ByVal(val) => {
                    write!(msg, " {:?}", val).unwrap();
                    if let PrimVal::Ptr(ptr) = val { allocs.push(ptr.alloc_id); }
                }
                Value::ByValPair(val1, val2) => {
                    write!(msg, " ({:?}, {:?})", val1, val2).unwrap();
                    if let PrimVal::Ptr(ptr) = val1 { allocs.push(ptr.alloc_id); }
                    if let PrimVal::Ptr(ptr) = val2 { allocs.push(ptr.alloc_id); }
                }
            }

            trace!("{}", msg);
            self.memory.dump_allocs(allocs);
        }
    }

    /// Convenience function to ensure correct usage of globals and code-sharing with locals.
    pub fn modify_global<F>(&mut self, cid: GlobalId<'tcx>, f: F) -> EvalResult<'tcx>
        where F: FnOnce(&mut Self, Value) -> EvalResult<'tcx, Value>,
    {
        let mut val = *self.globals.get(&cid).expect("global not cached");
        if !val.mutable {
            return Err(EvalError::ModifiedConstantMemory);
        }
        val.value = f(self, val.value)?;
        *self.globals.get_mut(&cid).expect("already checked") = val;
        Ok(())
    }

    /// Convenience function to ensure correct usage of locals and code-sharing with globals.
    pub fn modify_local<F>(
        &mut self,
        frame: usize,
        local: mir::Local,
        field: Option<(usize, Ty<'tcx>)>,
        f: F,
    ) -> EvalResult<'tcx>
        where F: FnOnce(&mut Self, Value) -> EvalResult<'tcx, Value>,
    {
        let val = self.get_local(frame, local, field.map(|(i, _)| i));
        let new_val = f(self, val)?;
        self.set_local(frame, local, field, new_val)?;
        // FIXME(solson): Run this when setting to Undef? (See previous version of this code.)
        // if let Value::ByRef(ptr) = self.stack[frame].get_local(local) {
        //     self.memory.deallocate(ptr)?;
        // }
        Ok(())
    }

    pub fn get_local(&self, frame: usize, local: mir::Local, field: Option<usize>) -> Value {
        // Subtract 1 because we don't store a value for the ReturnPointer, the local with index 0.
        if let Some(field) = field {
            match self.stack[frame].locals[local.index() - 1] {
                Value::ByRef(_) => bug!("can't have lvalue fields for ByRef"),
                val @ Value::ByVal(_) => {
                    assert_eq!(field, 0);
                    val
                },
                Value::ByValPair(a, b) => {
                    match field {
                        0 => Value::ByVal(a),
                        1 => Value::ByVal(b),
                        _ => bug!("ByValPair has only two fields, tried to access {}", field),
                    }
                },
            }
        } else {
            self.stack[frame].locals[local.index() - 1]
        }
    }

    pub(crate) fn set_local(&mut self, frame: usize, local: mir::Local, field: Option<(usize, Ty<'tcx>)>, value: Value) -> EvalResult<'tcx> {
        // Subtract 1 because we don't store a value for the ReturnPointer, the local with index 0.
        if let Some((field, ty)) = field {
            match self.stack[frame].locals[local.index() - 1] {
                Value::ByRef(_) => bug!("can't have lvalue fields for ByRef"),
                Value::ByVal(_) => {
                    assert_eq!(field, 0);
                    self.set_local(frame, local, None, value)?;
                },
                Value::ByValPair(a, b) => {
                    let prim = self.value_to_primval(value, ty)?;
                    match field {
                        0 => self.set_local(frame, local, None, Value::ByValPair(prim, b))?,
                        1 => self.set_local(frame, local, None, Value::ByValPair(a, prim))?,
                        _ => bug!("ByValPair has only two fields, tried to access {}", field),
                    }
                },
            }
        } else {
            self.stack[frame].locals[local.index() - 1] = value;
        }
        Ok(())
    }

    /// Returns the normalized type of a struct field
    pub fn field_ty(
        &self,
        f: &ty::FieldDef,
        param_substs: &Substs<'tcx>,
    ) -> ty::Ty<'tcx> {
        monomorphize_field_ty(self.tcx, f, param_substs)
    }
}

pub fn eval_main<'a, 'tcx: 'a>(
    tcx: TyCtxt<'a, 'tcx, 'tcx>,
    def_id: DefId,
    limits: ResourceLimits,
) {
    let mut ecx = EvalContext::new(tcx, limits);
    let mir = ecx.load_mir(def_id).expect("main function's MIR not found");

    if !mir.return_ty.is_nil() || mir.arg_count != 0 {
        let msg = "miri does not support main functions without `fn()` type signatures";
        tcx.sess.err(&EvalError::Unimplemented(String::from(msg)).to_string());
        return;
    }

    ecx.push_stack_frame(
        def_id,
        DUMMY_SP,
        mir,
        tcx.intern_substs(&[]),
        Lvalue::from_ptr(Pointer::zst_ptr()),
        StackPopCleanup::None,
        Vec::new(),
    ).expect("could not allocate first stack frame");

    loop {
        match ecx.step() {
            Ok(true) => {}
            Ok(false) => {
                let leaks = ecx.memory.leak_report();
                if leaks != 0 {
                    tcx.sess.err("the evaluated program leaked memory");
                }
                return;
            }
            Err(e) => {
                report(tcx, &ecx, e);
                return;
            }
        }
    }
}

fn report(tcx: TyCtxt, ecx: &EvalContext, e: EvalError) {
    let frame = ecx.stack().last().expect("stackframe was empty");
    let block = &frame.mir.basic_blocks()[frame.block];
    let span = if frame.stmt < block.statements.len() {
        block.statements[frame.stmt].source_info.span
    } else {
        block.terminator().source_info.span
    };
    let mut err = tcx.sess.struct_span_err(span, &e.to_string());
    for &Frame { def_id, substs, span, .. } in ecx.stack().iter().rev() {
        if tcx.def_key(def_id).disambiguated_data.data == DefPathData::ClosureExpr {
            err.span_note(span, "inside call to closure");
            continue;
        }
        // FIXME(solson): Find a way to do this without this Display impl hack.
        use rustc::util::ppaux;
        use std::fmt;
        struct Instance<'tcx>(DefId, &'tcx subst::Substs<'tcx>);
        impl<'tcx> ::std::panic::UnwindSafe for Instance<'tcx> {}
        impl<'tcx> ::std::panic::RefUnwindSafe for Instance<'tcx> {}
        impl<'tcx> fmt::Display for Instance<'tcx> {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                ppaux::parameterized(f, self.1, self.0, &[])
            }
        }
        err.span_note(span, &format!("inside call to {}", Instance(def_id, substs)));
    }
    err.emit();
}

pub fn run_mir_passes<'a, 'tcx>(tcx: TyCtxt<'a, 'tcx, 'tcx>) {
    let mut passes = ::rustc::mir::transform::Passes::new();
    passes.push_hook(Box::new(::rustc_mir::transform::dump_mir::DumpMir));
    passes.push_pass(Box::new(::rustc_mir::transform::no_landing_pads::NoLandingPads));
    passes.push_pass(Box::new(::rustc_mir::transform::simplify::SimplifyCfg::new("no-landing-pads")));

    // From here on out, regions are gone.
    passes.push_pass(Box::new(::rustc_mir::transform::erase_regions::EraseRegions));

    passes.push_pass(Box::new(::rustc_mir::transform::add_call_guards::AddCallGuards));
    passes.push_pass(Box::new(::rustc_borrowck::ElaborateDrops));
    passes.push_pass(Box::new(::rustc_mir::transform::no_landing_pads::NoLandingPads));
    passes.push_pass(Box::new(::rustc_mir::transform::simplify::SimplifyCfg::new("elaborate-drops")));

    // No lifetime analysis based on borrowing can be done from here on out.
    passes.push_pass(Box::new(::rustc_mir::transform::instcombine::InstCombine::new()));
    passes.push_pass(Box::new(::rustc_mir::transform::deaggregator::Deaggregator));
    passes.push_pass(Box::new(::rustc_mir::transform::copy_prop::CopyPropagation));

    passes.push_pass(Box::new(::rustc_mir::transform::simplify::SimplifyLocals));
    passes.push_pass(Box::new(::rustc_mir::transform::add_call_guards::AddCallGuards));
    passes.push_pass(Box::new(::rustc_mir::transform::dump_mir::Marker("PreMiri")));

    passes.run_passes(tcx);
}

// TODO(solson): Upstream these methods into rustc::ty::layout.

pub(super) trait IntegerExt {
    fn size(self) -> Size;
}

impl IntegerExt for layout::Integer {
    fn size(self) -> Size {
        use rustc::ty::layout::Integer::*;
        match self {
            I1 | I8 => Size::from_bits(8),
            I16 => Size::from_bits(16),
            I32 => Size::from_bits(32),
            I64 => Size::from_bits(64),
            I128 => Size::from_bits(128),
        }
    }
}


pub fn monomorphize_field_ty<'a, 'tcx: 'a>(tcx: TyCtxt<'a, 'tcx, 'tcx>, f: &ty::FieldDef, substs: &Substs<'tcx>) -> Ty<'tcx> {
    let substituted = f.ty(tcx, substs);
    tcx.normalize_associated_type(&substituted)
}

pub fn is_inhabited<'a, 'tcx: 'a>(tcx: TyCtxt<'a, 'tcx, 'tcx>, ty: Ty<'tcx>) -> bool {
    ty.uninhabited_from(&mut HashMap::default(), tcx).is_empty()
}

pub trait IntoValTyPair<'tcx> {
    fn into_val_ty_pair<'a>(self, ecx: &mut EvalContext<'a, 'tcx>) -> EvalResult<'tcx, (Value, Ty<'tcx>)> where 'tcx: 'a;
}

impl<'tcx> IntoValTyPair<'tcx> for (Value, Ty<'tcx>) {
    fn into_val_ty_pair<'a>(self, _: &mut EvalContext<'a, 'tcx>) -> EvalResult<'tcx, (Value, Ty<'tcx>)> where 'tcx: 'a {
        Ok(self)
    }
}

impl<'b, 'tcx: 'b> IntoValTyPair<'tcx> for &'b mir::Operand<'tcx> {
    fn into_val_ty_pair<'a>(self, ecx: &mut EvalContext<'a, 'tcx>) -> EvalResult<'tcx, (Value, Ty<'tcx>)> where 'tcx: 'a {
        let value = ecx.eval_operand(self)?;
        let value_ty = ecx.operand_ty(self);
        Ok((value, value_ty))
    }
}
