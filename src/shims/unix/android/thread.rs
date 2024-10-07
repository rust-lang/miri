use std::iter;

use rustc_span::Symbol;
use rustc_target::spec::abi::Abi;

use crate::helpers::check_min_arg_count;
use crate::shims::unix::thread::{DEFAULT_THREAD_NAME, EvalContextExt as _};
use crate::*;

pub fn prctl<'tcx>(
    this: &mut MiriInterpCx<'tcx>,
    link_name: Symbol,
    abi: Abi,
    args: &[OpTy<'tcx>],
    dest: &MPlaceTy<'tcx>,
) -> InterpResult<'tcx> {
    // We do not use `check_shim` here because `prctl` is variadic. The argument
    // count is checked bellow.
    this.check_abi_and_shim_symbol_clash(abi, Abi::C { unwind: false }, link_name)?;

    // FIXME: Use PR_SET_NAME and PR_GET_NAME constants when
    // https://github.com/rust-lang/libc/pull/3941 lands.
    const PR_SET_NAME: i32 = 15;
    const PR_GET_NAME: i32 = 16;
    const TASK_COMM_LEN: usize = 16;

    let [op] = check_min_arg_count("prctl", args)?;
    let res = match this.read_scalar(op)?.to_i32()? {
        PR_SET_NAME => {
            let [_, name] = check_min_arg_count("prctl(PR_SET_NAME, ...)", args)?;

            let name = this.read_scalar(name)?.to_pointer(this)?;
            let name =
                this.read_c_str(name)?.iter().take(TASK_COMM_LEN - 1).copied().collect::<Vec<_>>();

            let thread = this.pthread_self()?.to_int(this.libc_ty_layout("pthread_t").size)?;
            let thread = ThreadId::try_from(thread).unwrap();

            this.set_thread_name(thread, name);
            Scalar::from_u32(0)
        }
        PR_GET_NAME => {
            let [_, name] = check_min_arg_count("prctl(PR_GET_NAME, ...)", args)?;

            let name_out = this.read_scalar(name)?;
            let name_out = name_out.to_pointer(this)?;

            let thread = this.pthread_self()?.to_int(this.libc_ty_layout("pthread_t").size)?;
            let thread = ThreadId::try_from(thread).unwrap();

            // FIXME: we should use the program name if the thread name is not set
            let name = this.get_thread_name(thread).unwrap_or(DEFAULT_THREAD_NAME).to_owned();
            let name_len = name.len().max(TASK_COMM_LEN - 1);

            this.eval_context_mut().write_bytes_ptr(
                name_out,
                name.iter()
                    .take(name_len)
                    .copied()
                    .chain(iter::repeat_n(0u8, TASK_COMM_LEN.strict_sub(name_len))),
            )?;

            Scalar::from_u32(0)
        }
        op => {
            this.handle_unsupported_foreign_item(format!("can't execute prctl with OP {op}"))?;
            return interp_ok(());
        }
    };
    this.write_scalar(res, dest)?;
    interp_ok(())
}
