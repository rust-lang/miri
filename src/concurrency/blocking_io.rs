use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;
use std::time::Duration;

use mio::event::Source;
use mio::{Events, Interest, Poll, Token};
use rustc_data_structures::fx::FxHashMap;

use crate::*;

/// Capacity of the event queue which can be polled at a time.
/// Since we don't expect many simultaneous blocking I/O events
/// this value can be set rather low.
const IO_EVENT_CAPACITY: usize = 16;

// Supertrait to enforce that all sources implement [`Source`] as
// well as [`VisitProvenance`].
pub trait SourceRefExt: Source + VisitProvenance {}
impl<T> SourceRefExt for T where T: Source + VisitProvenance {}
type SourceRef = Rc<RefCell<dyn SourceRefExt>>;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
/// Types of I/O a thread can be blocked on.
pub enum BlockingIoKind {
    /// Attempting to accept an incoming TCP connection.
    TcpAccept,
}

/// Manager for managing blocking host I/O in a non-blocking manner.
/// We use [`Poll`] to poll for new I/O events from the OS for sources
/// registered using this manager.
///
/// Since blocking host I/O is inherently non-deterministic, no method on this
/// manager should be called when isolation is enabled. The only exception is
/// the [`BlockingIoManager::new`] function to create the manager. Everywhere else,
/// we assert that isolation is disabled!
pub struct BlockingIoManager {
    /// Poll instance to monitor I/O events from the OS.
    /// This is only [`None`] when Miri is run with isolation enabled.
    poll: Option<Poll>,
    /// Buffer used to store the ready I/O events when calling [`Poll::poll`].
    events: Events,
    /// Map between threads which are currently blocked, the kind of I/O
    /// they are blocked on and the underlying I/O source.
    sources: FxHashMap<ThreadId, (BlockingIoKind, SourceRef)>,
    /// List of threads which are ready to be unblocked together with the I/O kind
    /// they were blocked on.
    ready: VecDeque<(ThreadId, BlockingIoKind)>,
}

impl BlockingIoManager {
    /// Create a new blocking I/O manager instance based on the availability
    /// of communication with the host.
    pub fn new(communicate: bool) -> Result<Self, io::Error> {
        let manager = Self {
            poll: communicate.then_some(Poll::new()?),
            events: Events::with_capacity(IO_EVENT_CAPACITY),
            sources: FxHashMap::default(),
            ready: VecDeque::new(),
        };
        Ok(manager)
    }

    /// Poll for new I/O events from the OS or wait until the timeout expired. This method
    /// marks all threads which received an I/O event as ready. Those threads can then
    /// be unblocked using the [`EvalContextExt::unblock_next_ready_io_thread`] method.
    ///
    /// - If the timeout is [`Some`] and contains [`Duration::ZERO`], the poll doesn't block and just
    ///   reads all events since the last poll.
    /// - If the timeout is [`Some`] and contains a non-zero duration, it blocks at most for the
    ///   specified duration.
    /// - If the timeout is [`None`] the poll blocks indefinitely until an event occurs.
    ///
    /// Returns the total amount of threads ready to be unblocked, including ones which were already
    /// ready before the poll.
    pub fn poll(&mut self, timeout: Option<Duration>) -> Result<usize, io::Error> {
        let poll =
            self.poll.as_mut().expect("Blocking I/O should not be called with isolation enabled");

        // Poll for new I/O events from OS and store them in the events buffer.
        poll.poll(&mut self.events, timeout)?;

        // We need to clone the iterator here since it holds an immutable reference to `self.events`.
        // This doesn't work out since we need a mutable self-reference inside the loop body.
        let events = self.events.iter().cloned().collect::<Vec<_>>();
        for event in events {
            let token = event.token();
            // It's safe to convert the token identifier back to an u32
            // since we only create tokens from thread id's which are u32.
            #[expect(clippy::as_conversions)]
            let thread = ThreadId::new_unchecked(token.0 as u32);

            // Deregister this source as we only want to receive one event per thread.
            let kind = self.deregister(thread);
            // Add thread to the ready list such that it can be unblocked.
            self.ready.push_back((thread, kind));
        }

        Ok(self.ready.len())
    }

