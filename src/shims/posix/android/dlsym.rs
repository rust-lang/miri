use helpers::check_arg_count;
use log::trace;
use rustc_middle::mir;

use crate::*;

#[derive(Debug, Copy, Clone)]
#[allow(non_camel_case_types)]
pub enum Dlsym {
    signal,
}

impl Dlsym {
    // Returns an error for unsupported symbols, and None if this symbol
    // should become a NULL pointer (pretend it does not exist).
    pub fn from_str<'tcx>(name: &str) -> InterpResult<'tcx, Option<Dlsym>> {
        Ok(match &*name {
            "__pthread_get_minstack" => None,
            "getrandom" => None, // std falls back to syscall(SYS_getrandom, ...) when this is NULL.
            "statx" => None,     // std falls back to syscall(SYS_statx, ...) when this is NULL.
            "signal" | "bsd_signal" => Some(Dlsym::signal), // these have the same signature/implementation
            "android_set_abort_message" => None, // std falls back to just not doing anything when this is NULL.
            _ => throw_unsup_format!("unsupported Android dlsym: {}", name),
        })
    }
}

impl<'mir, 'tcx: 'mir> EvalContextExt<'mir, 'tcx> for crate::MiriEvalContext<'mir, 'tcx> {}
pub trait EvalContextExt<'mir, 'tcx: 'mir>: crate::MiriEvalContextExt<'mir, 'tcx> {
    fn call_dlsym(
        &mut self,
        dlsym: Dlsym,
        args: &[OpTy<'tcx, Tag>],
        dest: &PlaceTy<'tcx, Tag>,
        ret: Option<mir::BasicBlock>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        assert!(this.tcx.sess.target.os == "android");

        let ret = ret.expect("we don't support any diverging dlsym");

        match dlsym {
            Dlsym::signal => {
                let &[ref _sig, ref _func] = check_arg_count(args)?;
                this.write_null(dest)?;
            }
        }

        trace!("{:?}", this.dump_place(**dest));
        this.go_to_block(ret);
        Ok(())
    }
}
