use std::time::SystemTime;

use crate::*;
use stacked_borrows::Tag;
use thread::Time;

// pthread_mutexattr_t is either 4 or 8 bytes, depending on the platform.

// Our chosen memory layout for emulation (does not have to match the platform layout!):
// store an i32 in the first four bytes equal to the corresponding libc mutex kind constant
// (e.g. PTHREAD_MUTEX_NORMAL).

/// A flag that allows to distinguish `PTHREAD_MUTEX_NORMAL` from
/// `PTHREAD_MUTEX_DEFAULT`. Since in `glibc` they have the same numeric values,
/// but different behaviour, we need a way to distinguish them. We do this by
/// setting this bit flag to the `PTHREAD_MUTEX_NORMAL` mutexes. See the comment
/// in `pthread_mutexattr_settype` function.
const PTHREAD_MUTEX_NORMAL_FLAG: i32 = 0x8000000;

fn is_mutex_kind_default<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    kind: Scalar<Tag>,
) -> InterpResult<'tcx, bool> {
    Ok(kind == ecx.eval_libc("PTHREAD_MUTEX_DEFAULT")?)
}

fn is_mutex_kind_normal<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    kind: Scalar<Tag>,
) -> InterpResult<'tcx, bool> {
    let kind = kind.to_i32()?;
    let mutex_normal_kind = ecx.eval_libc("PTHREAD_MUTEX_NORMAL")?.to_i32()?;
    Ok(kind == (mutex_normal_kind | PTHREAD_MUTEX_NORMAL_FLAG))
}

fn mutexattr_get_kind<'mir, 'tcx: 'mir>(
    ecx: &MiriEvalContext<'mir, 'tcx>,
    attr_op: &OpTy<'tcx, Tag>,
) -> InterpResult<'tcx, ScalarMaybeUninit<Tag>> {
    ecx.read_scalar_at_offset(attr_op, 0, ecx.machine.layouts.i32)
}

fn mutexattr_set_kind<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    attr_op: &OpTy<'tcx, Tag>,
    kind: impl Into<ScalarMaybeUninit<Tag>>,
) -> InterpResult<'tcx, ()> {
    ecx.write_scalar_at_offset(attr_op, 0, kind, ecx.machine.layouts.i32)
}

// pthread_mutex_t is between 24 and 48 bytes, depending on the platform.

// Our chosen memory layout for the emulated mutex (does not have to match the platform layout!):
// bytes 0-3: reserved for signature on macOS
// (need to avoid this because it is set by static initializer macros)
// bytes 4-7: mutex id as u32 or 0 if id is not assigned yet.
// bytes 12-15 or 16-19 (depending on platform): mutex kind, as an i32
// (the kind has to be at its offset for compatibility with static initializer macros)

fn mutex_get_kind<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    mutex_op: &OpTy<'tcx, Tag>,
) -> InterpResult<'tcx, ScalarMaybeUninit<Tag>> {
    let offset = if ecx.pointer_size().bytes() == 8 { 16 } else { 12 };
    ecx.read_scalar_at_offset_atomic(
        mutex_op,
        offset,
        ecx.machine.layouts.i32,
        AtomicReadOp::Relaxed,
    )
}

fn mutex_set_kind<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    mutex_op: &OpTy<'tcx, Tag>,
    kind: impl Into<ScalarMaybeUninit<Tag>>,
) -> InterpResult<'tcx, ()> {
    let offset = if ecx.pointer_size().bytes() == 8 { 16 } else { 12 };
    ecx.write_scalar_at_offset_atomic(
        mutex_op,
        offset,
        kind,
        ecx.machine.layouts.i32,
        AtomicWriteOp::Relaxed,
    )
}

fn mutex_get_id<'mir, 'tcx: 'mir>(
    ecx: &MiriEvalContext<'mir, 'tcx>,
    mutex_op: &OpTy<'tcx, Tag>,
) -> InterpResult<'tcx, ScalarMaybeUninit<Tag>> {
    ecx.read_scalar_at_offset_atomic(mutex_op, 4, ecx.machine.layouts.u32, AtomicReadOp::Relaxed)
}

