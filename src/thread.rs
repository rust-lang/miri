//! Implements threads.

use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::convert::TryFrom;
use std::rc::Rc;
use std::num::TryFromIntError;
use std::time::{Duration, Instant, SystemTime};

use log::trace;

use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use rustc_hir::def_id::DefId;
use rustc_index::vec::{Idx, IndexVec};
use rustc_target::abi::Size;

use crate::sync::{SynchronizationState, CondvarId};
use crate::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulingAction {
    /// Execute step on the active thread.
    ExecuteStep,
    /// Execute a timeout callback.
    ExecuteTimeoutCallback,
    /// Execute destructors of the active thread.
    ExecuteDtors,
    /// Stop the program.
    Stop,
}

/// Timeout callbacks can be created by synchronization primitives to tell the
/// scheduler that they should be called once some period of time passes.
type TimeoutCallback<'mir, 'tcx> =
    Box<dyn FnOnce(&mut InterpCx<'mir, 'tcx, Evaluator<'mir, 'tcx>>) -> InterpResult<'tcx> + 'tcx>;

/// A thread identifier.
#[derive(Clone, Copy, Debug, PartialOrd, Ord, PartialEq, Eq, Hash)]
pub struct ThreadId(u32);

/// The main thread. When it terminates, the whole application terminates.
const MAIN_THREAD: ThreadId = ThreadId(0);

impl ThreadId {
    pub fn to_u32(self) -> u32 {
        self.0
    }
}

impl Idx for ThreadId {
    fn new(idx: usize) -> Self {
        ThreadId(u32::try_from(idx).unwrap())
    }

    fn index(self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

impl TryFrom<u64> for ThreadId {
    type Error = TryFromIntError;
    fn try_from(id: u64) -> Result<Self, Self::Error> {
        u32::try_from(id).map(|id_u32| Self(id_u32))
    }
}

impl From<u32> for ThreadId {
    fn from(id: u32) -> Self {
        Self(id)
    }
}

impl ThreadId {
    pub fn to_u32_scalar<'tcx>(&self) -> Scalar<Tag> {
        Scalar::from_u32(u32::try_from(self.0).unwrap())
    }
}

/// The state of a thread.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ThreadState {
    /// The thread is enabled and can be executed.
    Enabled,
    /// The thread tried to join the specified thread and is blocked until that
    /// thread terminates.
    BlockedOnJoin(ThreadId),
    /// The thread is blocked on some synchronization primitive. It is the
    /// responsibility of the synchronization primitives to track threads that
    /// are blocked by them.
    BlockedOnSync,
    /// The thread has yielded, but the full number of validation iterations
    /// has not yet occurred.
    /// Therefore this thread can be awoken if there are no enabled threads
    /// available.
    DelayOnYield,
    /// The thread has fully yielded, signalling that it requires another thread
    /// perform an action visible to it in order to make progress.
    /// If all threads are in this state then live-lock is reported.
    BlockedOnYield,
    /// The thread has terminated its execution. We do not delete terminated
    /// threads (FIXME: why?).
    Terminated,
}

/// The join status of a thread.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ThreadJoinStatus {
    /// The thread can be joined.
    Joinable,
    /// A thread is detached if its join handle was destroyed and no other
    /// thread can join it.
    Detached,
    /// The thread was already joined by some thread and cannot be joined again.
    Joined,
}

/// Set of sync objects that can have properties queried.
/// The futex is not included since it can only signal
/// and awake values.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum SyncObject {
    /// Can query lock state.
    Mutex(MutexId),
    /// Can query lock state.
    RwLock(RwLockId),
    /// Can query the if awaited.
    Condvar(CondvarId),

}

enum YieldRecordState {

    /// The thread has made progress and so the recording of
    /// states has terminated.
    /// A thread is considered to have made progress performs some
    /// action that is visible to another thread.
    /// Examples include a release-store, mutex-unlock,
    /// thread-termination and release-fence.
    MadeProgress,

    /// The thread is currently recording the variable watch
    /// state for the yield operation, this watch operation
    /// can occur once of multiple times to validate the yield
    /// is correct.
    /// Multiple iterations can prevent reporting live-lock
    /// in yield loops with finite iterations
    Recording {
        /// Map of global memory state to an record index.
        /// If set to zero then this location is not watched,
        /// a value of 1 means that it was watched on the first
        /// iteration and a value of x>=2 means that the value
        /// was watched on the 1,2,..,x iterations.
        watch_atomic: FxHashMap<AllocId, RangeMap<u32>>,

        /// Map of synchronization objects that can have properties
        /// queried and are currently watched by this thread.
        watch_sync: FxHashMap<SyncObject, u32>,
        
        /// The current iteration of the yield livelock loop
        /// recording, should always be less than or equal
        /// to the global live-lock loop counter.
        record_iteration: u32,
    }
}

impl YieldRecordState {
    