    /// Get the next thread from the ready list. If the list is empty, [`None`] is returned.
    pub fn get_next_ready(&mut self) -> Option<(ThreadId, BlockingIoKind)> {
        self.ready.pop_front()
    }

    /// Get the amount of threads ready to be unblocked.
    pub fn get_ready_count(&self) -> usize {
        self.ready.len()
    }

    /// Register a blocking I/O source for a thread together with it's poll interests.
    ///
    /// The source will be deregistered automatically once an event for it is received.
    ///
    /// As the OS can always produce spurious wake-ups, it's the callers responsibility to
    /// verify the requested I/O interests are really ready and to register again if they're not.
    pub fn register(
        &mut self,
        kind: BlockingIoKind,
        source: SourceRef,
        thread: ThreadId,
        interests: Interest,
    ) {
        let poll =
            self.poll.as_ref().expect("Blocking I/O should not be called with isolation enabled");

        #[allow(clippy::as_conversions)]
        let token = Token(thread.to_u32() as usize);

        assert!(
            !self.sources.contains_key(&thread),
            "A thread cannot be registered twice at the same time"
        );

        // Treat errors from registering as fatal. On UNIX hosts this can only
        // fail due to system resource errors (e.g. ENOMEM or ENOSPC).
        poll.registry().register(&mut *source.borrow_mut(), token, interests).unwrap();
        self.sources.insert(thread, (kind, source));
    }

    /// Deregister the event source for a thread. Returns the kind of I/O the thread was
    /// blocked on.
    fn deregister(&mut self, thread: ThreadId) -> BlockingIoKind {
        let poll =
            self.poll.as_ref().expect("Blocking I/O should not be called with isolation enabled");

        let Some((kind, source)) = self.sources.remove(&thread) else {
            panic!("Attempt to deregister a token which isn't registered")
        };

        // Treat errors from deregistering as fatal. On UNIX hosts this can only
        // fail due to system resource errors (e.g. ENOMEM or ENOSPC).
        poll.registry().deregister(&mut *source.borrow_mut()).unwrap();

        kind
    }
}

impl<'tcx> EvalContextExt<'tcx> for MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: MiriInterpCxExt<'tcx> {
    /// Block the current thread until some interests on an I/O source
    /// are fulfilled or the optional timeout exceeded.
    /// The callback will be invoked when the thread gets unblocked.
    ///
    /// There can be spurious wake-ups by the OS and thus it's the callers
    /// responsibility to verify that the requested I/O interests are
    /// really ready and to block again if they're not.
    #[inline]
    fn block_thread_for_io(
        &mut self,
        kind: BlockingIoKind,
        source: SourceRef,
        interests: Interest,
        timeout: Option<(TimeoutClock, TimeoutAnchor, Duration)>,
        callback: DynUnblockCallback<'tcx>,
    ) {
        let this = self.eval_context_mut();
        this.machine.blocking_io.register(
            kind,
            source,
            this.machine.threads.active_thread(),
            interests,
        );
        this.block_thread(BlockReason::IO { kind }, timeout, callback);
    }

    /// Unblock the next ready thread which was blocked for I/O.
    /// Returns [`None`] if there is no thread ready to be unblocked.
    fn unblock_next_ready_io_thread(&mut self) -> Option<InterpResult<'tcx>> {
        let this = self.eval_context_mut();
        let (thread, kind) = this.machine.blocking_io.get_next_ready()?;
        Some(this.unblock_thread(thread, BlockReason::IO { kind }))
    }
}

impl VisitProvenance for BlockingIoManager {
    fn visit_provenance(&self, visit: &mut VisitWith<'_>) {
        self.sources.iter().for_each(|(thread_id, (_, source))| {
            thread_id.visit_provenance(visit);
            source.borrow().visit_provenance(visit);
        });
        self.ready.iter().for_each(|(thread_id, _)| thread_id.visit_provenance(visit));
    }
}