fn mutex_set_id<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    mutex_op: &OpTy<'tcx, Tag>,
    id: impl Into<ScalarMaybeUninit<Tag>>,
) -> InterpResult<'tcx, ()> {
    ecx.write_scalar_at_offset_atomic(
        mutex_op,
        4,
        id,
        ecx.machine.layouts.u32,
        AtomicWriteOp::Relaxed,
    )
}

fn mutex_get_or_create_id<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    mutex_op: &OpTy<'tcx, Tag>,
) -> InterpResult<'tcx, MutexId> {
    let id = mutex_get_id(ecx, mutex_op)?.to_u32()?;
    if id == 0 {
        // 0 is a default value and also not a valid mutex id. Need to allocate
        // a new mutex.
        let id = ecx.mutex_create();
        mutex_set_id(ecx, mutex_op, id.to_u32_scalar())?;
        Ok(id)
    } else {
        Ok(MutexId::from_u32(id))
    }
}

// pthread_rwlock_t is between 32 and 56 bytes, depending on the platform.

// Our chosen memory layout for the emulated rwlock (does not have to match the platform layout!):
// bytes 0-3: reserved for signature on macOS
// (need to avoid this because it is set by static initializer macros)
// bytes 4-7: rwlock id as u32 or 0 if id is not assigned yet.

fn rwlock_get_id<'mir, 'tcx: 'mir>(
    ecx: &MiriEvalContext<'mir, 'tcx>,
    rwlock_op: &OpTy<'tcx, Tag>,
) -> InterpResult<'tcx, ScalarMaybeUninit<Tag>> {
    ecx.read_scalar_at_offset_atomic(rwlock_op, 4, ecx.machine.layouts.u32, AtomicReadOp::Relaxed)
}

fn rwlock_set_id<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    rwlock_op: &OpTy<'tcx, Tag>,
    id: impl Into<ScalarMaybeUninit<Tag>>,
) -> InterpResult<'tcx, ()> {
    ecx.write_scalar_at_offset_atomic(
        rwlock_op,
        4,
        id,
        ecx.machine.layouts.u32,
        AtomicWriteOp::Relaxed,
    )
}

fn rwlock_get_or_create_id<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    rwlock_op: &OpTy<'tcx, Tag>,
) -> InterpResult<'tcx, RwLockId> {
    let id = rwlock_get_id(ecx, rwlock_op)?.to_u32()?;
    if id == 0 {
        // 0 is a default value and also not a valid rwlock id. Need to allocate
        // a new read-write lock.
        let id = ecx.rwlock_create();
        rwlock_set_id(ecx, rwlock_op, id.to_u32_scalar())?;
        Ok(id)
    } else {
        Ok(RwLockId::from_u32(id))
    }
}

// pthread_condattr_t

// Our chosen memory layout for emulation (does not have to match the platform layout!):
// store an i32 in the first four bytes equal to the corresponding libc clock id constant
// (e.g. CLOCK_REALTIME).

fn condattr_get_clock_id<'mir, 'tcx: 'mir>(
    ecx: &MiriEvalContext<'mir, 'tcx>,
    attr_op: &OpTy<'tcx, Tag>,
) -> InterpResult<'tcx, ScalarMaybeUninit<Tag>> {
    ecx.read_scalar_at_offset(attr_op, 0, ecx.machine.layouts.i32)
}

fn condattr_set_clock_id<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    attr_op: &OpTy<'tcx, Tag>,
    clock_id: impl Into<ScalarMaybeUninit<Tag>>,
) -> InterpResult<'tcx, ()> {
    ecx.write_scalar_at_offset(attr_op, 0, clock_id, ecx.machine.layouts.i32)
}

// pthread_cond_t

// Our chosen memory layout for the emulated conditional variable (does not have
// to match the platform layout!):

// bytes 0-3: reserved for signature on macOS
// bytes 4-7: the conditional variable id as u32 or 0 if id is not assigned yet.
// bytes 8-11: the clock id constant as i32

fn cond_get_id<'mir, 'tcx: 'mir>(
    ecx: &MiriEvalContext<'mir, 'tcx>,
    cond_op: &OpTy<'tcx, Tag>,
) -> InterpResult<'tcx, ScalarMaybeUninit<Tag>> {
    ecx.read_scalar_at_offset_atomic(cond_op, 4, ecx.machine.layouts.u32, AtomicReadOp::Relaxed)
}