    /// Progress has been made on the thread.
    fn on_progress(&mut self) {
        *self = YieldRecordState::MadeProgress;
    }
    
    /// Mark an atomic variable as watched.
    fn on_watch_atomic(&mut self, alloc_id: AllocId, alloc_size: Size, offset: Size, len: Size) {
        if let YieldRecordState::Recording {
            watch_atomic, record_iteration, ..
        } = self {
            let range_map = watch_atomic.entry(alloc_id)
            .or_insert_with(|| RangeMap::new(alloc_size, 0));
            let mut assume_progress = false;
            range_map.iter_mut(offset, len)
            .for_each(|(_, watch)| {
                if *watch != *record_iteration - 1 {
                    // Value stored does not match the last loop
                    // so assume some progress has been made.
                    assume_progress = true;
                }
                *watch = *record_iteration
            });

            // The watch set is different so assume progress has been made
            if assume_progress {
                *self = YieldRecordState::MadeProgress;
            }
        }
    }

    /// Mark a synchronization object as watched.
    fn on_watch_sync(&mut self, sync: SyncObject) {
        if let YieldRecordState::Recording {
            watch_sync, record_iteration, ..
        } = self {
            let count = watch_sync.entry(sync).or_insert(0);
            if *count != *record_iteration - 1 {
                // Different content - assume progress.
                *self = YieldRecordState::MadeProgress; 
            }else{
                *count = *record_iteration;
            }
        }
    }

    /// Returns true if the atomic variable is currently watched.
    fn should_wake_atomic(&self, alloc_id: AllocId, offset: Size, len: Size) -> bool {
        if let YieldRecordState::Recording {
            watch_atomic, ..
        } = self {
            if let Some(range_map) = watch_atomic.get(&alloc_id) {
                range_map.iter(offset, len).any(|(_, &watch)| watch != 0)
            }else{
                false
            }
        }else{
            // First iteration yield, no wake metadata
            // so only awaken after there are no enabled threads.
            false
        }
    }

    /// Returns true if the sync object is currently watched.
    fn should_wake_sync(&self, sync: SyncObject) -> bool {
        if let YieldRecordState::Recording {
            watch_sync, ..
        } = self {
            if let Some(count) = watch_sync.get(&sync) {
                *count != 0
            }else{
                false
            }
        }else{
            // First iteration, no wake metadata.
            false
        }
    }

    /// Returns the number of yield iterations that have been executed.
    fn get_iteration_count(&self) -> u32 {
        if let YieldRecordState::Recording {
            record_iteration, ..
        } = self {
            *record_iteration
        }else{
            0
        }
    }

    fn should_watch(&self) -> bool {
        if let YieldRecordState::Recording {
            watch_atomic, watch_sync, ..
        } = self {
            // Should watch if either watch hash-set is non-empty
            !watch_atomic.is_empty() || !watch_sync.is_empty()
        }else{
            false
        }
    }

    /// Starts the next yield iteration
    fn start_iteration(&mut self) {
        if let YieldRecordState::Recording {
            record_iteration, ..
        } = self {
            *record_iteration += 1;
        }else{
            *self = YieldRecordState::Recording {
                watch_atomic: FxHashMap::default(),
                watch_sync: FxHashMap::default(),
                record_iteration: 1
            }
        }
    }
}

/// A thread.
pub struct Thread<'mir, 'tcx> {
    state: ThreadState,

    /// Metadata for blocking on yield operations
    yield_state: YieldRecordState,

    /// Name of the thread.
    thread_name: Option<Vec<u8>>,

    /// The virtual call stack.
    stack: Vec<Frame<'mir, 'tcx, Tag, FrameData<'tcx>>>,

    /// The join status.
    join_status: ThreadJoinStatus,

    /// The temporary used for storing the argument of
    /// the call to `miri_start_panic` (the panic payload) when unwinding.
    /// This is pointer-sized, and matches the `Payload` type in `src/libpanic_unwind/miri.rs`.
    pub(crate) panic_payload: Option<Scalar<Tag>>,

    /// Last OS error location in memory. It is a 32-bit integer.
    pub(crate) last_error: Option<MPlaceTy<'tcx, Tag>>,
}

impl<'mir, 'tcx> Thread<'mir, 'tcx> {
    /// Check if the thread is done executing (no more stack frames). If yes,
    /// change the state to terminated and return `true`.
    fn check_terminated(&mut self) -> bool {
        if self.state == ThreadState::Enabled {
            if self.stack.is_empty() {
                self.state = ThreadState::Terminated;
                return true;
            }
        }
        false
    }

    /// Get the name of the current thread, or `<unnamed>` if it was not set.
    fn thread_name(&self) -> &[u8] {
        if let Some(ref thread_name) = self.thread_name {
            thread_name
        } else {
            b"<unnamed>"
        }
    }

