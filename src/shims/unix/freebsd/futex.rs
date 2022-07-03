use crate::helpers::EvalContextExt as HEvalContext;
use crate::sync::EvalContextExt as SEvalContext;
use crate::thread::EvalContextExt as TEvalContext;
use crate::InterpResult;
use crate::OpTy;
use crate::{MiriEvalContext, Tag};

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

pub fn futex<'tcx>(
    this: &mut MiriEvalContext<'_, 'tcx>,
    obj: &OpTy<'tcx, Tag>,
    op: i32,
    val: u64,
    uaddr: &OpTy<'tcx, Tag>,
    uaddr2: &OpTy<'tcx, Tag>,
) -> Option<InterpResult<'tcx>> {
    // Object to operate on
    let obj = obj;
    // Operation type
    let _op_type = op;
    // Current value pointed to by `obj`
    let val = val;
    // Pointer that's purpose depends on the op_type
    let _uaddr = this.read_pointer(uaddr).ok()?;
    // Pointer that's purpose depends on the op_type
    let _uaddr2 = this.read_pointer(uaddr2).ok()?;
    match OpType::try_from(op) {
        Ok(op) =>
            match op {
                OpType::UmtxOpWait =>
                    if this.read_scalar(obj).ok()?.to_u64().ok()? == val {
                        this.futex_wait(
                            this.read_scalar(obj).ok()?.to_u64().ok()?,
                            this.get_active_thread(),
                            u32::MAX,
                        );
                        None
                    } else {
                        // The `val` value is invalid. Double check this against the manual.
                        let einval = this.eval_libc("EINVAL").ok()?;
                        this.set_last_error(einval).ok()?;
                        None
                    },
            },
        Err(_) => {
            // The `op` value is invalid.
            let einval = this.eval_libc("EINVAL").ok()?;
            this.set_last_error(einval).ok()?;
            None
        }
    }
}
