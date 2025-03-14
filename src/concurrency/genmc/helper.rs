use rustc_abi::Size;
use rustc_const_eval::interpret::{InterpCx, InterpResult, interp_ok};
use rustc_middle::mir::{Terminator, TerminatorKind};
use rustc_middle::ty::{self, ScalarInt, Ty};
use tracing::info;

use super::GenmcScalar;
use crate::alloc_addresses::EvalContextExt as _;
use crate::{
    BorTag, MiriInterpCx, MiriMachine, Pointer, Provenance, Scalar, ThreadId, throw_unsup_format,
};

const MEM_ACCESS_MAX_SIZE_BYTES: u64 = 8;

pub fn split_access(address: Size, size: Size) -> impl Iterator<Item = (u64, u64)> {
    // Handle possible misalignment: TODO GENMC: could always do largest power-of-two here
    let size_bytes = size.bytes();

    let start_address = address.bytes();
    let end_address = start_address + size_bytes;
    // TODO GENMC: optimize this:
    let start_missing = (MEM_ACCESS_MAX_SIZE_BYTES - (start_address % MEM_ACCESS_MAX_SIZE_BYTES))
        % MEM_ACCESS_MAX_SIZE_BYTES;
    let end_missing = end_address % MEM_ACCESS_MAX_SIZE_BYTES;

    let start_address_aligned = start_address + start_missing;
    let end_address_aligned = end_address - end_missing;

    info!(
        "GenMC: splitting NA memory access into {MEM_ACCESS_MAX_SIZE_BYTES} byte chunks: {start_missing}B + {} * {MEM_ACCESS_MAX_SIZE_BYTES}B + {end_missing}B = {size:?}",
        (end_address_aligned - start_address_aligned) / MEM_ACCESS_MAX_SIZE_BYTES
    );
    debug_assert_eq!(
        0,
        start_address_aligned % MEM_ACCESS_MAX_SIZE_BYTES,
        "Incorrectly aligned start address: {start_address_aligned} % {MEM_ACCESS_MAX_SIZE_BYTES} != 0, {start_address} + {start_missing}"
    );
    debug_assert_eq!(
        0,
        end_address_aligned % MEM_ACCESS_MAX_SIZE_BYTES,
        "Incorrectly aligned end address: {end_address_aligned} % {MEM_ACCESS_MAX_SIZE_BYTES} != 0, {end_address} - {end_missing}"
    );
    debug_assert!(
        start_missing < MEM_ACCESS_MAX_SIZE_BYTES && end_missing < MEM_ACCESS_MAX_SIZE_BYTES
    );

    let start_chunks = (start_address..start_address_aligned).map(|address| (address, 1));
    let aligned_chunks = (start_address_aligned..end_address_aligned)
        .step_by(MEM_ACCESS_MAX_SIZE_BYTES.try_into().unwrap())
        .map(|address| (address, MEM_ACCESS_MAX_SIZE_BYTES));
    let end_chunks = (end_address_aligned..end_address).map(|address| (address, 1));

    start_chunks.chain(aligned_chunks).chain(end_chunks)
}

/// Convert an address (originally selected by GenMC) back into form that GenMC expects.
pub fn size_to_genmc(miri_address: Size) -> u64 {
    miri_address.bytes()
}

/// Like `scalar_to_genmc_scalar`, but returns an error if the scalar is not an integer
pub fn rhs_scalar_to_genmc_scalar<'tcx>(
    ecx: &MiriInterpCx<'tcx>,
    scalar: Scalar,
) -> InterpResult<'tcx, GenmcScalar> {
    if matches!(scalar, Scalar::Ptr(..)) {
        throw_unsup_format!("Right hand side of atomic operation cannot be a pointer");
    }
    scalar_to_genmc_scalar(ecx, scalar)
}

pub fn option_scalar_to_genmc_scalar<'tcx>(
    ecx: &MiriInterpCx<'tcx>,
    maybe_scalar: Option<Scalar>,
) -> InterpResult<'tcx, GenmcScalar> {
    if let Some(scalar) = maybe_scalar {
        scalar_to_genmc_scalar(ecx, scalar)
    } else {
        interp_ok(GenmcScalar::UNINIT)
    }
}

pub fn scalar_to_genmc_scalar<'tcx>(
    ecx: &MiriInterpCx<'tcx>,
    scalar: Scalar,
) -> InterpResult<'tcx, GenmcScalar> {
    interp_ok(match scalar {
        rustc_const_eval::interpret::Scalar::Int(scalar_int) => {
            // TODO GENMC: u128 support
            let value: u64 = scalar_int.to_uint(scalar_int.size()).try_into().unwrap(); // TODO GENMC: doesn't work for size != 8
            GenmcScalar { value, extra: 0, is_init: true }
        }
        rustc_const_eval::interpret::Scalar::Ptr(pointer, size) => {
            let addr = Pointer::from(pointer).addr();
            if let Provenance::Wildcard = pointer.provenance {
                throw_unsup_format!("Pointers with wildcard provenance not allowed in GenMC mode");
            }
            let (alloc_id, _size, _prov_extra) =
                rustc_const_eval::interpret::Machine::ptr_get_alloc(ecx, pointer, size.into())
                    .unwrap();
            let base_addr = ecx.addr_from_alloc_id(alloc_id, None)?;
            GenmcScalar { value: addr.bytes(), extra: base_addr, is_init: true }
        }
    })
}