    /// Start the thread yielding. Returns true if this thread has watch metadata.
    fn on_yield(&mut self, max_yield: u32) -> bool {
        let iteration_count = self.yield_state.get_iteration_count();
        let block = if max_yield == 0 {
            // A value of 0 never blocks
            false
        }else{
            iteration_count >= max_yield
        };
        if block {
            self.state = ThreadState::BlockedOnYield;
        }else{
            self.state = ThreadState::DelayOnYield;
        }
        self.yield_state.should_watch()
    }
}

impl<'mir, 'tcx> std::fmt::Debug for Thread<'mir, 'tcx> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({:?}, {:?})", String::from_utf8_lossy(self.thread_name()), self.state, self.join_status)
    }
}

impl<'mir, 'tcx> Default for Thread<'mir, 'tcx> {
    fn default() -> Self {
        Self {
            state: ThreadState::Enabled,
            yield_state: YieldRecordState::MadeProgress,
            thread_name: None,
            stack: Vec::new(),
            join_status: ThreadJoinStatus::Joinable,
            panic_payload: None,
            last_error: None,
        }
    }
}

/// A specific moment in time.
#[derive(Debug)]
pub enum Time {
    Monotonic(Instant),
    RealTime(SystemTime),
}

impl Time {
    /// How long do we have to wait from now until the specified time?
    fn get_wait_time(&self) -> Duration {
        match self {
            Time::Monotonic(instant) => instant.saturating_duration_since(Instant::now()),
            Time::RealTime(time) =>
                time.duration_since(SystemTime::now()).unwrap_or(Duration::new(0, 0)),
        }
    }
}

/// Callbacks are used to implement timeouts. For example, waiting on a
/// conditional variable with a timeout creates a callback that is called after
/// the specified time and unblocks the thread. If another thread signals on the
/// conditional variable, the signal handler deletes the callback.
struct TimeoutCallbackInfo<'mir, 'tcx> {
    /// The callback should be called no earlier than this time.
    call_time: Time,
    /// The called function.
    callback: TimeoutCallback<'mir, 'tcx>,
}

impl<'mir, 'tcx> std::fmt::Debug for TimeoutCallbackInfo<'mir, 'tcx> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TimeoutCallback({:?})", self.call_time)
    }
}

/// A set of threads.
#[derive(Debug)]
pub struct ThreadManager<'mir, 'tcx> {
    /// Identifier of the currently active thread.
    active_thread: ThreadId,
    /// Threads used in the program.
    ///
    /// Note that this vector also contains terminated threads.
    threads: IndexVec<ThreadId, Thread<'mir, 'tcx>>,
    /// Set of threads that are currently yielding.
    yielding_thread_set: FxHashSet<ThreadId>,
    /// The maximum number of yields making no progress required
    /// on all threads to report a live-lock.
    max_yield_count: u32,
    /// This field is pub(crate) because the synchronization primitives
    /// (`crate::sync`) need a way to access it.
    pub(crate) sync: SynchronizationState,
    /// A mapping from a thread-local static to an allocation id of a thread
    /// specific allocation.
    thread_local_alloc_ids: RefCell<FxHashMap<(DefId, ThreadId), AllocId>>,
    /// A flag that indicates that we should change the active thread.
    yield_active_thread: bool,
    /// Callbacks that are called once the specified time passes.
    timeout_callbacks: FxHashMap<ThreadId, TimeoutCallbackInfo<'mir, 'tcx>>,
}

impl<'mir, 'tcx> ThreadManager<'mir, 'tcx> {
    pub fn new(max_yield_count: u32) -> Self {
        let mut threads = IndexVec::new();
        // Create the main thread and add it to the list of threads.
        let mut main_thread = Thread::default();
        // The main thread can *not* be joined on.
        main_thread.join_status = ThreadJoinStatus::Detached;
        threads.push(main_thread);
        Self {
            active_thread: ThreadId::new(0),
            threads: threads,
            yielding_thread_set: FxHashSet::default(),
            max_yield_count,
            sync: SynchronizationState::default(),
            thread_local_alloc_ids: Default::default(),
            yield_active_thread: false,
            timeout_callbacks: FxHashMap::default(),
        }
    }
}