fn cond_set_id<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    cond_op: &OpTy<'tcx, Tag>,
    id: impl Into<ScalarMaybeUninit<Tag>>,
) -> InterpResult<'tcx, ()> {
    ecx.write_scalar_at_offset_atomic(
        cond_op,
        4,
        id,
        ecx.machine.layouts.u32,
        AtomicWriteOp::Relaxed,
    )
}

fn cond_get_or_create_id<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    cond_op: &OpTy<'tcx, Tag>,
) -> InterpResult<'tcx, CondvarId> {
    let id = cond_get_id(ecx, cond_op)?.to_u32()?;
    if id == 0 {
        // 0 is a default value and also not a valid conditional variable id.
        // Need to allocate a new id.
        let id = ecx.condvar_create();
        cond_set_id(ecx, cond_op, id.to_u32_scalar())?;
        Ok(id)
    } else {
        Ok(CondvarId::from_u32(id))
    }
}

fn cond_get_clock_id<'mir, 'tcx: 'mir>(
    ecx: &MiriEvalContext<'mir, 'tcx>,
    cond_op: &OpTy<'tcx, Tag>,
) -> InterpResult<'tcx, ScalarMaybeUninit<Tag>> {
    ecx.read_scalar_at_offset(cond_op, 8, ecx.machine.layouts.i32)
}

fn cond_set_clock_id<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    cond_op: &OpTy<'tcx, Tag>,
    clock_id: impl Into<ScalarMaybeUninit<Tag>>,
) -> InterpResult<'tcx, ()> {
    ecx.write_scalar_at_offset(cond_op, 8, clock_id, ecx.machine.layouts.i32)
}

/// Try to reacquire the mutex associated with the condition variable after we
/// were signaled.
fn reacquire_cond_mutex<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    thread: ThreadId,
    mutex: MutexId,
) -> InterpResult<'tcx> {
    ecx.unblock_thread(thread);
    if ecx.mutex_is_locked(mutex) {
        ecx.mutex_enqueue_and_block(mutex, thread);
    } else {
        ecx.mutex_lock(mutex, thread);
    }
    Ok(())
}

/// After a thread waiting on a condvar was signalled:
/// Reacquire the conditional variable and remove the timeout callback if any
/// was registered.
fn post_cond_signal<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    thread: ThreadId,
    mutex: MutexId,
) -> InterpResult<'tcx> {
    reacquire_cond_mutex(ecx, thread, mutex)?;
    // Waiting for the mutex is not included in the waiting time because we need
    // to acquire the mutex always even if we get a timeout.
    ecx.unregister_timeout_callback_if_exists(thread);
    Ok(())
}

/// Release the mutex associated with the condition variable because we are
/// entering the waiting state.
fn release_cond_mutex_and_block<'mir, 'tcx: 'mir>(
    ecx: &mut MiriEvalContext<'mir, 'tcx>,
    active_thread: ThreadId,
    mutex: MutexId,
) -> InterpResult<'tcx> {
    if let Some(old_locked_count) = ecx.mutex_unlock(mutex, active_thread) {
        if old_locked_count != 1 {
            throw_unsup_format!("awaiting on a lock acquired multiple times is not supported");
        }
    } else {
        throw_ub_format!("awaiting on unlocked or owned by a different thread mutex");
    }
    ecx.block_thread(active_thread);
    Ok(())
}

