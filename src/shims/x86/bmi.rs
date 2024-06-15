use rustc_span::Symbol;
use rustc_target::spec::abi::Abi;

use crate::*;

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub(super) trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn emulate_x86_bmi_intrinsic(
        &mut self,
        link_name: Symbol,
        abi: Abi,
        args: &[OpTy<'tcx>],
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx, EmulateItemResult> {
        let this = self.eval_context_mut();
        // Prefix should have already been checked.
        let unprefixed_name = link_name.as_str().strip_prefix("llvm.x86.bmi.").unwrap();

        match unprefixed_name {
            "pdep.32" => {
                // <https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#text=_pdep_u32>
                this.expect_target_feature_for_intrinsic(link_name, "bmi2")?;

                let [source, mask] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;

                let source = this.read_scalar(source)?.to_u32()?;
                let mask = this.read_scalar(mask)?.to_u32()?;
                let destination = pdep(source as u64, mask as u64) as u32;

                this.write_scalar(Scalar::from_u32(destination), dest)?;
            }
            "pdep.64" => {
                // <https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#text=_pdep_u64>
                this.expect_target_feature_for_intrinsic(link_name, "bmi2")?;

                let [source, mask] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;

                let source = this.read_scalar(source)?.to_u64()?;
                let mask = this.read_scalar(mask)?.to_u64()?;
                let destination = pdep(source, mask);

                this.write_scalar(Scalar::from_u64(destination), dest)?;
            }
            "pext.32" => {
                // <https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#text=_pext_u32>
                this.expect_target_feature_for_intrinsic(link_name, "bmi2")?;

                let [source, mask] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;

                let source = this.read_scalar(source)?.to_u32()?;
                let mask = this.read_scalar(mask)?.to_u32()?;
                let destination = pext(source as u64, mask as u64) as u32;

                this.write_scalar(Scalar::from_u32(destination), dest)?;
            }
            "pext.64" => {
                // <https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#text=_pext_u64>
                this.expect_target_feature_for_intrinsic(link_name, "bmi2")?;

                let [source, mask] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;

                let source = this.read_scalar(source)?.to_u64()?;
                let mask = this.read_scalar(mask)?.to_u64()?;
                let destination = pext(source, mask);

                this.write_scalar(Scalar::from_u64(destination), dest)?;
            }
            _ => return Ok(EmulateItemResult::NotSupported),
        }
        Ok(EmulateItemResult::NeedsReturn)
    }
}

/// Parallel bit deposition
///
/// Deposit contiguous low bits from unsigned 64-bit integer `source` to `destination` at the corresponding bit locations
/// specified by `selector_mask`; all other bits in `destination` are set to zero.
///
/// See also
///
/// - https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#text=_pdep_u64
/// - https://en.wikipedia.org/wiki/X86_Bit_manipulation_instruction_set#Parallel_bit_deposit_and_extract
fn pdep(source: u64, selector_mask: u64) -> u64 {
    let mut destination = 0u64;
    let mut j = 0;
    for i in 0..64 {
        if selector_mask & (1 << i) != 0 {
            if source & (1 << j) != 0 {
                destination |= 1 << i;
            }

            j += 1;
        }
    }

    destination
}

/// Parallel bit extraction
///
/// Extract bits from unsigned 64-bit integer `source` at the corresponding bit locations specified by `selector_mask`
/// to contiguous low bits in `destination`; the remaining upper bits in `destination` are set to zero.
///
/// See also
///
/// - https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#text=_pext_u64
/// - https://en.wikipedia.org/wiki/X86_Bit_manipulation_instruction_set#Parallel_bit_deposit_and_extract
fn pext(source: u64, selector_mask: u64) -> u64 {
    let mut destination = 0u64;
    let mut j = 0;
    for i in 0..64 {
        if selector_mask & (1 << i) != 0 {
            if source & (1 << i) != 0 {
                destination |= 1 << j;
            }

            j += 1;
        }
    }

    destination
}
