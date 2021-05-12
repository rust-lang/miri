use std::collections::{hash_map::Entry, HashMap, VecDeque};
use std::convert::TryFrom;
use std::num::NonZeroU32;
use std::ops::Not;

use log::trace;

use rustc_index::vec::{Idx, IndexVec};

use crate::*;

/// We cannot use the `newtype_index!` macro because we have to use 0 as a
/// sentinel value meaning that the identifier is not assigned. This is because
/// the pthreads static initializers initialize memory with zeros (see the
/// `src/shims/sync.rs` file).
macro_rules! declare_id {
    ($name: ident) => {
        /// 0 is used to indicate that the id was not yet assigned and,
        /// therefore, is not a valid identifier.
        #[derive(Clone, Copy, Debug, PartialOrd, Ord, PartialEq, Eq, Hash)]
        pub struct $name(NonZeroU32);

        impl $name {
            // Panics if `id == 0`.
            pub fn from_u32(id: u32) -> Self {
                Self(NonZeroU32::new(id).unwrap())
            }
        }

        impl Idx for $name {
            fn new(idx: usize) -> Self {
                // We use 0 as a sentinel value (see the comment above) and,
                // therefore, need to shift by one when converting from an index
                // into a vector.
                let shifted_idx = u32::try_from(idx).unwrap().checked_add(1).unwrap();
                $name(NonZeroU32::new(shifted_idx).unwrap())
            }
            fn index(self) -> usize {
                // See the comment in `Self::new`.
                // (This cannot underflow because self is NonZeroU32.)
                usize::try_from(self.0.get() - 1).unwrap()
            }
        }

        impl $name {
            pub fn to_u32_scalar<'tcx>(&self) -> Scalar<Tag> {
                Scalar::from_u32(self.0.get())
            }
        }
    };
}

declare_id!(MutexId);

/// The mutex state.
#[derive(Default, Debug)]
struct Mutex {
    /// The thread that currently owns the lock.
    owner: Option<ThreadId>,
    /// How many times the mutex was locked by the owner.
    lock_count: usize,
    /// The queue of threads waiting for this mutex.
    queue: VecDeque<ThreadId>,
    /// Data race handle, this tracks the happens-before
    /// relationship between each mutex access. It is
    /// released to during unlock and acquired from during
    /// locking, and therefore stores the clock of the last
    /// thread to release this mutex.
    data_race: VClock,
}

declare_id!(RwLockId);

/// The read-write lock state.
#[derive(Default, Debug)]
struct RwLock {
    /// The writer thread that currently owns the lock.
    writer: Option<ThreadId>,
    /// The readers that currently own the lock and how many times they acquired
    /// the lock.
    readers: HashMap<ThreadId, usize>,
    /// The queue of writer threads waiting for this lock.
    writer_queue: VecDeque<ThreadId>,
    /// The queue of reader threads waiting for this lock.
    reader_queue: VecDeque<ThreadId>,
    /// Data race handle for writers, tracks the happens-before
    /// ordering between each write access to a rwlock and is updated
    /// after a sequence of concurrent readers to track the happens-
    /// before ordering between the set of previous readers and
    /// the current writer.
    /// Contains the clock of the last thread to release a writer
    /// lock or the joined clock of the set of last threads to release
    /// shared reader locks.
    data_race: VClock,
    /// Data race handle for readers, this is temporary storage
    /// for the combined happens-before ordering for between all
    /// concurrent readers and the next writer, and the value
    /// is stored to the main data_race variable once all
    /// readers are finished.
    /// Has to be stored separately since reader lock acquires
    /// must load the clock of the last write and must not
    /// add happens-before orderings between shared reader
    /// locks.
    data_race_reader: VClock,
}

declare_id!(CondvarId);

/// A thread waiting on a conditional variable.
#[derive(Debug)]
struct CondvarWaiter {
    /// The thread that is waiting on this variable.
    thread: ThreadId,
    /// The mutex on which the thread is waiting.
    mutex: MutexId,
}

