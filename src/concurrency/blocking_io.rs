use std::collections::VecDeque;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::time::Duration;

use mio::event::Source;
use mio::{Events, Interest, Poll, Token};
use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use rustc_index::Idx;

use crate::*;

/// Capacity of the event queue which can be polled at a time.
/// Since we don't expect many simultaneous blocking I/O events
/// this value can be set rather low.
const IO_EVENT_CAPACITY: usize = 16;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BlockingIoKind {
    /// Attempting to accept an incoming TCP connection.
    TcpAccept,
}

/// Manager for managing blocking host I/O in a non-blocking manner.
/// We use [`Poll`] to poll for new I/O events from the OS for sources
/// registered using this manager.
///
/// Since blocking host I/O is inherently non-deterministic, no method on this
/// manager should be called when isolation mode is enabled. The only exception is
/// the [`BlockingIoManager::new`] function to create the manager. Everywhere else
/// we assert that isolation mode is disabled!
pub struct BlockingIoManager {
    /// Poll instance to monitor I/O events from the OS.
    /// This is only [`None`] when Miri is run with isolation mode enabled.
    poll: Option<Poll>,
    /// Buffer used to store the ready I/O events when calling [`Poll::poll`].
    events: Events,
    /// Map between threads which are currently blocked, the kind of I/O
    /// they are blocked on and the underlying I/O source.
    sources: FxHashMap<ThreadId, (BlockingIoKind, BlockingIoSourceFd)>,
    /// Set of threads for which we ignore the events. Those threads are only still
    /// registered since deregistering them failed.
    ignored: FxHashSet<ThreadId>,
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
            ignored: FxHashSet::default(),
            ready: VecDeque::new(),
        };
        Ok(manager)
    }

    /// Poll for new I/O events from the OS. This method marks all threads which received
    /// an I/O event as ready. Those threads can then be unblocked using the [`unblock_next_ready`]
    /// method.
    /// If duration is [`Duration::ZERO`] the poll doesn't block and just reads all events
    /// since the last poll.
    /// Returns the total amount of threads ready to be unblocked, including ones which were already
    /// ready before the poll.
    pub fn poll(&mut self, duration: Duration) -> Result<usize, io::Error> {
        let poll = self
            .poll
            .as_mut()
            .expect("Blocking I/O should not be called with isolation mode enabled");

        // Poll for new I/O events from OS.
        poll.poll(&mut self.events, Some(duration))?;

        // We need to clone the iterator here since it holds an immutable reference to `self.events`.
        // This doesn't work out since we need a mutable self-reference inside the loop body.
        let events = self.events.iter().cloned().collect::<Vec<_>>();
        for event in events {
            let token = event.token();
            let thread = ThreadId::new(token.0);
            let is_ignored = self.ignored.contains(&thread);

            // Deregister this source as we only want to receive one event per token.
            match self.deregister(thread) {
                // Ignore the event as the thread was already ignored before.
                Ok(_) if is_ignored => {
                    // Ensure thread is no longer part of the ignored list.
                    self.ignored.remove(&thread);
                }
                // Ignore the event as the thread was already ignored before.
                // FIXME: What do we do with this error?
                _ if is_ignored => {}
                // Add thread to the ready list such that it can be unblocked.
                Ok(kind) => self.ready.push_back((thread, kind)),
                Err(_err) => {
                    // FIXME: What do we do with this error?

                    // Ignore future events for this thread.
                    self.ignored.insert(thread);

                    // We still want to unblock the thread now and deal
                    // with deregistering it again on it's next event.
                    if let Some((kind, _)) = self.sources.get(&thread) {
                        self.ready.push_back((thread, *kind));
                    }
                }
            };
        }

        Ok(self.ready.len())
    }

    /// Get the next thread from the ready list. If the list is empty [`None`] is returned.
    pub fn get_next_ready(&mut self) -> Option<(ThreadId, BlockingIoKind)> {
        self.ready.pop_front()
    }

    /// Register a blocking I/O source for a thread together with it's poll interests.
    ///
    /// The source will be deregistered automatically once an event for it is received.
    ///
    /// As the OS can always produce spurious wake-ups, it's the callers responsibility to
    /// verify the requested I/O operation is really ready and to register again if it's not.
    pub fn register(
        &mut self,
        kind: BlockingIoKind,
        mut source: BlockingIoSourceFd,
        thread: ThreadId,
        interests: Interest,
    ) -> Result<(), io::Error> {
        let poll = self
            .poll
            .as_ref()
            .expect("Blocking I/O should not be called with isolation mode enabled");

        #[allow(clippy::as_conversions)]
        let token = Token(thread.to_u32() as usize);

        if self.sources.contains_key(&thread) && self.ignored.contains(&thread) {
            // This thread should've already been deregistered and is thus ignored.
            // We can now attempt to re-register it with it's new interests.
            poll.registry().reregister(&mut source, token, interests)?;
            self.ignored.remove(&thread);
        } else {
            assert!(
                !self.sources.contains_key(&thread),
                "A thread cannot be registered twice at the same time"
            );

            poll.registry().register(&mut source, token, interests)?;
            self.sources.insert(thread, (kind, source));
        }

        Ok(())
    }

    /// Deregister the event source for a thread. Returns the kind of I/O the thread was
    /// blocked on.
    fn deregister(&mut self, thread: ThreadId) -> Result<BlockingIoKind, io::Error> {
        let poll = self
            .poll
            .as_ref()
            .expect("Blocking I/O should not be called with isolation mode enabled");

        let Some((kind, mut source)) = self.sources.remove(&thread) else {
            panic!("Attempt to deregister a token which isn't registered")
        };

        if let Err(err) = poll.registry().deregister(&mut source) {
            // Re-insert source as we weren't able to deregister it.
            self.sources.insert(thread, (kind, source));
            Err(err)?;
        }

        Ok(kind)
    }
}

