use crate::*;
use rustc_middle::ty::Ty;
use rustc_middle::ty::Mutability;
use rustc_middle::ty::layout::LayoutOf;

pub fn futex<'tcx>(
    this: &mut MiriInterpCx<'tcx>,
    args: &[OpTy<'tcx>],
    _dest: &MPlaceTy<'tcx>,
) -> InterpResult<'tcx> {
    //Checking for the minimal amount of arguments required to proceed with
    if args.len() < 3 {
        throw_ub_format!(
            "incorrect number of arguments for `futex` syscall: got {}, expected at least 3",
            args.len()
        );
    }

    //void *obj
    let obj = this.read_pointer(&args[0])?;
    //int op
    let op = this.read_scalar(&args[1])?.to_i32()?;
    //u_long val
    let val = this.read_scalar(&args[2])?.to_i64()?;
    //void *uaddr
    let _uaddr = this.read_pointer(&args[3])?;
    //void *uaddr2
    let _uaddr2 = this.read_pointer(&args[4])?;

    let umtx_op_wait_uint_private = this.eval_libc_i32("UMTX_OP_WAIT_UINT_PRIVATE");
    let umtx_op_wake_private = this.eval_libc_i32("UMTX_OP_WAKE_PRIVATE");
    let umtx_op_nwake_private = this.eval_libc_i32("UMTX_OP_NWAKE_PRIVATE");

    let umtx_op_wait = this.eval_libc_i32("UMTX_OP_WAIT");

    //FUTEX private operations are not supported by miri
    match op & !umtx_op_wait_uint_private & !umtx_op_wake_private & !umtx_op_nwake_private {
        umtx_op_wait => {
            let ptr_layout = this.layout_of(Ty::new_ptr(this.tcx.tcx, this.tcx.types.i32, Mutability::Not))?;
            let obj_val = this.deref_pointer(&args[0].transmute(ptr_layout, this)?)?;
            let obj_val = this.read_scalar(&obj_val)?.to_i32()?;

            if obj_val as i64 == val {
                //wait
            }


            let result = 0;
            if result == 0 {
                return Ok(())
            }

        }
        _ => throw_unsup_format!("Miri does not support `futex` syscall with op={}", op),
    }
    Ok(())
}
