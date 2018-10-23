use rustc::ty::{Ty, layout::TyLayout};
use rustc::mir;

use super::*;

pub trait EvalContextExt<'tcx> {
    fn ptr_op(
        &self,
        bin_op: mir::BinOp,
        left: Scalar<Borrow>,
        left_layout: TyLayout<'tcx>,
        right: Scalar<Borrow>,
        right_layout: TyLayout<'tcx>,
    ) -> EvalResult<'tcx, (Scalar<Borrow>, bool)>;

    fn ptr_int_arithmetic(
        &self,
        bin_op: mir::BinOp,
        left: Pointer<Borrow>,
        right: u128,
        signed: bool,
    ) -> EvalResult<'tcx, (Scalar<Borrow>, bool)>;

    fn ptr_eq(
        &self,
        left: Scalar<Borrow>,
        right: Scalar<Borrow>,
        size: Size,
    ) -> EvalResult<'tcx, bool>;

    fn pointer_offset_inbounds(
        &self,
        ptr: Scalar<Borrow>,
        pointee_ty: Ty<'tcx>,
        offset: i64,
    ) -> EvalResult<'tcx, Scalar<Borrow>>;
}

impl<'a, 'mir, 'tcx> EvalContextExt<'tcx> for super::MiriEvalContext<'a, 'mir, 'tcx> {
    fn ptr_op(
        &self,
        bin_op: mir::BinOp,
        left: Scalar<Borrow>,
        left_layout: TyLayout<'tcx>,
        right: Scalar<Borrow>,
        right_layout: TyLayout<'tcx>,
    ) -> EvalResult<'tcx, (Scalar<Borrow>, bool)> {
        use rustc::mir::BinOp::*;

        trace!("ptr_op: {:?} {:?} {:?}", left, bin_op, right);
        debug_assert!(left.is_ptr() || right.is_ptr() || bin_op == Offset);

        match bin_op {
            Offset => {
                let pointee_ty = left_layout.ty
                    .builtin_deref(true)
                    .expect("Offset called on non-ptr type")
                    .ty;
                let ptr = self.pointer_offset_inbounds(
                    left,
                    pointee_ty,
                    right.to_isize(self)?,
                )?;
                Ok((ptr, false))
            }
            // These work on anything
            Eq =>
                Ok((Scalar::from_bool(self.ptr_eq(left, right, left_layout.size)?), false)),
            Ne =>
                Ok((Scalar::from_bool(!self.ptr_eq(left, right, left_layout.size)?), false)),
            // These need both to be pointer, and fail if they are not in the same location
            Lt | Le | Gt | Ge | Sub if left.is_ptr() && right.is_ptr() => {
                let left = left.to_ptr().expect("we checked is_ptr");
                let right = right.to_ptr().expect("we checked is_ptr");
                if left.alloc_id == right.alloc_id {
                    let res = match bin_op {
                        Lt => left.offset < right.offset,
                        Le => left.offset <= right.offset,
                        Gt => left.offset > right.offset,
                        Ge => left.offset >= right.offset,
                        Sub => {
                            // subtract the offsets
                            let left_offset = Scalar::from_uint(left.offset.bytes(), self.memory.pointer_size());
                            let right_offset = Scalar::from_uint(right.offset.bytes(), self.memory.pointer_size());
                            let layout = self.layout_of(self.tcx.types.usize)?;
                            return self.binary_op(
                                Sub,
                                left_offset, layout,
                                right_offset, layout,
                            )
                        }
                        _ => bug!("We already established it has to be one of these operators."),
                    };
                    Ok((Scalar::from_bool(res), false))
                } else {
                    // Both are pointers, but from different allocations.
                    err!(InvalidPointerMath)
                }
            }
            // These work if the left operand is a pointer, and the right an integer
            Add | BitAnd | Sub | Rem if left.is_ptr() && right.is_bits() => {
                // Cast to i128 is fine as we checked the kind to be ptr-sized
                self.ptr_int_arithmetic(
                    bin_op,
                    left.to_ptr().expect("we checked is_ptr"),
                    right.to_bits(self.memory.pointer_size()).expect("we checked is_bits"),
                    right_layout.abi.is_signed(),
                )
            }
            // Commutative operators also work if the integer is on the left
            Add | BitAnd if left.is_bits() && right.is_ptr() => {
                // This is a commutative operation, just swap the operands
                self.ptr_int_arithmetic(
                    bin_op,
                    right.to_ptr().expect("we checked is_ptr"),
                    left.to_bits(self.memory.pointer_size()).expect("we checked is_bits"),
                    left_layout.abi.is_signed(),
                )
            }
            // Nothing else works
            _ => err!(InvalidPointerMath),
        }
    }

    fn ptr_eq(
        &self,
        left: Scalar<Borrow>,
        right: Scalar<Borrow>,
        size: Size,
    ) -> EvalResult<'tcx, bool> {
        Ok(match (left, right) {
            (Scalar::Bits { .. }, Scalar::Bits { .. }) =>
                left.to_bits(size)? == right.to_bits(size)?,
            (Scalar::Ptr(left), Scalar::Ptr(right)) => {
                // Comparison illegal if one of them is out-of-bounds, *unless* they
                // are in the same allocation.
                if left.alloc_id == right.alloc_id {
                    left.offset == right.offset
                } else {
                    // This accepts one-past-the end.  So technically there is still
                    // some non-determinism that we do not fully rule out when two
                    // allocations sit right next to each other.  The C/C++ standards are
                    // somewhat fuzzy about this case, so I think for now this check is
                    // "good enough".
                    self.memory.get(left.alloc_id)?.check_bounds_ptr(left, false)?;
                    self.memory.get(right.alloc_id)?.check_bounds_ptr(right, false)?;
                    // Two live in-bounds pointers, we can compare across allocations
                    left == right
                }
            }
            // Comparing ptr and integer
            (Scalar::Ptr(ptr), Scalar::Bits { bits, size }) |
            (Scalar::Bits { bits, size }, Scalar::Ptr(ptr)) => {
                assert_eq!(size as u64, self.pointer_size().bytes());
                let bits = bits as u64;
                let (alloc_size, alloc_align) = self.memory.get_size_and_align(ptr.alloc_id);

                // Case I: Comparing with NULL
                if bits == 0 {
                    // Test if the ptr is in-bounds. Then it cannot be NULL.
                    if ptr.offset <= alloc_size {
                        return Ok(false);
                    }
                }
                // Case II: Alignment gives it away
                if ptr.offset.bytes() % alloc_align.abi() == 0 {
                    // The offset maintains the allocation alignment, so we know `base+offset`
                    // is aligned by `alloc_align`.
                    // FIXME: We could be even more general, e.g. offset 2 into a 4-aligned
                    // allocation cannot equal 3.
                    if bits % alloc_align.abi() != 0 {
                        // The integer is *not* aligned. So they cannot be equal.
                        return Ok(false);
                    }
                }
                // Case III: The integer is too big, and the allocation goes on a bit
                // without wrapping around the address space.
                {
                    // Compute the highest address at which this allocation could live.
                    // Substract one more, because it must be possible to add the size
                    // to the base address without overflowing -- IOW, the very last address
                    // of the address space is never dereferencable (but it can be in-bounds, i.e.,
                    // one-past-the-end).
                    let max_base_addr =
                        ((1u128 << self.pointer_size().bits())
                         - u128::from(alloc_size.bytes())
                         - 1
                        ) as u64;
                    if let Some(max_addr) = max_base_addr.checked_add(ptr.offset.bytes()) {
                        if bits > max_addr {
                            // The integer is too big, this cannot possibly be equal
                            return Ok(false)
                        }
                    }
                }

                // None of the supported cases.
                return err!(InvalidPointerMath);
            }
        })
    }

    fn ptr_int_arithmetic(
        &self,
        bin_op: mir::BinOp,
        left: Pointer<Borrow>,
        right: u128,
        signed: bool,
    ) -> EvalResult<'tcx, (Scalar<Borrow>, bool)> {
        use rustc::mir::BinOp::*;

        fn map_to_primval((res, over): (Pointer<Borrow>, bool)) -> (Scalar<Borrow>, bool) {
            (Scalar::Ptr(res), over)
        }

        Ok(match bin_op {
            Sub =>
                // The only way this can overflow is by underflowing, so signdeness of the right operands does not matter
                map_to_primval(left.overflowing_signed_offset(-(right as i128), self)),
            Add if signed =>
                map_to_primval(left.overflowing_signed_offset(right as i128, self)),
            Add if !signed =>
                map_to_primval(left.overflowing_offset(Size::from_bytes(right as u64), self)),

            BitAnd if !signed => {
                let ptr_base_align = self.memory.get(left.alloc_id)?.align.abi();
                let base_mask = {
                    // FIXME: Use interpret::truncate, once that takes a Size instead of a Layout
                    let shift = 128 - self.memory.pointer_size().bits();
                    let value = !(ptr_base_align as u128 - 1);
                    // truncate (shift left to drop out leftover values, shift right to fill with zeroes)
                    (value << shift) >> shift
                };
                let ptr_size = self.memory.pointer_size().bytes() as u8;
                trace!("Ptr BitAnd, align {}, operand {:#010x}, base_mask {:#010x}",
                    ptr_base_align, right, base_mask);
                if right & base_mask == base_mask {
                    // Case 1: The base address bits are all preserved, i.e., right is all-1 there
                    let offset = (left.offset.bytes() as u128 & right) as u64;
                    (
                        Scalar::Ptr(Pointer::new_with_tag(
                            left.alloc_id,
                            Size::from_bytes(offset),
                            left.tag,
                        )),
                        false,
                    )
                } else if right & base_mask == 0 {
                    // Case 2: The base address bits are all taken away, i.e., right is all-0 there
                    (Scalar::Bits { bits: (left.offset.bytes() as u128) & right, size: ptr_size }, false)
                } else {
                    return err!(ReadPointerAsBytes);
                }
            }

            Rem if !signed => {
                // Doing modulo a divisor of the alignment is allowed.
                // (Intuition: Modulo a divisor leaks less information.)
                let ptr_base_align = self.memory.get(left.alloc_id)?.align.abi();
                let right = right as u64;
                let ptr_size = self.memory.pointer_size().bytes() as u8;
                if right == 1 {
                    // modulo 1 is always 0
                    (Scalar::Bits { bits: 0, size: ptr_size }, false)
                } else if ptr_base_align % right == 0 {
                    // the base address would be cancelled out by the modulo operation, so we can
                    // just take the modulo of the offset
                    (Scalar::Bits { bits: (left.offset.bytes() % right) as u128, size: ptr_size }, false)
                } else {
                    return err!(ReadPointerAsBytes);
                }
            }

            _ => {
                let msg = format!("unimplemented binary op on pointer {:?}: {:?}, {:?} ({})", bin_op, left, right, if signed { "signed" } else { "unsigned" });
                return err!(Unimplemented(msg));
            }
        })
    }

    /// This function raises an error if the offset moves the pointer outside of its allocation.  We consider
    /// ZSTs their own huge allocation that doesn't overlap with anything (and nothing moves in there because the size is 0).
    /// We also consider the NULL pointer its own separate allocation, and all the remaining integers pointers their own
    /// allocation.
    fn pointer_offset_inbounds(
        &self,
        ptr: Scalar<Borrow>,
        pointee_ty: Ty<'tcx>,
        offset: i64,
    ) -> EvalResult<'tcx, Scalar<Borrow>> {
        // FIXME: assuming here that type size is < i64::max_value()
        let pointee_size = self.layout_of(pointee_ty)?.size.bytes() as i64;
        let offset = offset.checked_mul(pointee_size).ok_or_else(|| EvalErrorKind::Overflow(mir::BinOp::Mul))?;
        // Now let's see what kind of pointer this is
        if let Scalar::Ptr(ptr) = ptr {
            // Both old and new pointer must be in-bounds.
            // (Of the same allocation, but that part is trivial with our representation.)
            self.memory.get(ptr.alloc_id)?.check_bounds_ptr(ptr, false)?;
            let ptr = ptr.signed_offset(offset, self)?;
            self.memory.get(ptr.alloc_id)?.check_bounds_ptr(ptr, false)?;
            Ok(Scalar::Ptr(ptr))
        } else {
            // An integer pointer. They can only be offset by 0, and we pretend there
            // is a little zero-sized allocation here.
            if offset == 0 {
                Ok(ptr)
            } else {
                err!(InvalidPointerMath)
            }
        }
    }
}
