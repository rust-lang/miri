use crate::concurrency::sync::ParkResult;
use crate::*;

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn lwp_self(&mut self) -> InterpResult<'tcx, Scalar> {
        let this = self.eval_context_mut();

        let thread_id = this.active_thread();
        interp_ok(Scalar::from_u32(thread_id.to_u32()))
    }

    /// Implements [`_lwp_park`].
    ///
    /// [`_lwp_park`]: https://man.netbsd.org/_lwp_park.2
    fn lwp_park(
        &mut self,
        clock_id: &OpTy<'tcx>,
        flags: &OpTy<'tcx>,
        ts: &OpTy<'tcx>,
        unpark: &OpTy<'tcx>,
        hint: &OpTy<'tcx>,
        unparkhint: &OpTy<'tcx>,
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();

        let clock_id = this.read_scalar(clock_id)?.to_i32()?;
        let flags = this.read_scalar(flags)?.to_i32()?;
        let ts = this.read_pointer(ts)?;
        let _hint = this.read_pointer(hint)?;
        let _unparkhint = this.read_pointer(unparkhint)?;

        if this.read_scalar(unpark)?.to_u32()? != 0 {
            if !this.lwp_unpark(unpark, unparkhint, dest)? {
                return interp_ok(());
            }
        }

        let (timeout, write_remaining) = if this.ptr_is_null(ts)? {
            (None, None)
        } else {
            if clock_id != this.eval_libc_i32("CLOCK_MONOTONIC") {
                throw_unsup_format!("lwp_park: only CLOCK_MONOTONIC is currently supported");
            }
            let clock = TimeoutClock::Monotonic;

            let ts = this.ptr_to_mplace(ts, this.libc_ty_layout("timespec"));
            let Some(duration) = this.read_timespec(&ts)? else {
                this.set_errno_and_return_neg1_i32(LibcError("EINVAL"))?;
                return interp_ok(());
            };

            let (style, write_remaining) = if flags == 0 {
                // No flags set, the timespec should be interpreted as a duration
                // to sleep for, i.e., a relative time.
                let deadline = this.machine.monotonic_clock.now().add_lossy(duration);
                (TimeoutStyle::Relative, Some((deadline, ts)))
            } else if flags == this.eval_libc_i32("TIMER_ABSTIME") {
                // Only flag TIMER_ABSTIME set, the timespec should be interpreted as
                // an absolute time.
                (TimeoutStyle::Absolute, None)
            } else {
                throw_unsup_format!(
                    "`lwp_park` unsupported flags {flags}, only no flags or \
                    TIMER_ABSTIME is supported"
                )
            };

            (Some(this.machine.timeout(clock, style, duration)), write_remaining)
        };

        let res = {
            let dest = dest.clone();
            this.thread_park(
                timeout,
                callback!(
                    @capture<'tcx> {
                        write_remaining: Option<(Instant, MPlaceTy<'tcx>)>,
                        ts: Pointer,
                        dest: MPlaceTy<'tcx>,
                    }
                    |this, unblock: UnblockKind| {
                        if let Some((deadline, return_ts)) = write_remaining {
                            let remaining = deadline.duration_since(this.machine.monotonic_clock.now());
                            this.write_timespec(remaining, &return_ts)?;
                        }

                        match unblock {
                            UnblockKind::Ready => {
                                this.write_scalar(Scalar::from_i32(0), &dest)
                            }
                            UnblockKind::TimedOut => {
                                this.set_errno_and_return_neg1(LibcError("ETIMEDOUT"), &dest)
                            }
                        }
                    }
                ),
            )?
        };

        match res {
            ParkResult::Already => this.set_errno_and_return_neg1(LibcError("EALREADY"), dest)?,
            ParkResult::Parked => {
                // This thread is blocked. `dest` will be filled by the callback
                // invoked when it is unblocked.
            }
        }

        interp_ok(())
    }

    /// Implements [`_lwp_unpark`].
    ///
    /// [`_lwp_unpark`]: https://man.netbsd.org/_lwp_unpark.2
    fn lwp_unpark(
        &mut self,
        lwp: &OpTy<'tcx>,
        hint: &OpTy<'tcx>,
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx, bool> {
        let this = self.eval_context_mut();

        let lwp = this.read_scalar(lwp)?.to_u32()?;
        let _hint = this.read_pointer(hint)?;

        let Ok(thread) = this.machine.threads.thread_id_try_from(lwp) else {
            this.set_errno_and_return_neg1(LibcError("ESRCH"), dest)?;
            return interp_ok(false);
        };

        // FIXME: this is a bit imprecise – `_lwp_park` is also used by NetBSD's
        //        pthread implementation, which will consume the thread token
        //        and then continue waiting.
        this.thread_unpark(thread)?;
        this.write_scalar(Scalar::from_i32(0), dest)?;
        interp_ok(true)
    }
}
