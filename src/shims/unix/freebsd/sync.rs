use crate::*;

pub fn futex<'tcx>(
    this: &mut MiriInterpCx<'tcx>,
    args: &[OpTy<'tcx>],
    _dest: &MPlaceTy<'tcx>,
) -> InterpResult<'tcx> {
    //void *obj
    let _obj = this.read_pointer(&args[0])?;
    //int op
    let op = this.read_scalar(&args[1])?.to_i32()?;
    //u_long val
    let _val = this.read_scalar(&args[2])?.to_u64()?;
    //void *uaddr
    let _uaddr = this.read_pointer(&args[3])?;
    //void *uaddr2
    let _uaddr2 = this.read_pointer(&args[4])?;

    let umtx_op_wait_uint_private = this.eval_libc_i32("UMTX_OP_WAIT_UINT_PRIVATE");
    let umtx_op_wake_private = this.eval_libc_i32("UMTX_OP_WAKE_PRIVATE");
    let umtx_op_nwake_private = this.eval_libc_i32("UMTX_OP_NWAKE_PRIVATE");

    //FUTEX private operations are not supported by miri
    match op & !umtx_op_wait_uint_private & !umtx_op_wake_private & !umtx_op_nwake_private {
        _ => throw_unsup_format!("Miri does not support `futex` syscall with op={}", op),
    }
}
