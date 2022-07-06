use crate::helpers::EvalContextExt as HEvalContext;
use crate::sync::EvalContextExt as SEvalContext;
use crate::thread::EvalContextExt as TEvalContext;
use crate::InterpResult;
use crate::OpTy;
use crate::{MiriEvalContext, Tag};
use std::fmt::Pointer;

enum OpType {
    UmtxOpWait = 2,
    //TODO: other types
}

impl TryFrom<i32> for OpType {
    type Error = &'static str;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            2 => Ok(OpType::UmtxOpWait),
            _ => Err("Unsupported Futex operation"),
        }
    }
}

struct Futex<'mir, 'tcx> {}

impl<'mir, 'tcx: 'mir> Futex<'mir, 'tcx> {
    /// Function that performs a specific action on a futex thread
    /// https://www.freebsd.org/cgi/man.cgi?query=_umtx_op&sektion=2&n=1
    ///
    /// # Arguments
    ///
    /// * `this`: Context of the evaluation
    /// * `obj`: Pointer to a variable of type long
    /// * `op`: Futex operation to perform on `obj`
    /// * `val`: Depends on the operation performed (see man pages)
    /// * `uaddr`: Depends on the operation performed (see man pages)
    /// * `uaddr2`: Depends on the operation performed (see man pages)
    ///
    /// returns: InterpResult<'tcx>
    pub fn futex<'tcx>(
        this: &mut MiriEvalContext<'_, 'tcx>,
        // Object to operate on
        obj: &OpTy<'tcx, Tag>,
        // Operation type
        op: i32,
        // Current value pointed to by `obj`
        val: u64,
        // Pointer that's purpose depends on the op_type
        //uaddr: &OpTy<'tcx, Tag>,
        uaddr: Pointer<Option<Tag>>,
        // Pointer that's purpose depends on the op_type
        uaddr2: Pointer<<Option<Tag>>,
    ) -> InterpResult<'tcx> {
        match OpType::try_from(op) {
            Ok(op) =>
                match op {
                    OpType::UmtxOpWait =>
                        if this.deref_operand(this.read_scalar(obj)?.to_machine_usize()?) == val {
                            this.futex_wait(
                                this.read_scalar(obj)?.to_machine_usize()?,
                                this.get_active_thread(),
                                u32::MAX,
                            );
                            Ok(())
                        } else {
                            // The `val` value is invalid. Double check this against the manual.
                            let einval = this.eval_libc("EINVAL")?;
                            this.set_last_error(einval)?;
                            Ok(())
                        },
                },
            Err(_) => {
                // The `op` value is invalid.
                let einval = this.eval_libc("EINVAL")?;
                this.set_last_error(einval)?;
                Ok(())
            }
        }
    }
}
