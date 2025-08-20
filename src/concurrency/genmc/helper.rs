use rustc_abi::Size;
use rustc_const_eval::interpret::{InterpResult, interp_ok};
use rustc_middle::ty::ScalarInt;
use tracing::info;

use super::GenmcScalar;
use crate::alloc_addresses::EvalContextExt as _;
use crate::{BorTag, MiriInterpCx, Pointer, Provenance, Scalar, throw_unsup_format};

/// This function is used to split up a large memory access into aligned, non-overlapping chunks of a limited size.
/// Returns an iterator over the chunks, yielding `(base address, size)` of each chunk, ordered by address.
pub fn split_access(address: Size, size: Size) -> impl Iterator<Item = (u64, u64)> {
    /// Maximum size memory access in bytes that GenMC supports.
    const MAX_SIZE: u64 = 8;

    let size_bytes = size.bytes();

    let start_address = address.bytes();
    let end_address = start_address + size_bytes;
    let start_missing = (MAX_SIZE - (start_address % MAX_SIZE)) % MAX_SIZE;
    let end_missing = end_address % MAX_SIZE;

    let start_address_aligned = start_address + start_missing;
    let end_address_aligned = end_address - end_missing;

    info!(
        "GenMC: splitting NA memory access into {MAX_SIZE} byte chunks: {start_missing}B + {} * {MAX_SIZE}B + {end_missing}B = {size:?}",
        (end_address_aligned - start_address_aligned) / MAX_SIZE
    );
    debug_assert_eq!(
        0,
        start_address_aligned % MAX_SIZE,
        "Incorrectly aligned start address: {start_address_aligned} % {MAX_SIZE} != 0, {start_address} + {start_missing}"
    );
    debug_assert_eq!(
        0,
        end_address_aligned % MAX_SIZE,
        "Incorrectly aligned end address: {end_address_aligned} % {MAX_SIZE} != 0, {end_address} - {end_missing}"
    );
    debug_assert!(start_missing < MAX_SIZE && end_missing < MAX_SIZE);

    // FIXME(genmc): could make remaining accesses powers-of-2, instead of 1 byte.
    let start_chunks = (start_address..start_address_aligned).map(|address| (address, 1));
    let aligned_chunks = (start_address_aligned..end_address_aligned)
        .step_by(MAX_SIZE.try_into().unwrap())
        .map(|address| (address, MAX_SIZE));
    let end_chunks = (end_address_aligned..end_address).map(|address| (address, 1));

    start_chunks.chain(aligned_chunks).chain(end_chunks)
}

/// Inverse function to `scalar_to_genmc_scalar`.
///
/// Convert a Miri `Scalar` to a `GenmcScalar`.
/// To be able to restore pointer provenance from a `GenmcScalar`, the base address of the allocation of the pointer is also stored in the `GenmcScalar`.
/// We cannot use the `AllocId` instead of the base address, since Miri has no control over the `AllocId`, and it may change across executions.
/// Pointers with `Wildcard` provenance are not supported.
pub fn scalar_to_genmc_scalar<'tcx>(
    ecx: &MiriInterpCx<'tcx>,
    scalar: Scalar,
) -> InterpResult<'tcx, GenmcScalar> {
    interp_ok(match scalar {
        rustc_const_eval::interpret::Scalar::Int(scalar_int) => {
            // FIXME(genmc): Add u128 support once GenMC supports it.
            let value: u64 = scalar_int.to_uint(scalar_int.size()).try_into().unwrap();
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

/// Inverse function to `scalar_to_genmc_scalar`.
///
/// Convert a `GenmcScalar` back into a Miri `Scalar`.
/// For pointers, attempt to convert the stored base address of their allocation back into an `AllocId`.
pub fn genmc_scalar_to_scalar<'tcx>(
    ecx: &MiriInterpCx<'tcx>,
    scalar: GenmcScalar,
    size: Size,
) -> InterpResult<'tcx, Scalar> {
    if scalar.extra != 0 {
        // We have a pointer!

        let addr = Size::from_bytes(scalar.value);
        let base_addr = scalar.extra;

        let alloc_size = 0; // TODO GENMC: what is the correct size here? Is 0 ok?
        let only_exposed_allocations = false;
        let Some(alloc_id) =
            ecx.alloc_id_from_addr(base_addr, alloc_size, only_exposed_allocations)
        else {
            // TODO GENMC: what is the correct error in this case? Maybe give the pointer wildcard provenance instead?
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

    // NOTE: GenMC always returns 64 bit values, and the upper bits are not yet truncated.
    // FIXME(genmc): GenMC should be doing the truncation, not Miri.
    let (value_scalar_int, _got_truncated) = ScalarInt::truncate_from_uint(scalar.value, size);
    interp_ok(Scalar::Int(value_scalar_int))
}