impl<'mir, 'tcx: 'mir> ThreadManager<'mir, 'tcx> {
    /// Check if we have an allocation for the given thread local static for the
    /// active thread.
    fn get_thread_local_alloc_id(&self, def_id: DefId) -> Option<AllocId> {
        self.thread_local_alloc_ids.borrow().get(&(def_id, self.active_thread)).cloned()
    }

    /// Set the allocation id as the allocation id of the given thread local
    /// static for the active thread.
    ///
    /// Panics if a thread local is initialized twice for the same thread.
    fn set_thread_local_alloc_id(&self, def_id: DefId, new_alloc_id: AllocId) {
        self.thread_local_alloc_ids
            .borrow_mut()
            .insert((def_id, self.active_thread), new_alloc_id)
            .unwrap_none();
    }

    /// Borrow the stack of the active thread.
    fn active_thread_stack(&self) -> &[Frame<'mir, 'tcx, Tag, FrameData<'tcx>>] {
        &self.threads[self.active_thread].stack
    }

    /// Mutably borrow the stack of the active thread.
    fn active_thread_stack_mut(&mut self) -> &mut Vec<Frame<'mir, 'tcx, Tag, FrameData<'tcx>>> {
        &mut self.threads[self.active_thread].stack
    }

    /// Create a new thread and returns its id.
    fn create_thread(&mut self) -> ThreadId {
        let new_thread_id = ThreadId::new(self.threads.len());
        self.threads.push(Default::default());
        new_thread_id
    }

    /// Set an active thread and return the id of the thread that was active before.
    fn set_active_thread_id(&mut self, id: ThreadId) -> ThreadId {
        let active_thread_id = self.active_thread;
        self.active_thread = id;
        assert!(self.active_thread.index() < self.threads.len());
        active_thread_id
    }

    /// Get the id of the currently active thread.
    fn get_active_thread_id(&self) -> ThreadId {
        self.active_thread
    }

    /// Get the total number of threads that were ever spawn by this program.
    fn get_total_thread_count(&self) -> usize {
        self.threads.len()
    }

    /// Has the given thread terminated?
    fn has_terminated(&self, thread_id: ThreadId) -> bool {
        self.threads[thread_id].state == ThreadState::Terminated
    }

    /// Enable the thread for execution. The thread must be terminated.
    fn enable_thread(&mut self, thread_id: ThreadId) {
        assert!(self.has_terminated(thread_id));
        self.threads[thread_id].state = ThreadState::Enabled;
    }

    /// Get a mutable borrow of the currently active thread.
    fn active_thread_mut(&mut self) -> &mut Thread<'mir, 'tcx> {
        &mut self.threads[self.active_thread]
    }

    /// Get a shared borrow of the currently active thread.
    fn active_thread_ref(&self) -> &Thread<'mir, 'tcx> {
        &self.threads[self.active_thread]
    }

    /// Mark the thread as detached, which means that no other thread will try
    /// to join it and the thread is responsible for cleaning up.
    fn detach_thread(&mut self, id: ThreadId) -> InterpResult<'tcx> {
        if self.threads[id].join_status != ThreadJoinStatus::Joinable {
            throw_ub_format!("trying to detach thread that was already detached or joined");
        }
        self.threads[id].join_status = ThreadJoinStatus::Detached;
        Ok(())
    }

    /// Mark that the active thread tries to join the thread with `joined_thread_id`.
    fn join_thread(&mut self, joined_thread_id: ThreadId, data_race: &Option<Rc<data_race::GlobalState>>) -> InterpResult<'tcx> {
        if self.threads[joined_thread_id].join_status != ThreadJoinStatus::Joinable {
            throw_ub_format!("trying to join a detached or already joined thread");
        }
        if joined_thread_id == self.active_thread {
            throw_ub_format!("trying to join itself");
        }
        assert!(
            self.threads
                .iter()
                .all(|thread| thread.state != ThreadState::BlockedOnJoin(joined_thread_id)),
            "a joinable thread already has threads waiting for its termination"
        );
        // Mark the joined thread as being joined so that we detect if other
        // threads try to join it.
        self.threads[joined_thread_id].join_status = ThreadJoinStatus::Joined;
        if self.threads[joined_thread_id].state != ThreadState::Terminated {
            // The joined thread is still running, we need to wait for it.
            self.active_thread_mut().state = ThreadState::BlockedOnJoin(joined_thread_id);
            trace!(
                "{:?} blocked on {:?} when trying to join",
                self.active_thread,
                joined_thread_id
            );
        } else {
            // The thread has already terminated - mark join happens-before
            if let Some(data_race) = data_race {
                data_race.thread_joined(self.active_thread, joined_thread_id);
            }
        }
        Ok(())
    }

    /// Set the name of the active thread.
    fn set_thread_name(&mut self, new_thread_name: Vec<u8>) {
        self.active_thread_mut().thread_name = Some(new_thread_name);
    }

    /// Get the name of the active thread.
    fn get_thread_name(&self) -> &[u8] {
        self.active_thread_ref().thread_name()
    }

    /// Put the thread into the blocked state.
    fn block_thread(&mut self, thread: ThreadId) {
        let state = &mut self.threads[thread].state;
        assert_eq!(*state, ThreadState::Enabled);
        *state = ThreadState::BlockedOnSync;
    }

    /// Put the blocked thread into the enabled state.
    fn unblock_thread(&mut self, thread: ThreadId) {
        let state = &mut self.threads[thread].state;
        assert_eq!(*state, ThreadState::BlockedOnSync);
        *state = ThreadState::Enabled;
    }

    /// Change the active thread to some enabled thread.
    fn yield_active_thread(&mut self) {
        // We do not yield immediately, as swapping out the current stack while executing a MIR statement
        // could lead to all sorts of confusion.
        // We should only switch stacks between steps.
        self.yield_active_thread = true;
    }

    /// Register the given `callback` to be called once the `call_time` passes.
    ///
    /// The callback will be called with `thread` being the active thread, and
    /// the callback may not change the active thread.
    fn register_timeout_callback(
        &mut self,
        thread: ThreadId,
        call_time: Time,
        callback: TimeoutCallback<'mir, 'tcx>,
    ) {
        self.timeout_callbacks
            .insert(thread, TimeoutCallbackInfo { call_time, callback })
            .unwrap_none();
    }

    /// Unregister the callback for the `thread`.
    fn unregister_timeout_callback_if_exists(&mut self, thread: ThreadId) {
        self.timeout_callbacks.remove(&thread);
    }

    /// Get a callback that is ready to be called.
    fn get_ready_callback(&mut self) -> Option<(ThreadId, TimeoutCallback<'mir, 'tcx>)> {
        // We iterate over all threads in the order of their indices because
        // this allows us to have a deterministic scheduler.
        for thread in self.threads.indices() {
            match self.timeout_callbacks.entry(thread) {
                Entry::Occupied(entry) =>
                    if entry.get().call_time.get_wait_time() == Duration::new(0, 0) {
                        return Some((thread, entry.remove().callback));
                    },
                Entry::Vacant(_) => {}
            }
        }
        None
    }

    /// Wakes up threads joining on the active one and deallocates thread-local statics.
    /// The `AllocId` that can now be freed is returned.
    fn thread_terminated(&mut self, data_race: &Option<Rc<data_race::GlobalState>>) -> Vec<AllocId> {
        let mut free_tls_statics = Vec::new();
        {
            let mut thread_local_statics = self.thread_local_alloc_ids.borrow_mut();
            thread_local_statics.retain(|&(_def_id, thread), &mut alloc_id| {
                if thread != self.active_thread {
                    // Keep this static around.
                    return true;
                }
                // Delete this static from the map and from memory.
                // We cannot free directly here as we cannot use `?` in this context.
                free_tls_statics.push(alloc_id);
                return false;
            });
        }
        // Set the thread into a terminated state in the data-race detector
        if let Some(data_race) = data_race {
            data_race.thread_terminated();
        }
        // Check if we need to unblock any threads.
        for (i, thread) in self.threads.iter_enumerated_mut() {
            if thread.state == ThreadState::BlockedOnJoin(self.active_thread) {
                // The thread has terminated, mark happens-before edge to joining thread
                if let Some(data_race) = data_race {
                    data_race.thread_joined(i, self.active_thread);
                }
                trace!("unblocking {:?} because {:?} terminated", i, self.active_thread);
                thread.state = ThreadState::Enabled;
            }
        }
        return free_tls_statics;
    }

    /// Called when the current thread performed
    /// an atomic operation that potentially makes
    /// progress
    fn thread_yield_progress(&mut self) {
        self.threads[self.active_thread].yield_state.on_progress();
    }

    /// Called when the current thread performs
    /// an atomic operation that may be visible to other threads.
    fn thread_yield_atomic_wake(&mut self, alloc_id: AllocId, offset: Size, len: Size) {
        let threads = &mut self.threads;

        // This thread performed an atomic update, mark as making progress.
        threads[self.active_thread].yield_state.on_progress();

        // Awake all threads that were awaiting on changes to the modified atomic.
        self.yielding_thread_set.drain_filter(move |&thread_id| {
            let thread = &mut threads[thread_id];
            if thread.yield_state.should_wake_atomic(alloc_id, offset, len) {
                thread.state = ThreadState::Enabled;
                thread.yield_state.on_progress();
                true
            }else{
                false
            }
        });
    }

    /// Called when the current thread performs
    /// an atomic read operation and may want
    /// to mark that variable as watched to wake
    /// the current yield.
    fn thread_yield_atomic_watch(&mut self, alloc_id: AllocId, alloc_size: Size, offset: Size, len: Size) {
        self.threads[self.active_thread].yield_state.on_watch_atomic(alloc_id, alloc_size, offset, len)
    }

    fn thread_yield_sync_wake(&mut self, sync: SyncObject) {
        let threads = &mut self.threads;

        // This thread performed an sync update, mark as making progress.
        threads[self.active_thread].yield_state.on_progress();

        // Awake all threads that were awaiting on changes to the sync object.
        self.yielding_thread_set.drain_filter(move |&thread_id| {
            let thread = &mut threads[thread_id];
            if thread.yield_state.should_wake_sync(sync) {
                thread.state = ThreadState::Enabled;
                thread.yield_state.on_progress();
                true
            }else{
                false
            }
        });
    }

    fn thread_yield_sync_watch(&mut self, sync: SyncObject) {
        self.threads[self.active_thread].yield_state.on_watch_sync(sync);
    }

    /// Decide which action to take next and on which thread.
    ///
    /// The currently implemented scheduling policy is the one that is commonly
    /// used in stateless model checkers such as Loom: run the active thread as
    /// long as we can and switch only when we have to (the active thread was
    /// blocked, terminated, or has explicitly asked to be preempted).
    fn schedule(&mut self, data_race: &Option<Rc<data_race::GlobalState>>) -> InterpResult<'tcx, SchedulingAction> {
        // Check whether the thread has **just** terminated (`check_terminated`
        // checks whether the thread has popped all its stack and if yes, sets
        // the thread state to terminated).
        if self.threads[self.active_thread].check_terminated() {
            return Ok(SchedulingAction::ExecuteDtors);
        }
        if self.threads[MAIN_THREAD].state == ThreadState::Terminated {
            // The main thread terminated; stop the program.
            if self.threads.iter().any(|thread| thread.state != ThreadState::Terminated) {
                // FIXME: This check should be either configurable or just emit
                // a warning. For example, it seems normal for a program to
                // terminate without waiting for its detached threads to
                // terminate. However, this case is not trivial to support
                // because we also probably do not want to consider the memory
                // owned by these threads as leaked.
                throw_unsup_format!("the main thread terminated without waiting for other threads");
            }
            return Ok(SchedulingAction::Stop);
        }
        // At least for `pthread_cond_timedwait` we need to report timeout when
        // the function is called already after the specified time even if a
        // signal is received before the thread gets scheduled. Therefore, we
        // need to schedule all timeout callbacks before we continue regular
        // execution.
        //
        // Documentation:
        // https://pubs.opengroup.org/onlinepubs/9699919799/functions/pthread_cond_timedwait.html#
        let potential_sleep_time =
            self.timeout_callbacks.values().map(|info| info.call_time.get_wait_time()).min();
        if potential_sleep_time == Some(Duration::new(0, 0)) {
            return Ok(SchedulingAction::ExecuteTimeoutCallback);
        }
        // No callbacks scheduled, pick a regular thread to execute.
        if self.threads[self.active_thread].state == ThreadState::Enabled {
            if self.yield_active_thread {
                // The currently active thread has yielded, update the state
                if self.threads[self.active_thread].on_yield(self.max_yield_count) {
                    // The thread has a non-zero set of wake metadata to exit the yield
                    // so insert into the set of threads that may wake.
                    self.yielding_thread_set.insert(self.active_thread);
                }
            }else{
                // The currently active thread is still enabled, just continue with it.
                return Ok(SchedulingAction::ExecuteStep);
            }
        }
        // We need to pick a new thread for execution.
        // First try to select a thread that has not yielded.
        let new_thread = if let Some(new_thread) = self.threads.iter_enumerated()
            .find(|(_, thread)| thread.state == ThreadState::Enabled) {
            Some(new_thread.0)
        }else{
            // No active threads, wake all non blocking yields and try again.
            let mut new_thread = None;
            for (id,thread) in self.threads.iter_enumerated_mut() {
                if thread.state == ThreadState::DelayOnYield {

                    // Re-enable the thread and start the next yield iteration.
                    thread.state = ThreadState::Enabled;
                    thread.yield_state.start_iteration();
                    self.yielding_thread_set.remove(&id);
                    new_thread = new_thread.or(Some(id));
                }
            }
            new_thread
        };
        self.yield_active_thread = false;

        // We found a valid thread to execute.
        if let Some(new_thread) = new_thread {
            self.active_thread = new_thread;
            if let Some(data_race) = data_race {
                data_race.thread_set_active(self.active_thread);
            }
            assert!(self.threads[self.active_thread].state == ThreadState::Enabled);
            return Ok(SchedulingAction::ExecuteStep);
        }

        // We have not found a thread to execute.
        if self.threads.iter().all(|thread| thread.state == ThreadState::Terminated) {
            unreachable!("all threads terminated without the main thread terminating?!");
        } else if let Some(sleep_time) = potential_sleep_time {
            // All threads are currently blocked, but we have unexecuted
            // timeout_callbacks, which may unblock some of the threads. Hence,
            // sleep until the first callback.
            std::thread::sleep(sleep_time);
            Ok(SchedulingAction::ExecuteTimeoutCallback)
        } else if self.threads.iter().any(|thread| thread.state == ThreadState::BlockedOnYield) {
            // At least one thread is blocked on a yield with max iterations.
            // Report a livelock instead of a deadlock.
            throw_machine_stop!(TerminationInfo::Livelock);
        } else {
            throw_machine_stop!(TerminationInfo::Deadlock);
        }
    }
}

