//! Contains FreeBSD-specific synchronization functions

use core::time::Duration;

use crate::concurrency::sync::FutexRef;
use crate::*;

pub struct FreeBsdFutex {
    futex: FutexRef,
}

pub struct UmtxTime {
    timeout: Duration,
    abs_time: bool,
    timeout_clock: TimeoutClock,
}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    /// Implementation of the FreeBSD [`_umtx_op`](https://man.freebsd.org/cgi/man.cgi?query=_umtx_op&sektion=2&manpath=FreeBSD+14.2-RELEASE+and+Ports) syscall. :
    /// This is used for futex operations.
    ///
    /// `obj`: a pointer to the futex object (can be a lot of things, mostly *AtomicU32)
    /// `op`: the futex operation to run
    /// `val`: the current value of the object as a `c_long` (for wait/wake)
    /// `uaddr`: pointer to optional parameter (mostly timeouts)
    /// `uaddr2`: pointer to optional parameter (mostly timeouts)
    /// `dest`: the place this syscall returns to, 0 for success, -1 for failure
    fn _umtx_op(
        &mut self,
        obj: &OpTy<'tcx>,
        op: &OpTy<'tcx>,
        val: &OpTy<'tcx>,
        uaddr: &OpTy<'tcx>,
        uaddr2: &OpTy<'tcx>,
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();

        let obj = this.read_pointer(obj)?;
        let op = this.read_scalar(op)?.to_i32()?;
        let val = this.read_target_usize(val)?;
        let uaddr = this.read_scalar(uaddr)?;
        let uaddr2 = this.read_pointer(uaddr2)?;

        let wait = this.eval_libc_i32("UMTX_OP_WAIT");
        let wait_uint = this.eval_libc_i32("UMTX_OP_WAIT_UINT");
        let wait_uint_private = this.eval_libc_i32("UMTX_OP_WAIT_UINT_PRIVATE");

        let wake = this.eval_libc_i32("UMTX_OP_WAKE");
        let wake_private = this.eval_libc_i32("UMTX_OP_WAKE_PRIVATE");

        match op {
            // UMTX_OP_WAIT_UINT and UMTX_OP_WAIT_UINT_PRIVATE only differ in whether they work across
            // processes or not. For Miri, we can treat them the same.
            op if op == wait || op == wait_uint || op == wait_uint_private => {
                let obj_layout =
                    if op == wait { this.machine.layouts.isize } else { this.machine.layouts.u32 };
                let obj = this.ptr_to_mplace(obj, obj_layout);

                // Read the Linux futex implementation in Miri to understand why this fence is needed.
                this.atomic_fence(AtomicFenceOrd::SeqCst)?;
                let obj_val = this
                    .read_scalar_atomic(&obj, AtomicReadOrd::Acquire)?
                    .to_bits(obj_layout.size)?; // isize and u32 can have different sizes

                if obj_val == u128::from(val) {
                    let futex_ref = this
                        .get_sync_or_init(obj.ptr(), |_| FreeBsdFutex { futex: Default::default() })
                        .unwrap()
                        .futex
                        .clone();

                    // From the man page:
                    // If `uaddr2` is null than `uaddr` can point to an optional timespec parameter
                    // otherwise `uaddr2` must point to a `_umtx_time` parameter and the value of `uaddr`
                    // must be equal to the size of that struct.
                    let timeout = if this.ptr_is_null(uaddr2)? {
                        let uaddr_ptr = uaddr.to_pointer(this)?;
                        if this.ptr_is_null(uaddr_ptr)? {
                            // Both ptrs are null -> no timespec.
                            None
                        } else {
                            let timespec =
                                this.ptr_to_mplace(uaddr_ptr, this.libc_ty_layout("timespec"));
                            let duration = match this.read_timespec(&timespec)? {
                                Some(duration) => duration,
                                None => {
                                    return this
                                        .set_last_error_and_return(LibcError("EINVAL"), dest);
                                }
                            };

                            Some((TimeoutClock::Monotonic, TimeoutAnchor::Relative, duration))
                        }
                    } else {
                        let umtx_time_place =
                            this.ptr_to_mplace(uaddr2, this.libc_ty_layout("_umtx_time"));
                        let uaddr_as_size = uaddr.to_target_usize(this)?;

                        if umtx_time_place.layout().size.bytes() != uaddr_as_size {
                            return this.set_last_error_and_return(LibcError("EINVAL"), dest);
                        }

                        // In FreeBSD the `umtx_time` contains a `timespec` struct, which must be parsed.
                        // This can fail, which fails `read_umtx_time`, so we need to catch that.
                        let umtx_time = match read_umtx_time(this, &umtx_time_place)? {
                            Some(duration) => duration,
                            None => {
                                return this.set_last_error_and_return(LibcError("EINVAL"), dest);
                            }
                        };

                        let anchor = if umtx_time.abs_time {
                            TimeoutAnchor::Absolute
                        } else {
                            TimeoutAnchor::Relative
                        };

                        Some((umtx_time.timeout_clock, anchor, umtx_time.timeout))
                    };

                    let dest = dest.clone();
                    this.futex_wait(
                        futex_ref,
                        u32::MAX, // we set the bitset to include all bits
                        timeout,
                        callback!(
                            @capture<'tcx> {
                                dest: MPlaceTy<'tcx>,
                            }
                            |ecx, unblock: UnblockKind| match unblock {
                                UnblockKind::Ready => {
                                    // From the docs:
                                    // If successful, all requests, except UMTX_SHM_CREAT and  UMTX_SHM_LOOKUP
                                    // sub-requests  of	 the  UMTX_OP_SHM  request,  will  return  zero.
                                    ecx.write_int(0, &dest)
                                }
                                UnblockKind::TimedOut => {
                                    ecx.set_last_error_and_return(LibcError("ETIMEDOUT"), &dest)
                                }
                            }
                        ),
                    );
                    interp_ok(())
                } else {
                    // The manual doesn't document what should happen if `val` is invalid, so we error out.
                    this.set_last_error_and_return(LibcError("EINVAL"), dest)
                }
            }
            // UMTX_OP_WAKE and UMTX_OP_WAKE_PRIVATE only differ in whether they work across
            // processes or not. For Miri, we can treat them the same.
            op if op == wake || op == wake_private => {
                let Some(futex_ref) =
                    this.get_sync_or_init(obj, |_| FreeBsdFutex { futex: Default::default() })
                else {
                    // From Linux implemenation:
                    // No AllocId, or no live allocation at that AllocId.
                    // Return an error code. (That seems nicer than silently doing something non-intuitive.)
                    // This means that if an address gets reused by a new allocation,
                    // we'll use an independent futex queue for this... that seems acceptable.
                    return this.set_last_error_and_return(LibcError("EFAULT"), dest);
                };
                let futex_ref = futex_ref.futex.clone();

                let count = usize::try_from(val).unwrap_or(usize::MAX);

                // Read the Linux futex implementation in Miri to understand why this fence is needed.
                this.atomic_fence(AtomicFenceOrd::SeqCst)?;
                // `_umtx_op` doesn't return the amount of woken threads.
                let _woken = this.futex_wake(
                    &futex_ref,
                    u32::MAX, // we set the bitset to include all bits
                    count,
                )?;
                // From the docs:
                // If successful, all requests, except UMTX_SHM_CREAT and  UMTX_SHM_LOOKUP
                // sub-requests  of	 the  UMTX_OP_SHM  request,  will  return  zero.
                this.write_int(0, dest)?;
                interp_ok(())
            }
            op => {
                throw_unsup_format!("Miri does not support `_umtx_op` syscall with op={}", op)
            }
        }
    }
}

