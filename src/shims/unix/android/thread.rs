use rustc_span::Symbol;
use rustc_target::spec::abi::Abi;

use crate::helpers::check_min_arg_count;
use crate::shims::unix::thread::EvalContextExt as _;
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

            let thread = this.pthread_self()?;
            let name = this.read_scalar(name)?;
            let name_len = Scalar::from_target_usize(TASK_COMM_LEN as u64, this);

            this.pthread_getname_np(thread, name, name_len)?
        }
        op => {
            this.handle_unsupported_foreign_item(format!("can't execute prctl with OP {op}"))?;
            return interp_ok(());
        }
    };
    this.write_scalar(res, dest)?;
    interp_ok(())
}