impl<'mir, 'tcx> EvalContextExt<'mir, 'tcx> for crate::MiriEvalContext<'mir, 'tcx> {}
pub trait EvalContextExt<'mir, 'tcx: 'mir>: crate::MiriEvalContextExt<'mir, 'tcx> {
    fn pthread_mutexattr_init(&mut self, attr_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let default_kind = this.eval_libc("PTHREAD_MUTEX_DEFAULT")?;
        mutexattr_set_kind(this, attr_op, default_kind)?;

        Ok(0)
    }

    fn pthread_mutexattr_settype(
        &mut self,
        attr_op: &OpTy<'tcx, Tag>,
        kind_op: &OpTy<'tcx, Tag>,
    ) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let kind = this.read_scalar(kind_op)?.check_init()?;
        if kind == this.eval_libc("PTHREAD_MUTEX_NORMAL")? {
            // In `glibc` implementation, the numeric values of
            // `PTHREAD_MUTEX_NORMAL` and `PTHREAD_MUTEX_DEFAULT` are equal.
            // However, a mutex created by explicitly passing
            // `PTHREAD_MUTEX_NORMAL` type has in some cases different behaviour
            // from the default mutex for which the type was not explicitly
            // specified. For a more detailed discussion, please see
            // https://github.com/rust-lang/miri/issues/1419.
            //
            // To distinguish these two cases in already constructed mutexes, we
            // use the same trick as glibc: for the case when
            // `pthread_mutexattr_settype` is caled explicitly, we set the
            // `PTHREAD_MUTEX_NORMAL_FLAG` flag.
            let normal_kind = kind.to_i32()? | PTHREAD_MUTEX_NORMAL_FLAG;
            // Check that after setting the flag, the kind is distinguishable
            // from all other kinds.
            assert_ne!(normal_kind, this.eval_libc("PTHREAD_MUTEX_DEFAULT")?.to_i32()?);
            assert_ne!(normal_kind, this.eval_libc("PTHREAD_MUTEX_ERRORCHECK")?.to_i32()?);
            assert_ne!(normal_kind, this.eval_libc("PTHREAD_MUTEX_RECURSIVE")?.to_i32()?);
            mutexattr_set_kind(this, attr_op, Scalar::from_i32(normal_kind))?;
        } else if kind == this.eval_libc("PTHREAD_MUTEX_DEFAULT")?
            || kind == this.eval_libc("PTHREAD_MUTEX_ERRORCHECK")?
            || kind == this.eval_libc("PTHREAD_MUTEX_RECURSIVE")?
        {
            mutexattr_set_kind(this, attr_op, kind)?;
        } else {
            let einval = this.eval_libc_i32("EINVAL")?;
            return Ok(einval);
        }

        Ok(0)
    }

    fn pthread_mutexattr_destroy(&mut self, attr_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        mutexattr_set_kind(this, attr_op, ScalarMaybeUninit::Uninit)?;

        Ok(0)
    }

    fn pthread_mutex_init(
        &mut self,
        mutex_op: &OpTy<'tcx, Tag>,
        attr_op: &OpTy<'tcx, Tag>,
    ) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let attr = this.read_scalar(attr_op)?.check_init()?;
        let kind = if this.is_null(attr)? {
            this.eval_libc("PTHREAD_MUTEX_DEFAULT")?
        } else {
            mutexattr_get_kind(this, attr_op)?.check_init()?
        };

        // Write 0 to use the same code path as the static initializers.
        mutex_set_id(this, mutex_op, Scalar::from_i32(0))?;

        mutex_set_kind(this, mutex_op, kind)?;

        Ok(0)
    }

    fn pthread_mutex_lock(&mut self, mutex_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let kind = mutex_get_kind(this, mutex_op)?.check_init()?;
        let id = mutex_get_or_create_id(this, mutex_op)?;
        let active_thread = this.get_active_thread();

        if this.mutex_is_locked(id) {
            let owner_thread = this.mutex_get_owner(id);
            if owner_thread != active_thread {
                // Enqueue the active thread.
                this.mutex_enqueue_and_block(id, active_thread);
                Ok(0)
            } else {
                // Trying to acquire the same mutex again.
                if is_mutex_kind_default(this, kind)? {
                    throw_ub_format!("trying to acquire already locked default mutex");
                } else if is_mutex_kind_normal(this, kind)? {
                    throw_machine_stop!(TerminationInfo::Deadlock);
                } else if kind == this.eval_libc("PTHREAD_MUTEX_ERRORCHECK")? {
                    this.eval_libc_i32("EDEADLK")
                } else if kind == this.eval_libc("PTHREAD_MUTEX_RECURSIVE")? {
                    this.mutex_lock(id, active_thread);
                    Ok(0)
                } else {
                    throw_unsup_format!(
                        "called pthread_mutex_lock on an unsupported type of mutex"
                    );
                }
            }
        } else {
            // The mutex is unlocked. Let's lock it.
            this.mutex_lock(id, active_thread);
            Ok(0)
        }
    }

    fn pthread_mutex_trylock(&mut self, mutex_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let kind = mutex_get_kind(this, mutex_op)?.check_init()?;
        let id = mutex_get_or_create_id(this, mutex_op)?;
        let active_thread = this.get_active_thread();

        if this.mutex_is_locked(id) {
            let owner_thread = this.mutex_get_owner(id);
            if owner_thread != active_thread {
                this.eval_libc_i32("EBUSY")
            } else {
                if is_mutex_kind_default(this, kind)?
                    || is_mutex_kind_normal(this, kind)?
                    || kind == this.eval_libc("PTHREAD_MUTEX_ERRORCHECK")?
                {
                    this.eval_libc_i32("EBUSY")
                } else if kind == this.eval_libc("PTHREAD_MUTEX_RECURSIVE")? {
                    this.mutex_lock(id, active_thread);
                    Ok(0)
                } else {
                    throw_unsup_format!(
                        "called pthread_mutex_trylock on an unsupported type of mutex"
                    );
                }
            }
        } else {
            // The mutex is unlocked. Let's lock it.
            this.mutex_lock(id, active_thread);
            Ok(0)
        }
    }

    fn pthread_mutex_unlock(&mut self, mutex_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let kind = mutex_get_kind(this, mutex_op)?.check_init()?;
        let id = mutex_get_or_create_id(this, mutex_op)?;
        let active_thread = this.get_active_thread();

        if let Some(_old_locked_count) = this.mutex_unlock(id, active_thread) {
            // The mutex was locked by the current thread.
            Ok(0)
        } else {
            // The mutex was locked by another thread or not locked at all. See
            // the “Unlock When Not Owner” column in
            // https://pubs.opengroup.org/onlinepubs/9699919799/functions/pthread_mutex_unlock.html.
            if is_mutex_kind_default(this, kind)? {
                throw_ub_format!(
                    "unlocked a default mutex that was not locked by the current thread"
                );
            } else if is_mutex_kind_normal(this, kind)? {
                throw_ub_format!(
                    "unlocked a PTHREAD_MUTEX_NORMAL mutex that was not locked by the current thread"
                );
            } else if kind == this.eval_libc("PTHREAD_MUTEX_ERRORCHECK")? {
                this.eval_libc_i32("EPERM")
            } else if kind == this.eval_libc("PTHREAD_MUTEX_RECURSIVE")? {
                this.eval_libc_i32("EPERM")
            } else {
                throw_unsup_format!("called pthread_mutex_unlock on an unsupported type of mutex");
            }
        }
    }

    fn pthread_mutex_destroy(&mut self, mutex_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let id = mutex_get_or_create_id(this, mutex_op)?;

        if this.mutex_is_locked(id) {
            throw_ub_format!("destroyed a locked mutex");
        }

        mutex_set_kind(this, mutex_op, ScalarMaybeUninit::Uninit)?;
        mutex_set_id(this, mutex_op, ScalarMaybeUninit::Uninit)?;
        // FIXME: delete interpreter state associated with this mutex.

        Ok(0)
    }

    fn pthread_rwlock_rdlock(&mut self, rwlock_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let id = rwlock_get_or_create_id(this, rwlock_op)?;
        let active_thread = this.get_active_thread();

        if this.rwlock_is_write_locked(id) {
            this.rwlock_enqueue_and_block_reader(id, active_thread);
            Ok(0)
        } else {
            this.rwlock_reader_lock(id, active_thread);
            Ok(0)
        }
    }

    fn pthread_rwlock_tryrdlock(&mut self, rwlock_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let id = rwlock_get_or_create_id(this, rwlock_op)?;
        let active_thread = this.get_active_thread();

        if this.rwlock_is_write_locked(id) {
            this.eval_libc_i32("EBUSY")
        } else {
            this.rwlock_reader_lock(id, active_thread);
            Ok(0)
        }
    }

    fn pthread_rwlock_wrlock(&mut self, rwlock_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let id = rwlock_get_or_create_id(this, rwlock_op)?;
        let active_thread = this.get_active_thread();

        if this.rwlock_is_locked(id) {
            // Note: this will deadlock if the lock is already locked by this
            // thread in any way.
            //
            // Relevant documentation:
            // https://pubs.opengroup.org/onlinepubs/9699919799/functions/pthread_rwlock_wrlock.html
            // An in-depth discussion on this topic:
            // https://github.com/rust-lang/rust/issues/53127
            //
            // FIXME: Detect and report the deadlock proactively. (We currently
            // report the deadlock only when no thread can continue execution,
            // but we could detect that this lock is already locked and report
            // an error.)
            this.rwlock_enqueue_and_block_writer(id, active_thread);
        } else {
            this.rwlock_writer_lock(id, active_thread);
        }

        Ok(0)
    }

    fn pthread_rwlock_trywrlock(&mut self, rwlock_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let id = rwlock_get_or_create_id(this, rwlock_op)?;
        let active_thread = this.get_active_thread();

        if this.rwlock_is_locked(id) {
            this.eval_libc_i32("EBUSY")
        } else {
            this.rwlock_writer_lock(id, active_thread);
            Ok(0)
        }
    }

    fn pthread_rwlock_unlock(&mut self, rwlock_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let id = rwlock_get_or_create_id(this, rwlock_op)?;
        let active_thread = this.get_active_thread();

        if this.rwlock_reader_unlock(id, active_thread) {
            Ok(0)
        } else if this.rwlock_writer_unlock(id, active_thread) {
            Ok(0)
        } else {
            throw_ub_format!("unlocked an rwlock that was not locked by the active thread");
        }
    }

    fn pthread_rwlock_destroy(&mut self, rwlock_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let id = rwlock_get_or_create_id(this, rwlock_op)?;

        if this.rwlock_is_locked(id) {
            throw_ub_format!("destroyed a locked rwlock");
        }

        rwlock_set_id(this, rwlock_op, ScalarMaybeUninit::Uninit)?;
        // FIXME: delete interpreter state associated with this rwlock.

        Ok(0)
    }

    fn pthread_condattr_init(&mut self, attr_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        // The default value of the clock attribute shall refer to the system
        // clock.
        // https://pubs.opengroup.org/onlinepubs/9699919799/functions/pthread_condattr_setclock.html
        let default_clock_id = this.eval_libc("CLOCK_REALTIME")?;
        condattr_set_clock_id(this, attr_op, default_clock_id)?;

        Ok(0)
    }

    fn pthread_condattr_setclock(
        &mut self,
        attr_op: &OpTy<'tcx, Tag>,
        clock_id_op: &OpTy<'tcx, Tag>,
    ) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let clock_id = this.read_scalar(clock_id_op)?.check_init()?;
        if clock_id == this.eval_libc("CLOCK_REALTIME")?
            || clock_id == this.eval_libc("CLOCK_MONOTONIC")?
        {
            condattr_set_clock_id(this, attr_op, clock_id)?;
        } else {
            let einval = this.eval_libc_i32("EINVAL")?;
            return Ok(einval);
        }

        Ok(0)
    }

    fn pthread_condattr_getclock(
        &mut self,
        attr_op: &OpTy<'tcx, Tag>,
        clk_id_op: &OpTy<'tcx, Tag>,
    ) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let clock_id = condattr_get_clock_id(this, attr_op)?;
        this.write_scalar(clock_id, &this.deref_operand(clk_id_op)?.into())?;

        Ok(0)
    }

    fn pthread_condattr_destroy(&mut self, attr_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        condattr_set_clock_id(this, attr_op, ScalarMaybeUninit::Uninit)?;

        Ok(0)
    }

    fn pthread_cond_init(
        &mut self,
        cond_op: &OpTy<'tcx, Tag>,
        attr_op: &OpTy<'tcx, Tag>,
    ) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let attr = this.read_scalar(attr_op)?.check_init()?;
        let clock_id = if this.is_null(attr)? {
            this.eval_libc("CLOCK_REALTIME")?
        } else {
            condattr_get_clock_id(this, attr_op)?.check_init()?
        };

        // Write 0 to use the same code path as the static initializers.
        cond_set_id(this, cond_op, Scalar::from_i32(0))?;

        cond_set_clock_id(this, cond_op, clock_id)?;

        Ok(0)
    }

    fn pthread_cond_signal(&mut self, cond_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();
        let id = cond_get_or_create_id(this, cond_op)?;
        if let Some((thread, mutex)) = this.condvar_signal(id) {
            post_cond_signal(this, thread, mutex)?;
        }

        Ok(0)
    }

    fn pthread_cond_broadcast(&mut self, cond_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();
        let id = cond_get_or_create_id(this, cond_op)?;

        while let Some((thread, mutex)) = this.condvar_signal(id) {
            post_cond_signal(this, thread, mutex)?;
        }

        Ok(0)
    }

    fn pthread_cond_wait(
        &mut self,
        cond_op: &OpTy<'tcx, Tag>,
        mutex_op: &OpTy<'tcx, Tag>,
    ) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let id = cond_get_or_create_id(this, cond_op)?;
        let mutex_id = mutex_get_or_create_id(this, mutex_op)?;
        let active_thread = this.get_active_thread();

        release_cond_mutex_and_block(this, active_thread, mutex_id)?;
        this.condvar_wait(id, active_thread, mutex_id);

        Ok(0)
    }

    fn pthread_cond_timedwait(
        &mut self,
        cond_op: &OpTy<'tcx, Tag>,
        mutex_op: &OpTy<'tcx, Tag>,
        abstime_op: &OpTy<'tcx, Tag>,
        dest: &PlaceTy<'tcx, Tag>,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();

        this.check_no_isolation("`pthread_cond_timedwait`")?;

        let id = cond_get_or_create_id(this, cond_op)?;
        let mutex_id = mutex_get_or_create_id(this, mutex_op)?;
        let active_thread = this.get_active_thread();

        // Extract the timeout.
        let clock_id = cond_get_clock_id(this, cond_op)?.to_i32()?;
        let duration = match this.read_timespec(abstime_op)? {
            Some(duration) => duration,
            None => {
                let einval = this.eval_libc("EINVAL")?;
                this.write_scalar(einval, dest)?;
                return Ok(());
            }
        };

        let timeout_time = if clock_id == this.eval_libc_i32("CLOCK_REALTIME")? {
            Time::RealTime(SystemTime::UNIX_EPOCH.checked_add(duration).unwrap())
        } else if clock_id == this.eval_libc_i32("CLOCK_MONOTONIC")? {
            Time::Monotonic(this.machine.time_anchor.checked_add(duration).unwrap())
        } else {
            throw_unsup_format!("unsupported clock id: {}", clock_id);
        };

        release_cond_mutex_and_block(this, active_thread, mutex_id)?;
        this.condvar_wait(id, active_thread, mutex_id);

        // We return success for now and override it in the timeout callback.
        this.write_scalar(Scalar::from_i32(0), dest)?;

        let dest = *dest;

        // Register the timeout callback.
        this.register_timeout_callback(
            active_thread,
            timeout_time,
            Box::new(move |ecx| {
                // We are not waiting for the condvar any more, wait for the
                // mutex instead.
                reacquire_cond_mutex(ecx, active_thread, mutex_id)?;

                // Remove the thread from the conditional variable.
                ecx.condvar_remove_waiter(id, active_thread);

                // Set the return value: we timed out.
                let etimedout = ecx.eval_libc("ETIMEDOUT")?;
                ecx.write_scalar(etimedout, &dest)?;

                Ok(())
            }),
        );

        Ok(())
    }

    fn pthread_cond_destroy(&mut self, cond_op: &OpTy<'tcx, Tag>) -> InterpResult<'tcx, i32> {
        let this = self.eval_context_mut();

        let id = cond_get_or_create_id(this, cond_op)?;
        if this.condvar_is_awaited(id) {
            throw_ub_format!("destroying an awaited conditional variable");
        }
        cond_set_id(this, cond_op, ScalarMaybeUninit::Uninit)?;
        cond_set_clock_id(this, cond_op, ScalarMaybeUninit::Uninit)?;
        // FIXME: delete interpreter state associated with this condvar.

        Ok(0)
    }
}
