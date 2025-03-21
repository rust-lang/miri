//! Contains FreeBSD-specific synchronization functions

use core::time::Duration;

use crate::concurrency::sync::FutexRef;
use crate::*;

pub struct FreeBSDFutex {
    futex: FutexRef,
}

pub struct UmtxTime {
    timeout: Duration,
    flags: u32,
    _clock_id: u32, // TODO: I'm not understanding why this is needed atm
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
        let uaddr2 = this.read_pointer(uaddr2)?;

        let wait = this.eval_libc_i32("UMTX_OP_WAIT");
        let wait_uint = this.eval_libc_i32("UMTX_OP_WAIT_UINT");
        let wait_uint_private = this.eval_libc_i32("UMTX_OP_WAIT_UINT_PRIVATE");

        let wake = this.eval_libc_i32("UMTX_OP_WAKE");
        let wake_private = this.eval_libc_i32("UMTX_OP_WAKE_PRIVATE");

        let absolute_time_flag = this.eval_libc_u32("UMTX_ABSTIME");

        match op {
            // UMTX_OP_WAIT_UINT has a private variant that enables an optimization that stops it from working across processes.
            // Miri doesn't support that anyway, so we ignore that variant and use the same implementation for all wait ops.
            op if op == wait || op == wait_uint || op == wait_uint_private => {
                // TODO: A better way to do this? Because I don't want to duplicate the actual logic.
                let equal = if op == wait {
                    // Read a long = isize.
                    let obj = this.ptr_to_mplace(obj, this.machine.layouts.isize);

                    // Read the Linux futex implementation in Miri to understand why this fence is needed.
                    this.atomic_fence(AtomicFenceOrd::SeqCst)?;
                    let obj_val = this
                        .read_scalar_atomic(&obj, AtomicReadOrd::Acquire)?
                        .to_target_isize(this)?;

                    // If obj points to something negative this can never hit, otherwise we can test.
                    if obj_val < 0 { false } else { val == u64::try_from(obj_val).unwrap() }
                } else {
                    // read a u_int = u32
                    let obj = this.ptr_to_mplace(obj, this.machine.layouts.u32);

                    // Read the Linux futex implementation in Miri to understand why this fence is needed.
                    this.atomic_fence(AtomicFenceOrd::SeqCst)?;
                    let obj_val =
                        this.read_scalar_atomic(&obj, AtomicReadOrd::Acquire)?.to_u32()?;
                    val == u64::from(obj_val)
                };

                if equal {
                    let futex_ref = this
                        .get_sync_or_init(obj, |_| FreeBSDFutex { futex: Default::default() })
                        .unwrap()
                        .futex
                        .clone();

                    // TODO: This can be cleaned up no? :)
                    // From the man page:
                    // If `uaddr2` is null than `uaddr` can point to an optional timespec parameter
                    // otherwise `uaddr2` must point to a `_umtx_time` parameter and the value of `uaddr`
                    // must be equal to the size of that struct.
                    let timeout = if this.ptr_is_null(uaddr2)? {
                        if this.ptr_is_null(this.read_pointer(uaddr)?)? {
                            // ptr is null -> no timespec
                            None
                        } else {
                            let timespec =
                                this.deref_pointer_as(uaddr, this.libc_ty_layout("timespec"))?;
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
                        let uaddr_as_size = this.read_target_usize(uaddr)?;

                        if umtx_time_place.layout().size.bytes() != uaddr_as_size {
                            return this.set_last_error_and_return(LibcError("EINVAL"), dest);
                        }

                        // Inner `timespec` must still be valid.
                        let umtx_time = match this.read_umtx_time(&umtx_time_place)? {
                            Some(duration) => duration,
                            None => {
                                return this.set_last_error_and_return(LibcError("EINVAL"), dest);
                            }
                        };

                        let anchor = if umtx_time.flags == absolute_time_flag {
                            TimeoutAnchor::Absolute
                        } else {
                            TimeoutAnchor::Relative
                        };

                        Some((TimeoutClock::Monotonic, anchor, umtx_time.timeout))
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
            // UMTX_OP_WAKE has a private variant that enables an optimization that stops it from working across processes.
            // Miri doesn't support that anyway, so we ignore that variant and use the same implementation for all wake ops.
            op if op == wake || op == wake_private => {
                let Some(futex_ref) =
                    this.get_sync_or_init(obj, |_| FreeBSDFutex { futex: Default::default() })
                else {
                    // From Linux implemenation:
                    // No AllocId, or no live allocation at that AllocId.
                    // Return an error code. (That seems nicer than silently doing something non-intuitive.)
                    // This means that if an address gets reused by a new allocation,
                    // we'll use an independent futex queue for this... that seems acceptable.
                    return this.set_last_error_and_return(LibcError("EFAULT"), dest);
                };
                let futex_ref = futex_ref.futex.clone();

                let count = match val{
                    u64::MAX => usize::MAX, // Preserve MAX because it specifies to wake everyone.
                    // This fits because we did `read_target_usize`.
                    val => usize::try_from(val).expect("`futex_wait` accepts `count` as usize, which can't seem to fit `val` of `_umtx_op`")
                };

                // Read the Linux futex implementation in Miri to understand why this fence is needed.
                this.atomic_fence(AtomicFenceOrd::SeqCst)?;
                // `_umtx_op` doesn't return the amount of woken threads.
                let _woken = this.futex_wake(
                    &futex_ref,
                    u32::MAX, // we set the bitset to include all bits
                    count,
                )?;
                this.write_int(0, dest)?;
                interp_ok(())
            }
            op => {
                throw_unsup_format!("Miri does not support `_umtx_op` syscall with op={}", op)
            }
        }
    }

    /// Parses a `_umtx_time` struct.
    /// Returns `None` if the underlying `timespec` struct is invalid.
    fn read_umtx_time(&mut self, ut: &MPlaceTy<'tcx>) -> InterpResult<'tcx, Option<UmtxTime>> {
        let this = self.eval_context_mut();
        let timespec_place = this.project_field(ut, 0)?;
        // Inner `timespec` must still be valid.
        let duration = match this.read_timespec(&timespec_place)? {
            Some(dur) => dur,
            None => return interp_ok(None),
        };
        let flags_place = this.project_field(ut, 1)?;
        let flags = this.read_scalar(&flags_place)?.to_u32()?;
        let clock_id_place = this.project_field(ut, 2)?;
        let clock_id = this.read_scalar(&clock_id_place)?.to_u32()?;
        interp_ok(Some(UmtxTime { timeout: duration, flags, _clock_id: clock_id }))
    }
}