/// The conditional variable state.
#[derive(Default, Debug)]
struct Condvar {
    waiters: VecDeque<CondvarWaiter>,
    /// Tracks the happens-before relationship
    /// between a cond-var signal and a cond-var
    /// wait during a non-suprious signal event.
    /// Contains the clock of the last thread to
    /// perform a futex-signal.
    data_race: VClock,
}

/// The futex state.
#[derive(Default, Debug)]
struct Futex {
    waiters: VecDeque<FutexWaiter>,
    /// Tracks the happens-before relationship
    /// between a futex-wake and a futex-wait
    /// during a non-spurious wake event.
    /// Contains the clock of the last thread to
    /// perform a futex-wake.
    data_race: VClock,
}

/// A thread waiting on a futex.
#[derive(Debug)]
struct FutexWaiter {
    /// The thread that is waiting on this futex.
    thread: ThreadId,
}

/// The state of all synchronization variables.
#[derive(Default, Debug)]
pub(super) struct SynchronizationState {
    mutexes: IndexVec<MutexId, Mutex>,
    rwlocks: IndexVec<RwLockId, RwLock>,
    condvars: IndexVec<CondvarId, Condvar>,
    futexes: HashMap<Pointer, Futex>,
}

// Private extension trait for local helper methods
impl<'mir, 'tcx: 'mir> EvalContextExtPriv<'mir, 'tcx> for crate::MiriEvalContext<'mir, 'tcx> {}
trait EvalContextExtPriv<'mir, 'tcx: 'mir>: crate::MiriEvalContextExt<'mir, 'tcx> {
    /// Take a reader out of the queue waiting for the lock.
    /// Returns `true` if some thread got the rwlock.
    #[inline]
    fn rwlock_dequeue_and_lock_reader(&mut self, id: RwLockId) -> bool {
        let this = self.eval_context_mut();
        if let Some(reader) = this.machine.threads.sync.rwlocks[id].reader_queue.pop_front() {
            this.unblock_thread(reader);
            this.rwlock_reader_lock(id, reader);
            true
        } else {
            false
        }
    }

    /// Take the writer out of the queue waiting for the lock.
    /// Returns `true` if some thread got the rwlock.
    #[inline]
    fn rwlock_dequeue_and_lock_writer(&mut self, id: RwLockId) -> bool {
        let this = self.eval_context_mut();
        if let Some(writer) = this.machine.threads.sync.rwlocks[id].writer_queue.pop_front() {
            this.unblock_thread(writer);
            this.rwlock_writer_lock(id, writer);
            true
        } else {
            false
        }
    }

    /// Take a thread out of the queue waiting for the mutex, and lock
    /// the mutex for it. Returns `true` if some thread has the mutex now.
    #[inline]
    fn mutex_dequeue_and_lock(&mut self, id: MutexId) -> bool {
        let this = self.eval_context_mut();
        if let Some(thread) = this.machine.threads.sync.mutexes[id].queue.pop_front() {
            this.unblock_thread(thread);
            this.mutex_lock(id, thread);
            true
        } else {
            false
        }
    }
}

