use rustc_abi::Size;
use rustc_const_eval::interpret::{InterpResult, interp_ok};
use rustc_middle::ty::ScalarInt;
use tracing::info;

use super::GenmcScalar;
use crate::{MiriInterpCx, Scalar, throw_unsup_format};

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
    _ecx: &MiriInterpCx<'tcx>,
    scalar: Scalar,
) -> InterpResult<'tcx, GenmcScalar> {
    interp_ok(match scalar {
        rustc_const_eval::interpret::Scalar::Int(scalar_int) => {
            // FIXME(genmc): 128bit atomics support
            let value: u64 = scalar_int.to_uint(scalar_int.size()).try_into().unwrap();
            GenmcScalar { value, is_init: true }
        }
        rustc_const_eval::interpret::Scalar::Ptr(_pointer, _size) =>
            throw_unsup_format!(
                "FIXME(genmc): Implement sending pointers (with provenance) to GenMC."
            ),
    })
}

pub fn genmc_scalar_to_scalar<'tcx>(
    _ecx: &MiriInterpCx<'tcx>,
    scalar: GenmcScalar,
    size: Size,
) -> InterpResult<'tcx, Scalar> {
    // FIXME(genmc): Add GencmScalar to Miri Pointer conversion.

    // NOTE: GenMC always returns 64 bit values, and the upper bits are not yet truncated.
    // FIXME(genmc): GenMC should be doing the truncation, not Miri.
    let (value_scalar_int, _got_truncated) = ScalarInt::truncate_from_uint(scalar.value, size);
    interp_ok(Scalar::Int(value_scalar_int))
}
