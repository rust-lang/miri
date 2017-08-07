use rustc::mir;
use rustc::ty::layout::{Size, Align};
use rustc::ty::{self, Ty};
use rustc_data_structures::indexed_vec::Idx;
use syntax::ast::Mutability;

use super::{EvalResult, EvalContext, MemoryPointer, PrimVal, Value, Pointer, Machine};

#[derive(Copy, Clone, Debug)]
pub enum Lvalue<'tcx> {
    /// An lvalue referring to a value allocated in the `Memory` system.
    Ptr {
        /// An lvalue may have an invalid (integral or undef) pointer,
        /// since it might be turned back into a reference
        /// before ever being dereferenced.
        ptr: Pointer,
        extra: LvalueExtra,
        /// Remember whether this lvalue is *supposed* to be aligned.
        aligned: bool,
    },

    /// An lvalue referring to a value on the stack. Represented by a stack frame index paired with
    /// a Mir local index.
    Local { frame: usize, local: mir::Local },

    /// An lvalue referring to a global
    Global(GlobalId<'tcx>),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LvalueExtra {
    None,
    Length(u64),
    Vtable(MemoryPointer),
    DowncastVariant(usize),
}

/// Uniquely identifies a specific constant or static.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct GlobalId<'tcx> {
    /// For a constant or static, the `Instance` of the item itself.
    /// For a promoted global, the `Instance` of the function they belong to.
    pub instance: ty::Instance<'tcx>,

    /// The index for promoted globals within their function's `Mir`.
    pub promoted: Option<mir::Promoted>,
}

#[derive(Clone, Debug)]
pub struct Global<'tcx> {
    pub value: Value,
    /// Only used in `force_allocation` to ensure we don't mark the memory
    /// before the static is initialized. It is possible to convert a
    /// global which initially is `Value::ByVal(PrimVal::Undef)` and gets
    /// lifted to an allocation before the static is fully initialized
    pub(super) initialized: bool,
    pub(super) mutable: Mutability,
    pub(super) ty: Ty<'tcx>,
}

impl<'tcx> Lvalue<'tcx> {
    /// Produces an Lvalue that will error if attempted to be read from
    pub fn undef() -> Self {
        Self::from_primval_ptr(PrimVal::Undef.into())
    }

    pub fn from_primval_ptr(ptr: Pointer) -> Self {
        Lvalue::Ptr {
            ptr,
            extra: LvalueExtra::None,
            aligned: true,
        }
    }

    pub fn from_ptr(ptr: MemoryPointer) -> Self {
        Self::from_primval_ptr(ptr.into())
    }

    pub(super) fn to_ptr_extra_aligned(self) -> (Pointer, LvalueExtra, bool) {
        match self {
            Lvalue::Ptr {
                ptr,
                extra,
                aligned,
            } => (ptr, extra, aligned),
            _ => bug!("to_ptr_and_extra: expected Lvalue::Ptr, got {:?}", self),

        }
    }

    pub fn to_ptr(self) -> EvalResult<'tcx, MemoryPointer> {
        let (ptr, extra, _aligned) = self.to_ptr_extra_aligned();
        // At this point, we forget about the alignment information
        // -- the lvalue has been turned into a reference,
        // and no matter where it came from, it now must be aligned.
        assert_eq!(extra, LvalueExtra::None);
        ptr.to_ptr()
    }

    pub(super) fn elem_ty_and_len(self, ty: Ty<'tcx>) -> (Ty<'tcx>, u64) {
        match ty.sty {
            ty::TyArray(elem, n) => (elem, n as u64),

            ty::TySlice(elem) => {
                match self {
                    Lvalue::Ptr { extra: LvalueExtra::Length(len), .. } => (elem, len),
                    _ => {
                        bug!(
                            "elem_ty_and_len of a TySlice given non-slice lvalue: {:?}",
                            self
                        )
                    }
                }
            }

            _ => bug!("elem_ty_and_len expected array or slice, got {:?}", ty),
        }
    }
}