/// Parses a `_umtx_time` struct.
/// Returns `None` if the underlying `timespec` struct is invalid.
fn read_umtx_time<'tcx>(
    ecx: &mut MiriInterpCx<'tcx>,
    ut: &MPlaceTy<'tcx>,
) -> InterpResult<'tcx, Option<UmtxTime>> {
    let this = ecx.eval_context_mut();
    // Only flag allowed is UMTX_ABSTIME.
    let abs_time = this.eval_libc_u32("UMTX_ABSTIME");

    let timespec_place = this.project_field(ut, 0)?;
    // Inner `timespec` must still be valid.
    let duration = match this.read_timespec(&timespec_place)? {
        Some(dur) => dur,
        None => return interp_ok(None),
    };

    let flags_place = this.project_field(ut, 1)?;
    let flags = this.read_scalar(&flags_place)?.to_u32()?;
    let abs_time_flag = flags == abs_time;

    let clock_id_place = this.project_field(ut, 2)?;
    let clock_id = this.read_scalar(&clock_id_place)?.to_i32()?;
    let timeout_clock = umtx_time_translate_clock_id(this, clock_id)?;

    interp_ok(Some(UmtxTime { timeout: duration, abs_time: abs_time_flag, timeout_clock }))
}

fn umtx_time_translate_clock_id<'tcx>(
    ecx: &mut MiriInterpCx<'tcx>,
    id: i32,
) -> InterpResult<'tcx, TimeoutClock> {
    let timeout = if id == ecx.eval_libc_i32("CLOCK_REALTIME") {
        TimeoutClock::RealTime
    } else if id == ecx.eval_libc_i32("CLOCK_MONOTONIC") {
        TimeoutClock::Monotonic
    } else {
        throw_unsup_format!("unsupported clock id {id}");
    };
    interp_ok(timeout)
}
