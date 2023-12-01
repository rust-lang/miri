use rustc_span::Symbol;
use rustc_target::spec::abi::Abi;

use crate::*;
use shims::foreign_items::EmulateForeignItemResult;
use shims::unix::thread::EvalContextExt as _;

pub fn is_dyn_sym(_name: &str) -> bool {
    false
}

impl<'mir, 'tcx: 'mir> EvalContextExt<'mir, 'tcx> for crate::MiriInterpCx<'mir, 'tcx> {}
pub trait EvalContextExt<'mir, 'tcx: 'mir>: crate::MiriInterpCxExt<'mir, 'tcx> {
    fn emulate_foreign_item_inner(
        &mut self,
        link_name: Symbol,
        abi: Abi,
        args: &[OpTy<'tcx, Provenance>],
        dest: &PlaceTy<'tcx, Provenance>,
    ) -> InterpResult<'tcx, EmulateForeignItemResult> {
        let this = self.eval_context_mut();
        match link_name.as_str() {
            // errno
            "__errno" => {
                let [] = this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;
                let errno_place = this.last_error_place()?;
                this.write_scalar(errno_place.to_ref(this).to_scalar(), dest)?;
            }

            // Threading
            "pthread_setname_np" => {
                let [thread, name] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;
                // The thread name can have a size up to PTHREAD_MAX_NAMELEN_NP(32)
                // https://illumos.org/man/3C/pthread_getname_np
                let max_len = 32;
                let res = this.pthread_setname_np(
                    this.read_scalar(thread)?,
                    this.read_scalar(name)?,
                    max_len,
                )?;
                this.write_scalar(res, dest)?;
            }
            "pthread_getname_np" => {
                let [thread, name, len] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;
                let res = this.pthread_getname_np(
                    this.read_scalar(thread)?,
                    this.read_scalar(name)?,
                    this.read_scalar(len)?,
                )?;
                this.write_scalar(res, dest)?;
            }
            "getrandom" => {
                let [ptr, len, flags] =
                    this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;
                let ptr = this.read_pointer(ptr)?;
                let len = this.read_target_usize(len)?;
                let _flags = this.read_scalar(flags)?.to_i32()?;
                // GRND_RANDOM, nor GRND_NONBLOCK have any effect on the PRNG.
                // https://smartos.org/man/2/getrandom
                this.gen_random(ptr, len)?;
                this.write_scalar(Scalar::from_target_usize(len, this), dest)?;
            }

            _ => return Ok(EmulateForeignItemResult::NotSupported),
        }
        Ok(EmulateForeignItemResult::NeedsJumping)
    }
}