pub fn genmc_scalar_to_scalar<'tcx>(
    ecx: &MiriInterpCx<'tcx>,
    scalar: GenmcScalar,
    size: Size,
) -> InterpResult<'tcx, Scalar> {
    // TODO GENMC: proper handling of large integers
    // TODO GENMC: proper handling of pointers (currently assumes all integers)

    if scalar.extra != 0 {
        // We have a pointer!

        let addr = Size::from_bytes(scalar.value);
        let base_addr = scalar.extra;

        let alloc_size = 0; // TODO GENMC: what is the correct size here? Is 0 ok?
        let only_exposed_allocations = false;
        let Some(alloc_id) =
            ecx.alloc_id_from_addr(base_addr, alloc_size, only_exposed_allocations)
        else {
            // TODO GENMC: what is the correct error in this case?
            throw_unsup_format!(
                "Cannot get allocation id of pointer received from GenMC (base address: 0x{base_addr:x}, pointer address: 0x{:x})",
                addr.bytes()
            );
        };

        // TODO GENMC: is using `size: Size` ok here? Can we ever have `size != sizeof pointer`?

        // FIXME: Currently GenMC mode incompatible with aliasing model checking
        let tag = BorTag::default();
        let provenance = crate::machine::Provenance::Concrete { alloc_id, tag };
        let offset = addr;
        let ptr = rustc_middle::mir::interpret::Pointer::new(provenance, offset);

        let size = size.bytes().try_into().unwrap();
        return interp_ok(Scalar::Ptr(ptr, size));
    }

    // TODO GENMC (HACK): since we give dummy values to GenMC for NA accesses, we need to be able to convert it back:
    let trunc_value = if size.bits() >= 64 {
        scalar.value
    } else {
        let mask = (1u64 << size.bits()) - 1;
        // let trunc_value = value & mask;
        // eprintln!(
        //     "Masking {value} = 0x{value:x} to size {size:?}, with mask 0x{mask:x}, result: {trunc_value} = 0x{trunc_value:x}"
        // );
        // trunc_value
        scalar.value & mask
    };

    let Some(value_scalar_int) = ScalarInt::try_from_uint(trunc_value, size) else {
        todo!(
            "GenMC: cannot currently convert GenMC value {} (0x{:x}) (truncated {trunc_value} = 0x{trunc_value:x}), with size {size:?} into a Miri Scalar",
            scalar.value,
            scalar.value,
        );
    };
    interp_ok(Scalar::Int(value_scalar_int))
}

pub fn is_terminator_atomic<'tcx>(
    ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
    terminator: &Terminator<'tcx>,
    thread_id: ThreadId,
    // _cache: &mut FxHashMap<ty::Ty<'tcx>, bool>,
) -> InterpResult<'tcx, bool> {
    match &terminator.kind {
        // All atomics are modeled as function calls to intrinsic functions.
        // The one exception is thread joining, but those are also calls.
        TerminatorKind::Call { func, .. } | TerminatorKind::TailCall { func, .. } => {
            let frame = ecx.machine.threads.get_thread_stack(thread_id).last().unwrap();
            let func_ty = func.ty(&frame.body().local_decls, *ecx.tcx);
            info!("GenMC: terminator is a call with operand: {func:?}, ty of operand: {func_ty:?}");

            is_function_atomic(ecx, func_ty)

            // match cache.entry(func_ty) {
            //     std::collections::hash_map::Entry::Occupied(occupied_entry) => {
            //         assert_eq!(is_function_atomic(ecx, func_ty)?, *occupied_entry.get());
            //         interp_ok(*occupied_entry.get())
            //     }
            //     std::collections::hash_map::Entry::Vacant(vacant_entry) => {
            //         let is_atomic = is_function_atomic(ecx, func_ty)?;
            //         vacant_entry.insert(is_atomic);
            //         interp_ok(is_atomic)
            //     }
            // }
            // }
        }
        _ => interp_ok(false),
    }
}

fn is_function_atomic<'tcx>(
    ecx: &InterpCx<'tcx, MiriMachine<'tcx>>,
    func_ty: Ty<'tcx>,
    // func: &Operand<'tcx>,
) -> InterpResult<'tcx, bool> {
    let callee_def_id = match func_ty.kind() {
        ty::FnDef(def_id, _args) => def_id,
        _ => return interp_ok(true), // we don't know the callee, might be an intrinsic or pthread_join
    };
    if ecx.tcx.is_foreign_item(*callee_def_id) {
        // Some shims, like pthread_join, must be considered loads. So just consider them all loads,
        // these calls are not *that* common.
        return interp_ok(true);
    }

    let Some(intrinsic_def) = ecx.tcx.intrinsic(callee_def_id) else {
        // TODO GENMC: Make this work for other platforms?
        let item_name = ecx.tcx.item_name(*callee_def_id);
        info!("GenMC:  function DefId: {callee_def_id:?}, item name: {item_name:?}");
        if matches!(item_name.as_str(), "pthread_join" | "WaitForSingleObject") {
            info!("GenMC:   found a 'join' terminator: '{}'", item_name.as_str(),);
            return interp_ok(true);
        }
        return interp_ok(false);
    };
    let intrinsice_name = intrinsic_def.name.as_str();
    info!("GenMC:   intrinsic name: '{intrinsice_name}'");
    // TODO GENMC(ENHANCEMENT): make this more precise (only loads). How can we make this maintainable?
    interp_ok(intrinsice_name.starts_with("atomic_"))
}
