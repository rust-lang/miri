use std::cell::RefCell;
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

pub struct BlockingIoManager {
    /// Poll instance to monitor I/O events from the OS.
    poll: RefCell<Poll>,
    /// Buffer used to store the ready I/O events when calling [`Poll::poll`].
    events: RefCell<Events>,
    /// Map between threads which are currently blocked, the kind of I/O
    /// they are blocked on and the underlying I/O source.
    sources: RefCell<FxHashMap<ThreadId, (BlockingIoKind, BlockingIoSourceFd)>>,
    /// Set of threads for which we ignore the events. Those threads are only still
    /// registered since deregistering them failed.
    ignored: RefCell<FxHashSet<ThreadId>>,
    /// List of threads which are ready to be unblocked together with the I/O kind
    /// they were blocked for.
    ready: RefCell<VecDeque<(ThreadId, BlockingIoKind)>>,
}

impl BlockingIoManager {
    pub fn new() -> Result<Self, io::Error> {
        let manager = Self {
            poll: RefCell::new(Poll::new()?),
            events: RefCell::new(Events::with_capacity(IO_EVENT_CAPACITY)),
            sources: RefCell::new(FxHashMap::default()),
            ignored: RefCell::new(FxHashSet::default()),
            ready: RefCell::new(VecDeque::new()),
        };
        Ok(manager)
    }

    /// Non-blockingly poll for I/O events. This method marks all threads which received
    /// an I/O event as ready. Those threads can then be unblocked using the [`unblock_next_ready`]
    /// method.
    /// Returns the amount of threads ready to be unblocked.
    pub fn poll(&self, duration: Duration) -> Result<usize, io::Error> {
        let mut events = self.events.borrow_mut();
        self.poll.borrow_mut().poll(&mut events, Some(duration))?;

        let mut ignored = self.ignored.borrow_mut();
        let mut ready = self.ready.borrow_mut();
        events.iter().for_each(|event| {
            let token = event.token();
            let thread = ThreadId::new(token.0);
            let is_ignored = ignored.contains(&thread);

            // Deregister this source as we only want to receive one event per token.
            match self.deregister(thread) {
                // Ignore the event as the thread was already ignored before.
                Ok(_) if is_ignored => {
                    // Ensure thread is no longer part of the ignored list.
                    ignored.remove(&thread);
                }
                // Ignore the event as the thread was already ignored before.
                // FIXME: What do we do with this error?
                _ if is_ignored => {}
                // Add thread to the ready list such that it can be unblocked.
                Ok(kind) => ready.push_back((thread, kind)),
                Err(_err) => {
                    // FIXME: What do we do with this error?

                    // Ignore future events for this thread.
                    ignored.insert(thread);

                    // We still want to unblock the thread now and deal
                    // with deregistering it again on it's next event.
                    if let Some((kind, _)) = self.sources.borrow().get(&thread) {
                        ready.push_back((thread, *kind));
                    }
                }
            };
        });

        Ok(ready.len())
    }

    /// Get the next thread from the ready list. If the list is empty [`None`] is returned.
    pub fn get_next_ready(&self) -> Option<(ThreadId, BlockingIoKind)> {
        let mut ready = self.ready.borrow_mut();
        ready.pop_front()
    }

    /// Register a blocking I/O source for a thread together with it's poll interests.
    ///
    /// The source will be deregistered automatically once an event for it is received.
    ///
    /// As the OS can always produce spurious wake-ups, it's the callers responsibility to
    /// verify the requested I/O operation is really ready and to register again if it's not.
    pub fn register(
        &self,
        kind: BlockingIoKind,
        mut source: BlockingIoSourceFd,
        thread: ThreadId,
        interests: Interest,
    ) -> Result<(), io::Error> {
        #[allow(clippy::as_conversions)]
        let token = Token(thread.to_u32() as usize);
        let mut sources = self.sources.borrow_mut();
        let mut ignored = self.ignored.borrow_mut();

        if sources.contains_key(&thread) && ignored.contains(&thread) {
            // This thread should've already been deregistered and is thus ignored.
            // We can now attempt to re-register it with it's new interests.
            self.poll.borrow().registry().reregister(&mut source, token, interests)?;
            ignored.remove(&thread);
        } else {
            assert!(
                !sources.contains_key(&thread),
                "A thread cannot be registered twice at the same time"
            );

            self.poll.borrow().registry().register(&mut source, token, interests)?;
            sources.insert(thread, (kind, source));
        }

        Ok(())
    }

    /// Deregister the event source for a thread. Returns the kind of I/O the thread was
    /// blocked on.
    fn deregister(&self, thread: ThreadId) -> Result<BlockingIoKind, io::Error> {
        let mut sources = self.sources.borrow_mut();
        let Some((kind, mut source)) = sources.remove(&thread) else {
            panic!("Attempt to deregister a token which isn't registered")
        };

        if let Err(err) = self.poll.borrow().registry().deregister(&mut source) {
            // Re-insert source as we weren't able to deregister it.
            sources.insert(thread, (kind, source));
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
