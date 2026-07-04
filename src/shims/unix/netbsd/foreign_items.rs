use rustc_abi::CanonAbi;
use rustc_middle::ty::Ty;
use rustc_span::Symbol;
use rustc_target::callconv::FnAbi;

use super::lwp::EvalContextExt as _;
use crate::shims::unix::ThreadNameResult;
use crate::shims::unix::thread::EvalContextExt as _;
use crate::*;

// See https://github.com/NetBSD/src/blob/adc9c78bb5681db46effe4b421961463a5156f50/lib/libpthread/pthread.h#L281
const PTHREAD_MAX_NAMELEN_NP: u64 = 32;

pub fn is_dyn_sym(_name: &str) -> bool {
    false
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn emulate_foreign_item_inner(
        &mut self,
        link_name: Symbol,
        abi: &FnAbi<'tcx, Ty<'tcx>>,
        args: &[OpTy<'tcx>],
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx, EmulateItemResult> {
        let this = self.eval_context_mut();
        match link_name.as_str() {
            // Threading
            "pthread_setname_np" => {
                let [thread, template, arg] =
                    this.check_shim_sig_lenient(abi, CanonAbi::C, link_name, args)?;

                let template = this.read_pointer(template)?;
                if this.read_c_str(template)? != b"%s" {
                    throw_unsup_format!(
                        "`pthread_setname_np` with a non-trivial template is not supported"
                    );
                }

                let res = match this.pthread_setname_np(
                    this.read_scalar(thread)?,
                    this.read_scalar(arg)?,
                    PTHREAD_MAX_NAMELEN_NP,
                    /* truncate */ false,
                )? {
                    ThreadNameResult::Ok => Scalar::from_u32(0),
                    ThreadNameResult::NameTooLong => this.eval_libc("EINVAL"),
                    ThreadNameResult::ThreadNotFound => this.eval_libc("ESRCH"),
                };
                this.write_scalar(res, dest)?;
            }
            "pthread_getname_np" => {
                let [thread, name, len] =
                    this.check_shim_sig_lenient(abi, CanonAbi::C, link_name, args)?;

                let res = match this.pthread_getname_np(
                    this.read_scalar(thread)?,
                    this.read_scalar(name)?,
                    this.read_scalar(len)?,
                    /* truncate*/ true,
                )? {
                    ThreadNameResult::Ok => Scalar::from_u32(0),
                    ThreadNameResult::NameTooLong => unreachable!(),
                    ThreadNameResult::ThreadNotFound => this.eval_libc("ESRCH"),
                };
                this.write_scalar(res, dest)?;
            }
            "_lwp_self" => {
                let [] = this.check_shim_sig_lenient(abi, CanonAbi::C, link_name, args)?;
                let result = this.lwp_self()?;
                this.write_scalar(result, dest)?;
            }
            "___lwp_park60" => {
                let [clock_id, flags, ts, unpark, hint, unparkhint] =
                    this.check_shim_sig_lenient(abi, CanonAbi::C, link_name, args)?;
                this.lwp_park(clock_id, flags, ts, unpark, hint, unparkhint, dest)?;
            }
            "_lwp_unpark" => {
                let [lwp, hint] = this.check_shim_sig_lenient(abi, CanonAbi::C, link_name, args)?;
                this.lwp_unpark(lwp, hint, dest)?;
            }

            // Miscellaneous
            "__errno" => {
                let [] = this.check_shim_sig_lenient(abi, CanonAbi::C, link_name, args)?;
                let errno_place = this.last_error_place()?;
                this.write_scalar(errno_place.to_ref(this).to_scalar(), dest)?;
            }

            // Incomplete shims that we "stub out" just to get pre-main initialization code to work.
            // These shims are enabled only when the caller is in the standard library.
            "_cpuset_create" if this.frame_in_std() => {
                let [] = this.check_shim_sig_lenient(abi, CanonAbi::C, link_name, args)?;
                this.write_null(dest)?;
            }
            _ => return interp_ok(EmulateItemResult::NotSupported),
        }
        interp_ok(EmulateItemResult::NeedsReturn)
    }
}