impl<'tcx> Global<'tcx> {
    pub(super) fn uninitialized(ty: Ty<'tcx>) -> Self {
        Global {
            value: Value::ByVal(PrimVal::Undef),
            mutable: Mutability::Mutable,
            ty,
            initialized: false,
        }
    }

    pub(super) fn initialized(ty: Ty<'tcx>, value: Value, mutable: Mutability) -> Self {
        Global {
            value,
            mutable,
            ty,
            initialized: true,
        }
    }
}

impl<'a, 'tcx, M: Machine<'tcx>> EvalContext<'a, 'tcx, M> {
    /// Reads a value from the lvalue without going through the intermediate step of obtaining
    /// a `miri::Lvalue`
    pub fn try_read_lvalue(
        &mut self,
        lvalue: &mir::Lvalue<'tcx>,
    ) -> EvalResult<'tcx, Option<Value>> {
        use rustc::mir::Lvalue::*;
        match *lvalue {
            // Might allow this in the future,
            // right now there's no way to do this from Rust code anyway
            Local(mir::RETURN_POINTER) => err!(ReadFromReturnPointer),
            // Directly reading a local will always succeed
            Local(local) => self.frame().get_local(local).map(Some),
            // Directly reading a static will always succeed
            Static(ref static_) => {
                let instance = ty::Instance::mono(self.tcx, static_.def_id);
                let cid = GlobalId {
                    instance,
                    promoted: None,
                };
                Ok(Some(
                    self.globals.get(&cid).expect("global not cached").value,
                ))
            }
            Projection(ref proj) => self.try_read_lvalue_projection(proj),
        }
    }

    fn try_read_lvalue_projection(
        &mut self,
        proj: &mir::LvalueProjection<'tcx>,
    ) -> EvalResult<'tcx, Option<Value>> {
        use rustc::mir::ProjectionElem::*;
        let base = match self.try_read_lvalue(&proj.base)? {
            Some(base) => base,
            None => return Ok(None),
        };
        let base_ty = self.lvalue_ty(&proj.base);
        match proj.elem {
            Field(field, _) => match (field.index(), base) {
                // the only field of a struct
                (0, Value::ByVal(val)) => Ok(Some(Value::ByVal(val))),
                // split fat pointers, 2 element tuples, ...
                (0...1, Value::ByValPair(a, b)) if self.get_field_count(base_ty)? == 2 => {
                    let val = [a, b][field.index()];
                    Ok(Some(Value::ByVal(val)))
                },
                // the only field of a struct is a fat pointer
                (0, Value::ByValPair(..)) => Ok(Some(base)),
                _ => Ok(None),
            },
            // The NullablePointer cases should work fine, need to take care for normal enums
            Downcast(..) |
            Subslice { .. } |
            // reading index 0 or index 1 from a ByVal or ByVal pair could be optimized
            ConstantIndex { .. } | Index(_) |
            // No way to optimize this projection any better than the normal lvalue path
            Deref => Ok(None),
        }
    }

    /// Returns a value and (in case of a ByRef) if we are supposed to use aligned accesses.
    pub(super) fn eval_and_read_lvalue(
        &mut self,
        lvalue: &mir::Lvalue<'tcx>,
    ) -> EvalResult<'tcx, Value> {
        // Shortcut for things like accessing a fat pointer's field,
        // which would otherwise (in the `eval_lvalue` path) require moving a `ByValPair` to memory
        // and returning an `Lvalue::Ptr` to it
        if let Some(val) = self.try_read_lvalue(lvalue)? {
            return Ok(val);
        }
        let lvalue = self.eval_lvalue(lvalue)?;
        self.read_lvalue(lvalue)
    }

    pub fn read_lvalue(&self, lvalue: Lvalue<'tcx>) -> EvalResult<'tcx, Value> {
        match lvalue {
            Lvalue::Ptr {
                ptr,
                extra,
                aligned,
            } => {
                assert_eq!(extra, LvalueExtra::None);
                Ok(Value::ByRef { ptr, aligned })
            }
            Lvalue::Local { frame, local } => self.stack[frame].get_local(local),
            Lvalue::Global(cid) => Ok(self.globals.get(&cid).expect("global not cached").value),
        }
    }

    pub fn eval_lvalue(
        &mut self,
        mir_lvalue: &mir::Lvalue<'tcx>,
    ) -> EvalResult<'tcx, Lvalue<'tcx>> {
        use rustc::mir::Lvalue::*;
        let lvalue = match *mir_lvalue {
            Local(mir::RETURN_POINTER) => self.frame().return_lvalue,
            Local(local) => Lvalue::Local {
                frame: self.cur_frame(),
                local,
            },

            Static(ref static_) => {
                let instance = ty::Instance::mono(self.tcx, static_.def_id);
                Lvalue::Global(GlobalId {
                    instance,
                    promoted: None,
                })
            }

            Projection(ref proj) => {
                let ty = self.lvalue_ty(&proj.base);
                let lvalue = self.eval_lvalue(&proj.base)?;
                return self.eval_lvalue_projection(lvalue, ty, &proj.elem);
            }
        };

        if log_enabled!(::log::LogLevel::Trace) {
            self.dump_local(lvalue);
        }

        Ok(lvalue)
    }

    pub fn lvalue_field(
        &mut self,
        base: Lvalue<'tcx>,
        field_index: usize,
        base_ty: Ty<'tcx>,
        field_ty: Ty<'tcx>,
    ) -> EvalResult<'tcx, Lvalue<'tcx>> {
        let base_layout = self.type_layout(base_ty)?;
        use rustc::ty::layout::Layout::*;
        let (offset, packed) = match *base_layout {
            Univariant { ref variant, .. } => (variant.offsets[field_index], variant.packed),

            General { ref variants, .. } => {
                let (_, base_extra, _) = base.to_ptr_extra_aligned();
                if let LvalueExtra::DowncastVariant(variant_idx) = base_extra {
                    // +1 for the discriminant, which is field 0
                    (
                        variants[variant_idx].offsets[field_index + 1],
                        variants[variant_idx].packed,
                    )
                } else {
                    bug!("field access on enum had no variant index");
                }
            }

            RawNullablePointer { .. } => {
                assert_eq!(field_index, 0);
                return Ok(base);
            }

            StructWrappedNullablePointer { ref nonnull, .. } => {
                (nonnull.offsets[field_index], nonnull.packed)
            }

            UntaggedUnion { .. } => return Ok(base),

            Vector { element, count } => {
                let field = field_index as u64;
                assert!(field < count);
                let elem_size = element.size(&self.tcx.data_layout).bytes();
                (Size::from_bytes(field * elem_size), false)
            }

            // We treat arrays + fixed sized indexing like field accesses
            Array { .. } => {
                let field = field_index as u64;
                let elem_size = match base_ty.sty {
                    ty::TyArray(elem_ty, n) => {
                        assert!(field < n as u64);
                        self.type_size(elem_ty)?.expect("array elements are sized") as u64
                    }
                    _ => {
                        bug!(
                            "lvalue_field: got Array layout but non-array type {:?}",
                            base_ty
                        )
                    }
                };
                (Size::from_bytes(field * elem_size), false)
            }

            FatPointer { .. } => {
                let bytes = field_index as u64 * self.memory.pointer_size();
                let offset = Size::from_bytes(bytes);
                (offset, false)
            }

            _ => bug!("field access on non-product type: {:?}", base_layout),
        };

        // Do not allocate in trivial cases
        let (base_ptr, base_extra, aligned) = match base {
            Lvalue::Ptr {
                ptr,
                extra,
                aligned,
            } => (ptr, extra, aligned),
            Lvalue::Local { frame, local } => {
                match self.stack[frame].get_local(local)? {
                    // in case the type has a single field, just return the value
                    Value::ByVal(_)
                        if self.get_field_count(base_ty).map(|c| c == 1).unwrap_or(
                            false,
                        ) => {
                        assert_eq!(
                            offset.bytes(),
                            0,
                            "ByVal can only have 1 non zst field with offset 0"
                        );
                        return Ok(base);
                    }
                    Value::ByRef { .. } |
                    Value::ByValPair(..) |
                    Value::ByVal(_) => self.force_allocation(base)?.to_ptr_extra_aligned(),
                }
            }
            Lvalue::Global(cid) => {
                match self.globals.get(&cid).expect("uncached global").value {
                    // in case the type has a single field, just return the value
                    Value::ByVal(_)
                        if self.get_field_count(base_ty).map(|c| c == 1).unwrap_or(
                            false,
                        ) => {
                        assert_eq!(
                            offset.bytes(),
                            0,
                            "ByVal can only have 1 non zst field with offset 0"
                        );
                        return Ok(base);
                    }
                    Value::ByRef { .. } |
                    Value::ByValPair(..) |
                    Value::ByVal(_) => self.force_allocation(base)?.to_ptr_extra_aligned(),
                }
            }
        };

        let offset = match base_extra {
            LvalueExtra::Vtable(tab) => {
                let (_, align) = self.size_and_align_of_dst(
                    base_ty,
                    base_ptr.to_value_with_vtable(tab),
                )?;
                offset
                    .abi_align(Align::from_bytes(align, align).unwrap())
                    .bytes()
            }
            _ => offset.bytes(),
        };

        let ptr = base_ptr.offset(offset, &self)?;

        let field_ty = self.monomorphize(field_ty, self.substs());

        let extra = if self.type_is_sized(field_ty) {
            LvalueExtra::None
        } else {
            match base_extra {
                LvalueExtra::None => bug!("expected fat pointer"),
                LvalueExtra::DowncastVariant(..) => {
                    bug!("Rust doesn't support unsized fields in enum variants")
                }
                LvalueExtra::Vtable(_) |
                LvalueExtra::Length(_) => {}
            }
            base_extra
        };

        Ok(Lvalue::Ptr {
            ptr,
            extra,
            aligned: aligned && !packed,
        })
    }

    pub(super) fn val_to_lvalue(&self, val: Value, ty: Ty<'tcx>) -> EvalResult<'tcx, Lvalue<'tcx>> {
        Ok(match self.tcx.struct_tail(ty).sty {
            ty::TyDynamic(..) => {
                let (ptr, vtable) = val.into_ptr_vtable_pair(&self.memory)?;
                Lvalue::Ptr {
                    ptr,
                    extra: LvalueExtra::Vtable(vtable),
                    aligned: true,
                }
            }
            ty::TyStr | ty::TySlice(_) => {
                let (ptr, len) = val.into_slice(&self.memory)?;
                Lvalue::Ptr {
                    ptr,
                    extra: LvalueExtra::Length(len),
                    aligned: true,
                }
            }
            _ => Lvalue::Ptr {
                ptr: val.into_ptr(&self.memory)?,
                extra: LvalueExtra::None,
                aligned: true,
            },
        })
    }

    pub(super) fn lvalue_index(
        &mut self,
        base: Lvalue<'tcx>,
        outer_ty: Ty<'tcx>,
        n: u64,
    ) -> EvalResult<'tcx, Lvalue<'tcx>> {
        // Taking the outer type here may seem odd;
        // it's needed because for array types, the outer type gives away the length.
        let base = self.force_allocation(base)?;
        let (base_ptr, _, aligned) = base.to_ptr_extra_aligned();

        let (elem_ty, len) = base.elem_ty_and_len(outer_ty);
        let elem_size = self.type_size(elem_ty)?.expect(
            "slice element must be sized",
        );
        assert!(
            n < len,
            "Tried to access element {} of array/slice with length {}",
            n,
            len
        );
        let ptr = base_ptr.offset(n * elem_size, self.memory.layout)?;
        Ok(Lvalue::Ptr {
            ptr,
            extra: LvalueExtra::None,
            aligned,
        })
    }

    pub(super) fn eval_lvalue_projection(
        &mut self,
        base: Lvalue<'tcx>,
        base_ty: Ty<'tcx>,
        proj_elem: &mir::ProjectionElem<'tcx, mir::Operand<'tcx>, Ty<'tcx>>,
    ) -> EvalResult<'tcx, Lvalue<'tcx>> {
        use rustc::mir::ProjectionElem::*;
        let (ptr, extra, aligned) = match *proj_elem {
            Field(field, field_ty) => {
                return self.lvalue_field(base, field.index(), base_ty, field_ty);
            }

            Downcast(_, variant) => {
                let base_layout = self.type_layout(base_ty)?;
                // FIXME(solson)
                let base = self.force_allocation(base)?;
                let (base_ptr, base_extra, aligned) = base.to_ptr_extra_aligned();

                use rustc::ty::layout::Layout::*;
                let extra = match *base_layout {
                    General { .. } => LvalueExtra::DowncastVariant(variant),
                    RawNullablePointer { .. } |
                    StructWrappedNullablePointer { .. } => base_extra,
                    _ => bug!("variant downcast on non-aggregate: {:?}", base_layout),
                };
                (base_ptr, extra, aligned)
            }

            Deref => {
                let val = self.read_lvalue(base)?;

                let pointee_type = match base_ty.sty {
                    ty::TyRawPtr(ref tam) |
                    ty::TyRef(_, ref tam) => tam.ty,
                    ty::TyAdt(def, _) if def.is_box() => base_ty.boxed_ty(),
                    _ => bug!("can only deref pointer types"),
                };

                trace!("deref to {} on {:?}", pointee_type, val);

                return self.val_to_lvalue(val, pointee_type);
            }

            Index(ref operand) => {
                // FIXME(solson)
                let n_ptr = self.eval_operand(operand)?;
                let usize = self.tcx.types.usize;
                let n = self.value_to_primval(n_ptr, usize)?.to_u64()?;
                return self.lvalue_index(base, base_ty, n);
            }

            ConstantIndex {
                offset,
                min_length,
                from_end,
            } => {
                // FIXME(solson)
                let base = self.force_allocation(base)?;
                let (base_ptr, _, aligned) = base.to_ptr_extra_aligned();

                let (elem_ty, n) = base.elem_ty_and_len(base_ty);
                let elem_size = self.type_size(elem_ty)?.expect(
                    "sequence element must be sized",
                );
                assert!(n >= min_length as u64);

                let index = if from_end {
                    n - u64::from(offset)
                } else {
                    u64::from(offset)
                };

                let ptr = base_ptr.offset(index * elem_size, &self)?;
                (ptr, LvalueExtra::None, aligned)
            }

            Subslice { from, to } => {
                // FIXME(solson)
                let base = self.force_allocation(base)?;
                let (base_ptr, _, aligned) = base.to_ptr_extra_aligned();

                let (elem_ty, n) = base.elem_ty_and_len(base_ty);
                let elem_size = self.type_size(elem_ty)?.expect(
                    "slice element must be sized",
                );
                assert!(u64::from(from) <= n - u64::from(to));
                let ptr = base_ptr.offset(u64::from(from) * elem_size, &self)?;
                let extra = LvalueExtra::Length(n - u64::from(to) - u64::from(from));
                (ptr, extra, aligned)
            }
        };

        Ok(Lvalue::Ptr {
            ptr,
            extra,
            aligned,
        })
    }

    pub(super) fn lvalue_ty(&self, lvalue: &mir::Lvalue<'tcx>) -> Ty<'tcx> {
        self.monomorphize(
            lvalue.ty(self.mir(), self.tcx).to_ty(self.tcx),
            self.substs(),
        )
    }
}