impl<'tcx> EvalContextExt<'tcx> for MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: MiriInterpCxExt<'tcx> {
    /// Block the current thread until some interests on an I/O source
    /// are fulfilled or the optional timeout exceeded.
    /// The callback will be invoked when the thread gets unblocked.
    #[inline]
    fn block_thread_for_io(
        &mut self,
        kind: BlockingIoKind,
        source: impl AsBlockingIoSourceFd,
        interests: Interest,
        timeout: Option<(TimeoutClock, TimeoutAnchor, Duration)>,
        callback: DynUnblockCallback<'tcx>,
    ) -> Result<(), io::Error> {
        let this = self.eval_context_mut();
        this.machine.blocking_io.register(
            kind,
            source.as_source_fd(),
            this.machine.threads.active_thread(),
            interests,
        )?;
        this.block_thread(BlockReason::IO { kind }, timeout, callback);
        Ok(())
    }

    /// Unblock the next ready thread which was blocked for I/O.
    /// Returns [`None`] if there is no thread ready to be unblocked.
    fn unblock_next_ready_io_thread(&mut self) -> Option<InterpResult<'tcx>> {
        let this = self.eval_context_mut();
        let (thread, kind) = this.machine.blocking_io.get_next_ready()?;
        Some(this.unblock_thread(thread, BlockReason::IO { kind }))
    }
}

/// File descriptor of a blocking I/O source living on the heap.
pub struct BlockingIoSourceFd(Box<RawFd>);

pub trait AsBlockingIoSourceFd {
    /// Get a file descriptor for a blocking I/O source.
    fn as_source_fd(&self) -> BlockingIoSourceFd;
}

// Every RawFd can be turned into a BlockingIoSourceFd.
impl<T> AsBlockingIoSourceFd for &T
where
    T: AsRawFd,
{
    fn as_source_fd(&self) -> BlockingIoSourceFd {
        BlockingIoSourceFd(Box::new(self.as_raw_fd()))
    }
}

// On UNIX targets we can implement [`mio::event::Source`] for every [`AsBlockingIoSourceFd`]
// since the UNIX OS interfaces allow polling any file descriptor.
#[cfg(unix)]
impl Source for BlockingIoSourceFd {
    fn register(
        &mut self,
        registry: &mio::Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        use mio::unix::SourceFd;
        let mut sourcefd = SourceFd(&self.0);
        registry.register(&mut sourcefd, token, interests)
    }

    fn reregister(
        &mut self,
        registry: &mio::Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        use mio::unix::SourceFd;
        let mut sourcefd = SourceFd(&self.0);
        registry.reregister(&mut sourcefd, token, interests)
    }

    fn deregister(&mut self, registry: &mio::Registry) -> io::Result<()> {
        use mio::unix::SourceFd;
        let mut sourcefd = SourceFd(&self.0);
        registry.deregister(&mut sourcefd)
    }
}