// Public interface to synchronization primitives. Please note that in most
// cases, the function calls are infallible and it is the client's (shim
// implementation's) responsibility to detect and deal with erroneous
// situations.
impl<'mir, 'tcx: 'mir> EvalContextExt<'mir, 'tcx> for crate::MiriEvalContext<'mir, 'tcx> {}
pub trait EvalContextExt<'mir, 'tcx: 'mir>: crate::MiriEvalContextExt<'mir, 'tcx> {
    #[inline]
    /// Create state for a new mutex.
    fn mutex_create(&mut self) -> MutexId {
        let this = self.eval_context_mut();
        this.machine.threads.sync.mutexes.push(Default::default())
    }

    #[inline]
    /// Get the id of the thread that currently owns this lock.
    fn mutex_get_owner(&mut self, id: MutexId) -> ThreadId {
        let this = self.eval_context_ref();
        this.machine.threads.sync.mutexes[id].owner.unwrap()
    }

    #[inline]
    /// Check if locked.
    fn mutex_is_locked(&self, id: MutexId) -> bool {
        let this = self.eval_context_ref();
        this.machine.threads.sync.mutexes[id].owner.is_some()
    }

    /// Lock by setting the mutex owner and increasing the lock count.
    fn mutex_lock(&mut self, id: MutexId, thread: ThreadId) {
        let this = self.eval_context_mut();
        let mutex = &mut this.machine.threads.sync.mutexes[id];
        if let Some(current_owner) = mutex.owner {
            assert_eq!(thread, current_owner, "mutex already locked by another thread");
            assert!(
                mutex.lock_count > 0,
                "invariant violation: lock_count == 0 iff the thread is unlocked"
            );
        } else {
            mutex.owner = Some(thread);
        }
        mutex.lock_count = mutex.lock_count.checked_add(1).unwrap();
        if let Some(data_race) = &this.memory.extra.data_race {
            data_race.validate_lock_acquire(&mutex.data_race, thread);
        }
    }

    /// Try unlocking by decreasing the lock count and returning the old lock
    /// count. If the lock count reaches 0, release the lock and potentially
    /// give to a new owner. If the lock was not locked by `expected_owner`,
    /// return `None`.
    fn mutex_unlock(&mut self, id: MutexId, expected_owner: ThreadId) -> Option<usize> {
        let this = self.eval_context_mut();
        let mutex = &mut this.machine.threads.sync.mutexes[id];
        if let Some(current_owner) = mutex.owner {
            // Mutex is locked.
            if current_owner != expected_owner {
                // Only the owner can unlock the mutex.
                return None;
            }
            let old_lock_count = mutex.lock_count;
            mutex.lock_count = old_lock_count
                .checked_sub(1)
                .expect("invariant violation: lock_count == 0 iff the thread is unlocked");
            if mutex.lock_count == 0 {
                mutex.owner = None;
                // The mutex is completely unlocked. Try transfering ownership
                // to another thread.
                if let Some(data_race) = &this.memory.extra.data_race {
                    data_race.validate_lock_release(&mut mutex.data_race, current_owner);
                }
                this.mutex_dequeue_and_lock(id);
            }
            Some(old_lock_count)
        } else {
            // Mutex is not locked.
            None
        }
    }

    #[inline]
    /// Put the thread into the queue waiting for the mutex.
    fn mutex_enqueue_and_block(&mut self, id: MutexId, thread: ThreadId) {
        let this = self.eval_context_mut();
        assert!(this.mutex_is_locked(id), "queing on unlocked mutex");
        this.machine.threads.sync.mutexes[id].queue.push_back(thread);
        this.block_thread(thread);
    }

    #[inline]
    /// Create state for a new read write lock.
    fn rwlock_create(&mut self) -> RwLockId {
        let this = self.eval_context_mut();
        this.machine.threads.sync.rwlocks.push(Default::default())
    }

    #[inline]
    /// Check if locked.
    fn rwlock_is_locked(&self, id: RwLockId) -> bool {
        let this = self.eval_context_ref();
        let rwlock = &this.machine.threads.sync.rwlocks[id];
        trace!(
            "rwlock_is_locked: {:?} writer is {:?} and there are {} reader threads (some of which could hold multiple read locks)",
            id,
            rwlock.writer,
            rwlock.readers.len(),
        );
        rwlock.writer.is_some() || rwlock.readers.is_empty().not()
    }

    #[inline]
    /// Check if write locked.
    fn rwlock_is_write_locked(&self, id: RwLockId) -> bool {
        let this = self.eval_context_ref();
        let rwlock = &this.machine.threads.sync.rwlocks[id];
        trace!("rwlock_is_write_locked: {:?} writer is {:?}", id, rwlock.writer);
        rwlock.writer.is_some()
    }

    /// Read-lock the lock by adding the `reader` the list of threads that own
    /// this lock.
    fn rwlock_reader_lock(&mut self, id: RwLockId, reader: ThreadId) {
        let this = self.eval_context_mut();
        assert!(!this.rwlock_is_write_locked(id), "the lock is write locked");
        trace!("rwlock_reader_lock: {:?} now also held (one more time) by {:?}", id, reader);
        let rwlock = &mut this.machine.threads.sync.rwlocks[id];
        let count = rwlock.readers.entry(reader).or_insert(0);
        *count = count.checked_add(1).expect("the reader counter overflowed");
        if let Some(data_race) = &this.memory.extra.data_race {
            data_race.validate_lock_acquire(&rwlock.data_race, reader);
        }
    }

    /// Try read-unlock the lock for `reader` and potentially give the lock to a new owner.
    /// Returns `true` if succeeded, `false` if this `reader` did not hold the lock.
    fn rwlock_reader_unlock(&mut self, id: RwLockId, reader: ThreadId) -> bool {
        let this = self.eval_context_mut();
        let rwlock = &mut this.machine.threads.sync.rwlocks[id];
        match rwlock.readers.entry(reader) {
            Entry::Occupied(mut entry) => {
                let count = entry.get_mut();
                assert!(*count > 0, "rwlock locked with count == 0");
                *count -= 1;
                if *count == 0 {
                    trace!("rwlock_reader_unlock: {:?} no longer held by {:?}", id, reader);
                    entry.remove();
                } else {
                    trace!("rwlock_reader_unlock: {:?} held one less time by {:?}", id, reader);
                }
            }
            Entry::Vacant(_) => return false, // we did not even own this lock
        }
        if let Some(data_race) = &this.memory.extra.data_race {
            data_race.validate_lock_release_shared(&mut rwlock.data_race_reader, reader);
        }

        // The thread was a reader. If the lock is not held any more, give it to a writer.
        if this.rwlock_is_locked(id).not() {
            // All the readers are finished, so set the writer data-race handle to the value
            //  of the union of all reader data race handles, since the set of readers
            //  happen-before the writers
            let rwlock = &mut this.machine.threads.sync.rwlocks[id];
            rwlock.data_race.clone_from(&rwlock.data_race_reader);
            this.rwlock_dequeue_and_lock_writer(id);
        }
        true
    }

    #[inline]
    /// Put the reader in the queue waiting for the lock and block it.
    fn rwlock_enqueue_and_block_reader(&mut self, id: RwLockId, reader: ThreadId) {
        let this = self.eval_context_mut();
        assert!(this.rwlock_is_write_locked(id), "read-queueing on not write locked rwlock");
        this.machine.threads.sync.rwlocks[id].reader_queue.push_back(reader);
        this.block_thread(reader);
    }

    #[inline]
    /// Lock by setting the writer that owns the lock.
    fn rwlock_writer_lock(&mut self, id: RwLockId, writer: ThreadId) {
        let this = self.eval_context_mut();
        assert!(!this.rwlock_is_locked(id), "the rwlock is already locked");
        trace!("rwlock_writer_lock: {:?} now held by {:?}", id, writer);
        let rwlock = &mut this.machine.threads.sync.rwlocks[id];
        rwlock.writer = Some(writer);
        if let Some(data_race) = &this.memory.extra.data_race {
            data_race.validate_lock_acquire(&rwlock.data_race, writer);
        }
    }

    #[inline]
    /// Try to unlock by removing the writer.
    fn rwlock_writer_unlock(&mut self, id: RwLockId, expected_writer: ThreadId) -> bool {
        let this = self.eval_context_mut();
        let rwlock = &mut this.machine.threads.sync.rwlocks[id];
        if let Some(current_writer) = rwlock.writer {
            if current_writer != expected_writer {
                // Only the owner can unlock the rwlock.
                return false;
            }
            rwlock.writer = None;
            trace!("rwlock_writer_unlock: {:?} unlocked by {:?}", id, expected_writer);
            // Release memory to both reader and writer vector clocks
            //  since this writer happens-before both the union of readers once they are finished
            //  and the next writer
            if let Some(data_race) = &this.memory.extra.data_race {
                data_race.validate_lock_release(&mut rwlock.data_race, current_writer);
                data_race.validate_lock_release(&mut rwlock.data_race_reader, current_writer);
            }
            // The thread was a writer.
            //
            // We are prioritizing writers here against the readers. As a
            // result, not only readers can starve writers, but also writers can
            // starve readers.
            if this.rwlock_dequeue_and_lock_writer(id) {
                // Someone got the write lock, nice.
            } else {
                // Give the lock to all readers.
                while this.rwlock_dequeue_and_lock_reader(id) {
                    // Rinse and repeat.
                }
            }
            true
        } else {
            false
        }
    }

    #[inline]
    /// Put the writer in the queue waiting for the lock.
    fn rwlock_enqueue_and_block_writer(&mut self, id: RwLockId, writer: ThreadId) {
        let this = self.eval_context_mut();
        assert!(this.rwlock_is_locked(id), "write-queueing on unlocked rwlock");
        this.machine.threads.sync.rwlocks[id].writer_queue.push_back(writer);
        this.block_thread(writer);
    }

    #[inline]
    /// Create state for a new conditional variable.
    fn condvar_create(&mut self) -> CondvarId {
        let this = self.eval_context_mut();
        this.machine.threads.sync.condvars.push(Default::default())
    }

    #[inline]
    /// Is the conditional variable awaited?
    fn condvar_is_awaited(&mut self, id: CondvarId) -> bool {
        let this = self.eval_context_mut();
        !this.machine.threads.sync.condvars[id].waiters.is_empty()
    }

    /// Mark that the thread is waiting on the conditional variable.
    fn condvar_wait(&mut self, id: CondvarId, thread: ThreadId, mutex: MutexId) {
        let this = self.eval_context_mut();
        let waiters = &mut this.machine.threads.sync.condvars[id].waiters;
        assert!(waiters.iter().all(|waiter| waiter.thread != thread), "thread is already waiting");
        waiters.push_back(CondvarWaiter { thread, mutex });
    }

    /// Wake up some thread (if there is any) sleeping on the conditional
    /// variable.
    fn condvar_signal(&mut self, id: CondvarId) -> Option<(ThreadId, MutexId)> {
        let this = self.eval_context_mut();
        let current_thread = this.get_active_thread();
        let condvar = &mut this.machine.threads.sync.condvars[id];
        let data_race = &this.memory.extra.data_race;

        // Each condvar signal happens-before the end of the condvar wake
        if let Some(data_race) = data_race {
            data_race.validate_lock_release(&mut condvar.data_race, current_thread);
        }
        condvar.waiters.pop_front().map(|waiter| {
            if let Some(data_race) = data_race {
                data_race.validate_lock_acquire(&mut condvar.data_race, waiter.thread);
            }
            (waiter.thread, waiter.mutex)
        })
    }

    #[inline]
    /// Remove the thread from the queue of threads waiting on this conditional variable.
    fn condvar_remove_waiter(&mut self, id: CondvarId, thread: ThreadId) {
        let this = self.eval_context_mut();
        this.machine.threads.sync.condvars[id].waiters.retain(|waiter| waiter.thread != thread);
    }

    fn futex_wait(&mut self, addr: Pointer<stacked_borrows::Tag>, thread: ThreadId) {
        let this = self.eval_context_mut();
        let futex = &mut this.machine.threads.sync.futexes.entry(addr.erase_tag()).or_default();
        let waiters = &mut futex.waiters;
        assert!(waiters.iter().all(|waiter| waiter.thread != thread), "thread is already waiting");
        waiters.push_back(FutexWaiter { thread });
    }

    fn futex_wake(&mut self, addr: Pointer<stacked_borrows::Tag>) -> Option<ThreadId> {
        let this = self.eval_context_mut();
        let current_thread = this.get_active_thread();
        let futex = &mut this.machine.threads.sync.futexes.get_mut(&addr.erase_tag())?;
        let data_race = &this.memory.extra.data_race;

        // Each futex-wake happens-before the end of the futex wait
        if let Some(data_race) = data_race {
            data_race.validate_lock_release(&mut futex.data_race, current_thread);
        }
        let res = futex.waiters.pop_front().map(|waiter| {
            if let Some(data_race) = data_race {
                data_race.validate_lock_acquire(&futex.data_race, waiter.thread);
            }
            waiter.thread
        });
        res
    }

    fn futex_remove_waiter(&mut self, addr: Pointer<stacked_borrows::Tag>, thread: ThreadId) {
        let this = self.eval_context_mut();
        if let Some(futex) = this.machine.threads.sync.futexes.get_mut(&addr.erase_tag()) {
            futex.waiters.retain(|waiter| waiter.thread != thread);
        }
    }
}