// Public interface to thread management.
impl<'mir, 'tcx: 'mir> EvalContextExt<'mir, 'tcx> for crate::MiriEvalContext<'mir, 'tcx> {}
pub trait EvalContextExt<'mir, 'tcx: 'mir>: crate::MiriEvalContextExt<'mir, 'tcx> {
    /// Get a thread-specific allocation id for the given thread-local static.
    /// If needed, allocate a new one.
    fn get_or_create_thread_local_alloc_id(&mut self, def_id: DefId) -> InterpResult<'tcx, AllocId> {
        let this = self.eval_context_mut();
        let tcx = this.tcx;
        if let Some(new_alloc_id) = this.machine.threads.get_thread_local_alloc_id(def_id) {
            // We already have a thread-specific allocation id for this
            // thread-local static.
            Ok(new_alloc_id)
        } else {
            // We need to allocate a thread-specific allocation id for this
            // thread-local static.
            // First, we compute the initial value for this static.
            if tcx.is_foreign_item(def_id) {
                throw_unsup_format!("foreign thread-local statics are not supported");
            }
            let allocation = tcx.eval_static_initializer(def_id)?;
            // Create a fresh allocation with this content.
            let new_alloc_id = this.memory.allocate_with(allocation.clone(), MiriMemoryKind::Tls.into()).alloc_id;
            this.machine.threads.set_thread_local_alloc_id(def_id, new_alloc_id);
            Ok(new_alloc_id)
        }
    }

    #[inline]
    fn create_thread(&mut self) -> ThreadId {
        let this = self.eval_context_mut();
        let id = this.machine.threads.create_thread();
        if let Some(data_race) = &this.memory.extra.data_race {
            data_race.thread_created(id);
        }
        id
    }

    #[inline]
    fn detach_thread(&mut self, thread_id: ThreadId) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        this.machine.threads.detach_thread(thread_id)
    }

    #[inline]
    fn join_thread(&mut self, joined_thread_id: ThreadId) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let data_race = &this.memory.extra.data_race;
        this.machine.threads.join_thread(joined_thread_id, data_race)?;
        Ok(())
    }

    #[inline]
    fn set_active_thread(&mut self, thread_id: ThreadId) -> ThreadId {
        let this = self.eval_context_mut();
        if let Some(data_race) = &this.memory.extra.data_race {
            data_race.thread_set_active(thread_id);
        }
        this.machine.threads.set_active_thread_id(thread_id)
    }

    #[inline]
    fn get_active_thread(&self) -> ThreadId {
        let this = self.eval_context_ref();
        this.machine.threads.get_active_thread_id()
    }

    #[inline]
    fn active_thread_mut(&mut self) -> &mut Thread<'mir, 'tcx> {
        let this = self.eval_context_mut();
        this.machine.threads.active_thread_mut()
    }

    #[inline]
    fn active_thread_ref(&self) -> &Thread<'mir, 'tcx> {
        let this = self.eval_context_ref();
        this.machine.threads.active_thread_ref()
    }

    #[inline]
    fn get_total_thread_count(&self) -> usize {
        let this = self.eval_context_ref();
        this.machine.threads.get_total_thread_count()
    }

    #[inline]
    fn has_terminated(&self, thread_id: ThreadId) -> bool {
        let this = self.eval_context_ref();
        this.machine.threads.has_terminated(thread_id)
    }

    #[inline]
    fn enable_thread(&mut self, thread_id: ThreadId) {
        let this = self.eval_context_mut();
        this.machine.threads.enable_thread(thread_id);
    }

    #[inline]
    fn active_thread_stack(&self) -> &[Frame<'mir, 'tcx, Tag, FrameData<'tcx>>] {
        let this = self.eval_context_ref();
        this.machine.threads.active_thread_stack()
    }

    #[inline]
    fn active_thread_stack_mut(&mut self) -> &mut Vec<Frame<'mir, 'tcx, Tag, FrameData<'tcx>>> {
        let this = self.eval_context_mut();
        this.machine.threads.active_thread_stack_mut()
    }

    #[inline]
    fn set_active_thread_name(&mut self, new_thread_name: Vec<u8>) {
        let this = self.eval_context_mut();
        if let Some(data_race) = &this.memory.extra.data_race {
            if let Ok(string) = String::from_utf8(new_thread_name.clone()) {
                data_race.thread_set_name(
                    this.machine.threads.active_thread, string
                );
            }
        }
        this.machine.threads.set_thread_name(new_thread_name);
    }

    #[inline]
    fn get_active_thread_name<'c>(&'c self) -> &'c [u8]
    where
        'mir: 'c,
    {
        let this = self.eval_context_ref();
        this.machine.threads.get_thread_name()
    }

    #[inline]
    fn block_thread(&mut self, thread: ThreadId) {
        let this = self.eval_context_mut();
        this.machine.threads.block_thread(thread);

        // This is waiting on some other concurrency object
        // so for yield live-lock detection it has made progress.
        this.machine.threads.thread_yield_progress();
    }

    #[inline]
    fn unblock_thread(&mut self, thread: ThreadId) {
        let this = self.eval_context_mut();
        this.machine.threads.unblock_thread(thread);
    }

    #[inline]
    fn yield_active_thread(&mut self) {
        let this = self.eval_context_mut();
        this.machine.threads.yield_active_thread();
    }

    #[inline]
    fn register_timeout_callback(
        &mut self,
        thread: ThreadId,
        call_time: Time,
        callback: TimeoutCallback<'mir, 'tcx>,
    ) {
        let this = self.eval_context_mut();
        this.machine.threads.register_timeout_callback(thread, call_time, callback);
    }

    #[inline]
    fn unregister_timeout_callback_if_exists(&mut self, thread: ThreadId) {
        let this = self.eval_context_mut();
        this.machine.threads.unregister_timeout_callback_if_exists(thread);
    }

    /// Execute a timeout callback on the callback's thread.
    #[inline]
    fn run_timeout_callback(&mut self) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let (thread, callback) =
            this.machine.threads.get_ready_callback().expect("no callback found");
        // This back-and-forth with `set_active_thread` is here because of two
        // design decisions:
        // 1. Make the caller and not the callback responsible for changing
        //    thread.
        // 2. Make the scheduler the only place that can change the active
        //    thread.
        let old_thread = this.set_active_thread(thread);
        callback(this)?;
        this.set_active_thread(old_thread);
        Ok(())
    }

    /// Decide which action to take next and on which thread.
    #[inline]
    fn schedule(&mut self) -> InterpResult<'tcx, SchedulingAction> {
        let this = self.eval_context_mut();
        let data_race = &this.memory.extra.data_race;
        this.machine.threads.schedule(data_race)
    }

    /// Handles thread termination of the active thread: wakes up threads joining on this one,
    /// and deallocated thread-local statics.
    ///
    /// This is called from `tls.rs` after handling the TLS dtors.
    #[inline]
    fn thread_terminated(&mut self) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let data_race = &this.memory.extra.data_race;
        for alloc_id in this.machine.threads.thread_terminated(data_race) {
            let ptr = this.memory.global_base_pointer(alloc_id.into())?;
            this.memory.deallocate(ptr, None, MiriMemoryKind::Tls.into())?;
        }
        Ok(())
    }

    /// Called to state that some concurrent operation has occurred
    /// and to invalidate any current live-lock metadata.
    #[inline]
    fn thread_yield_concurrent_progress(&mut self) {
        let this = self.eval_context_mut();
        this.machine.threads.thread_yield_progress();
    }

    /// Mark internal mutex state as modified.
    #[inline]
    fn thread_yield_mutex_wake(&mut self, mutex: MutexId) {
        let this = self.eval_context_mut();
        this.machine.threads.thread_yield_sync_wake(SyncObject::Mutex(mutex));
    }

    /// Mark internal rw-lock state as modified.
    #[inline]
    fn thread_yield_rwlock_wake(&mut self, rwlock: RwLockId) {
        let this = self.eval_context_mut();
        this.machine.threads.thread_yield_sync_wake(SyncObject::RwLock(rwlock));
    }

    /// Mark internal cond-var state as modified.
    #[inline]
    fn thread_yield_condvar_wake(&mut self, condvar: CondvarId) {
        let this = self.eval_context_mut();
        this.machine.threads.thread_yield_sync_wake(SyncObject::Condvar(condvar));
    }

    /// Called when the current thread performs
    /// an atomic operation that may be visible to other threads.
    #[inline]
    fn thread_yield_atomic_wake(&mut self, alloc_id: AllocId, offset: Size, len: Size) {
        let this = self.eval_context_mut();
        this.machine.threads.thread_yield_atomic_wake(alloc_id, offset, len);
    }

    /// Awaken any threads that are yielding on an update to a mutex.
    #[inline]
    fn thread_yield_mutex_watch(&mut self, mutex: MutexId) {
        let this = self.eval_context_mut();
        this.machine.threads.thread_yield_sync_watch(SyncObject::Mutex(mutex));
    }

    /// Awaken any threads that are yielding on an update to a rwlock.
    #[inline]
    fn thread_yield_rwlock_watch(&mut self, rwlock: RwLockId) {
        let this = self.eval_context_mut();
        this.machine.threads.thread_yield_sync_watch(SyncObject::RwLock(rwlock));
    }

    /// Awaken any threads that are yielding on an update to a condvar.
    #[inline]
    fn thread_yield_condvar_watch(&mut self, condvar: CondvarId) {
        let this = self.eval_context_mut();
        this.machine.threads.thread_yield_sync_watch(SyncObject::Condvar(condvar));
    }

    /// Called when the current thread performs
    /// an atomic read operation and may want
    /// to mark that variable as watched to wake
    /// the current yield.
    #[inline]
    fn thread_yield_atomic_watch(&mut self, alloc_id: AllocId, alloc_size: Size, offset: Size, len: Size) {
        let this = self.eval_context_mut();
        this.machine.threads.thread_yield_atomic_watch(alloc_id, alloc_size, offset, len);
    }
}
