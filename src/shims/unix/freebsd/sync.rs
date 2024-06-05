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

    throw_unsup_format!("Miri does not support `futex` syscall with op={}", op);
}
