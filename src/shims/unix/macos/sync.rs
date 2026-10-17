//! Contains macOS-specific synchronization functions.

use std::cell::Cell;
use std::time::Duration;

use rustc_abi::Size;

use crate::concurrency::sync::{FutexRef, SyncObj};
use crate::*;

pub enum MacOsFutexTimeout<'a, 'tcx> {
    None,
    Relative { clock_op: &'a OpTy<'tcx>, timeout_op: &'a OpTy<'tcx> },
    Absolute { clock_op: &'a OpTy<'tcx>, timeout_op: &'a OpTy<'tcx> },
}

/// Metadata for a macOS futex.
///
/// Since macOS 11.0, Apple has exposed the previously private futex API consisting
/// of `os_sync_wait_on_address` (and friends) and `os_sync_wake_by_address_{any, all}`.
/// These work with different value sizes and flags, which are validated to be consistent.
/// This structure keeps track of both the futex queue and these values.
struct MacOsFutex {
    futex: FutexRef,
    /// The size in bytes of the atomic primitive underlying this futex.
    size: Cell<u64>,
    /// Whether the futex is shared across process boundaries.
    shared: Cell<bool>,
}

impl SyncObj for MacOsFutex {}

impl<'tcx> EvalContextExt<'tcx> for crate::MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: crate::MiriInterpCxExt<'tcx> {
    /// Implements [`os_sync_wait_on_address`], [`os_sync_wait_on_address_with_deadline`]
    /// and [`os_sync_wait_on_address_with_timeout`].
    ///
    /// [`os_sync_wait_on_address`]: https://developer.apple.com/documentation/os/os_sync_wait_on_address?language=objc
    /// [`os_sync_wait_on_address_with_deadline`]: https://developer.apple.com/documentation/os/os_sync_wait_on_address_with_deadline?language=objc
    /// [`os_sync_wait_on_address_with_timeout`]: https://developer.apple.com/documentation/os/os_sync_wait_on_address_with_timeout?language=objc
    fn os_sync_wait_on_address(
        &mut self,
        addr_op: &OpTy<'tcx>,
        value_op: &OpTy<'tcx>,
        size_op: &OpTy<'tcx>,
        flags_op: &OpTy<'tcx>,
        timeout: MacOsFutexTimeout<'_, 'tcx>,
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let none = this.eval_libc_u32("OS_SYNC_WAIT_ON_ADDRESS_NONE");
        let shared = this.eval_libc_u32("OS_SYNC_WAIT_ON_ADDRESS_SHARED");
        let absolute_clock = this.eval_libc_u32("OS_CLOCK_MACH_ABSOLUTE_TIME");

        let ptr = this.read_pointer(addr_op)?;
        let value = this.read_scalar(value_op)?.to_u64()?;
        let size = this.read_target_usize(size_op)?;
        let flags = this.read_scalar(flags_op)?.to_u32()?;

        let clock_timeout = match timeout {
            MacOsFutexTimeout::None => None,
            MacOsFutexTimeout::Relative { clock_op, timeout_op } => {
                let clock = this.read_scalar(clock_op)?.to_u32()?;
                let timeout = this.read_scalar(timeout_op)?.to_u64()?;
                Some((clock, TimeoutStyle::Relative, timeout))
            }
            MacOsFutexTimeout::Absolute { clock_op, timeout_op } => {
                let clock = this.read_scalar(clock_op)?.to_u32()?;
                let timeout = this.read_scalar(timeout_op)?.to_u64()?;
                Some((clock, TimeoutStyle::Absolute, timeout))
            }
        };

        // Perform validation of the arguments.
        let addr = ptr.addr().bytes();
        if addr == 0
            || !matches!(size, 4 | 8)
            || !addr.is_multiple_of(size)
            || (flags != none && flags != shared)
            || clock_timeout
                .is_some_and(|(clock, _, timeout)| clock != absolute_clock || timeout == 0)
        {
            this.set_errno_and_return_neg1(LibcError("EINVAL"), dest)?;
            return interp_ok(());
        }

        let is_shared = flags == shared;
        let deadline = clock_timeout.map(|(_, style, timeout)| {
            // The only clock that is currently supported is the monotonic clock (checked above).
            // While the deadline argument of `os_sync_wait_on_address_with_deadline`
            // is actually not in nanoseconds but in the units of `mach_current_time`,
            // the two are equivalent in miri.
            this.machine.timeout(TimeoutClock::Monotonic, style, Duration::from_nanos(timeout))
        });

        // See the Linux futex implementation for why this fence exists.
        this.atomic_fence(AtomicFenceOrd::SeqCst)?;

        let layout = this.machine.layouts.uint(Size::from_bytes(size)).unwrap();
        let futex_val = this
            .read_scalar_atomic(&this.ptr_to_mplace(ptr, layout), AtomicReadOrd::Acquire)?
            .to_bits(Size::from_bytes(size))?;

        let futex = this
            .get_sync_or_init(ptr, |_| {
                MacOsFutex {
                    futex: Default::default(),
                    size: Cell::new(size),
                    shared: Cell::new(is_shared),
                }
            })
            .unwrap();

        // Detect mismatches between the flags and sizes used on this address
        // by comparing it with the parameters used by the other waiters in
        // the current list. If the list is currently empty, update those
        // parameters.
        if futex.futex.waiters() == 0 {
            futex.size.set(size);
            futex.shared.set(is_shared);
        } else if futex.size.get() != size || futex.shared.get() != is_shared {
            this.set_errno_and_return_neg1(LibcError("EINVAL"), dest)?;
            return interp_ok(());
        }

        if futex_val == value.into() {
            // If the values are the same, we have to block.
            let futex_ref = futex.futex.clone();
            let dest = dest.clone();
            this.futex_wait(
                futex_ref.clone(),
                u32::MAX, // bitset
                deadline,
                callback!(
                    @capture<'tcx> {
                        dest: MPlaceTy<'tcx>,
                        futex_ref: FutexRef,
                    }
                    |this, unblock: UnblockKind| {
                        match unblock {
                            UnblockKind::Ready => {
                                let remaining = futex_ref.waiters().try_into().unwrap();
                                this.write_scalar(Scalar::from_i32(remaining), &dest)
                            }
                            UnblockKind::TimedOut => {
                                this.set_errno_and_return_neg1(LibcError("ETIMEDOUT"), &dest)
                            }
                        }
                    }
                ),
            );
        } else {
            // else retrieve the current number of waiters.
            let waiters = futex.futex.waiters().try_into().unwrap();
            this.write_scalar(Scalar::from_i32(waiters), dest)?;
        }

        interp_ok(())
    }

    /// Implements [`os_sync_wake_by_address_all`] and [`os_sync_wake_by_address_any`].
    ///
    /// [`os_sync_wake_by_address_all`]: https://developer.apple.com/documentation/os/os_sync_wake_by_address_all?language=objc
    /// [`os_sync_wake_by_address_any`]: https://developer.apple.com/documentation/os/os_sync_wake_by_address_any?language=objc
    fn os_sync_wake_by_address(
        &mut self,
        addr_op: &OpTy<'tcx>,
        size_op: &OpTy<'tcx>,
        flags_op: &OpTy<'tcx>,
        all: bool,
        dest: &MPlaceTy<'tcx>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let none = this.eval_libc_u32("OS_SYNC_WAKE_BY_ADDRESS_NONE");
        let shared = this.eval_libc_u32("OS_SYNC_WAKE_BY_ADDRESS_SHARED");

        let ptr = this.read_pointer(addr_op)?;
        let size = this.read_target_usize(size_op)?;
        let flags = this.read_scalar(flags_op)?.to_u32()?;

        // Perform validation of the arguments.
        let addr = ptr.addr().bytes();
        if addr == 0 || !matches!(size, 4 | 8) || (flags != none && flags != shared) {
            this.set_errno_and_return_neg1(LibcError("EINVAL"), dest)?;
            return interp_ok(());
        }

        let is_shared = flags == shared;

        let Some(futex) = this.get_sync_or_init(ptr, |_| {
            MacOsFutex {
                futex: Default::default(),
                size: Cell::new(size),
                shared: Cell::new(is_shared),
            }
        }) else {
            // No AllocId, or no live allocation at that AllocId. Return an
            // error code. (That seems nicer than silently doing something
            // non-intuitive.) This means that if an address gets reused by a
            // new allocation, we'll use an independent futex queue for this...
            // that seems acceptable.
            this.set_errno_and_return_neg1(LibcError("ENOENT"), dest)?;
            return interp_ok(());
        };

        if futex.futex.waiters() == 0 {
            this.set_errno_and_return_neg1(LibcError("ENOENT"), dest)?;
            return interp_ok(());
        // If there are waiters in the queue, they have all used the parameters
        // stored in `futex` (we check this in `os_sync_wait_on_address` above).
        // Detect mismatches between "our" parameters and the parameters used by
        // the waiters and return an error in that case.
        } else if futex.size.get() != size || futex.shared.get() != is_shared {
            this.set_errno_and_return_neg1(LibcError("EINVAL"), dest)?;
            return interp_ok(());
        }

        let futex_ref = futex.futex.clone();

        // See the Linux futex implementation for why this fence exists.
        this.atomic_fence(AtomicFenceOrd::SeqCst)?;
        this.futex_wake(&futex_ref, u32::MAX, if all { usize::MAX } else { 1 })?;
        this.write_scalar(Scalar::from_i32(0), dest)?;
        interp_ok(())
    }
}
