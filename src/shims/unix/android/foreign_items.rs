use rustc_span::Symbol;
use rustc_target::spec::abi::Abi;

use crate::shims::unix::*;
use crate::*;

pub fn is_dyn_sym(_name: &str) -> bool {
    false
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn emulate_foreign_item_inner(
        &mut self,
        link_name: Symbol,
        abi: Abi,
        args: &[OpTy<'tcx>],
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx, EmulateItemResult> {
        let this = self.eval_context_mut();
        match link_name.as_str() {
            // Miscellaneous
            "__errno" => {
                let [] = this.check_shim(abi, Abi::C { unwind: false }, link_name, args)?;
                let errno_place = this.last_error_place()?;
                this.write_scalar(errno_place.to_ref(this).to_scalar(), dest)?;
            }

            // Threading
            "prctl" => {
                // We do not use `check_shim` here because `prctl` is variadic. The argument
                // count is checked bellow.
                this.check_abi_and_shim_symbol_clash(abi, Abi::C { unwind: false }, link_name)?;

                check_args_len("prctl", args, 1)?;

                let id = this.read_scalar(&args[0])?.to_i32()?;
                // FIXME: Use PR_SET_NAME and PR_GET_NAME constants when
                // https://github.com/rust-lang/libc/pull/3941 lands.
                const PR_SET_NAME: i32 = 15;
                const PR_GET_NAME: i32 = 16;

                let res = match id {
                    PR_SET_NAME => {
                        check_args_len("'PR_SET_NAME' prctl", args, 2)?;

                        let tid = this.pthread_self()?;
                        let name = this.read_scalar(&args[1])?;
                        let name_len = 16;

                        this.pthread_setname_np(tid, name, name_len)?
                    }
                    PR_GET_NAME => {
                        check_args_len("'PR_GET_NAME' prctl", args, 2)?;

                        let tid = this.pthread_self()?;
                        let name = this.read_scalar(&args[1])?;
                        let name_len = Scalar::from_target_usize(16, this);

                        this.pthread_getname_np(tid, name, name_len)?
                    }
                    _ => {
                        this.handle_unsupported_foreign_item(format!(
                            "can't execute prctl with ID {id}"
                        ))?;
                        return interp_ok(EmulateItemResult::AlreadyJumped);
                    }
                };
                this.write_scalar(res, dest)?;
            }

            _ => return interp_ok(EmulateItemResult::NotSupported),
        }
        interp_ok(EmulateItemResult::NeedsReturn)
    }
}

fn check_args_len<'tcx>(
    link_name: &str,
    args: &[OpTy<'tcx>],
    args_expected: usize,
) -> InterpResult<'tcx, ()> {
    let args_actual = args.len();
    if args_actual < args_expected {
        throw_unsup_format!(
            "incorrect number of arguments for {link_name}: got {args_actual}, expected at least {args_expected}"
        );
    }
    interp_ok(())
}
