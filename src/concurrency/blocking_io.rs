use std::collections::BTreeMap;
use std::io;
use std::time::Duration;

use mio::event::Source;
use mio::{Events, Interest, Poll, Token};

use crate::shims::{EpollEventKey, EpollEvents, FdId, FileDescription, FileDescriptionRef};
use crate::*;

/// Capacity of the event queue which can be polled at a time.
/// Since we don't expect many simultaneous blocking I/O events
/// this value can be set rather low.
const IO_EVENT_CAPACITY: usize = 16;

/// Trait for file descriptions that contain a mio [`Source`].
pub trait WithSource: FileDescription {
    /// Invoke `f` on the source inside `self`.
    fn with_source(&self, f: &mut dyn FnMut(&mut dyn Source) -> io::Result<()>) -> io::Result<()>;
}

/// An interest receiver defines the action that should be taken when
/// the associated [`Interest`] is fulfilled.
#[derive(Debug, PartialEq, Clone, Copy, Eq, PartialOrd, Ord)]
pub enum InterestReceiver {
    /// The specified thread should be unblocked.
    UnblockThread(ThreadId),
    /// The file descriptor for the file description in `epoll_key` received
    /// an epoll event on the epoll instance with id `epfd_id`.
    Epoll { epfd_id: FdId, epoll_key: EpollEventKey },
}

/// An I/O interest for an [`InterestReceiver`].
/// For all variants the interest in error events is implicitly added
/// to the specified interests!
#[derive(Debug)]
pub enum BlockingIoInterest {
    /// The receiver is interested in [`Interest::READABLE`].
    Read,
    /// The receiver is interested in [`Interest::WRITABLE`].
    Write,
    /// The receiver is interested in [`Interest::READABLE`] and
    /// [`Interest::WRITABLE`].
    ReadWrite,
}

struct BlockingIoSource {
    /// The source file description which is registered into the poll.
    fd: FileDescriptionRef<dyn WithSource>,
    /// The registered receivers for this file description.
    receivers: BTreeMap<InterestReceiver, BlockingIoInterest>,
    /// The current readiness of the file description.
    /// It's the file descriptions responsibility to set the
    /// readiness fields to `false` once they no longer hold
    /// (e.g. `epollin` should be set to `false` once an
    /// EWOULDBLOCK is returned when attempting to read)
    readiness: EpollEvents,
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
    /// This is not part of the state and only stored to avoid allocating a
    /// new buffer for every poll.
    events: Events,
    /// Map from source file description ids to the actual sources with their
    /// registered receivers and their current readiness.
    sources: BTreeMap<FdId, BlockingIoSource>,
}

impl BlockingIoManager {
    /// Create a new blocking I/O manager instance based on the availability
    /// of communication with the host.
    pub fn new(communicate: bool) -> Result<Self, io::Error> {
        let manager = Self {
            poll: communicate.then_some(Poll::new()?),
            events: Events::with_capacity(IO_EVENT_CAPACITY),
            sources: BTreeMap::default(),
        };
        Ok(manager)
    }

    /// Poll for new I/O events from the OS or wait until the timeout expired.
    ///
    /// - If the timeout is [`Some`] and contains [`Duration::ZERO`], the poll doesn't block and just
    ///   reads all events since the last poll.
    /// - If the timeout is [`Some`] and contains a non-zero duration, it blocks at most for the
    ///   specified duration.
    /// - If the timeout is [`None`] the poll blocks indefinitely until an event occurs.
    ///
    /// Returns the interest receivers whose events are currently fulfilled together with the file description
    /// they were registered for.
    pub fn poll(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<Vec<(InterestReceiver, FileDescriptionRef<dyn WithSource>)>, io::Error> {
        let poll =
            self.poll.as_mut().expect("Blocking I/O should not be called with isolation enabled");

        // Poll for new I/O events from OS and store them in the events buffer.
        poll.poll(&mut self.events, timeout)?;

        self.events.iter().for_each(|event| {
            let token = event.token();
            // We know all tokens are valid `FdId`.
            let fd_id = FdId::new_unchecked(token.0);
            let source = self.sources.get_mut(&fd_id).expect("Source should be registered");
            let fd = source.fd.clone();

            assert_eq!(fd.id(), fd_id);

            // Best-effort mapping from cross platform mio event readiness to epoll readiness.
            let new_readiness = EpollEvents {
                epollin: event.is_readable(),
                epollout: event.is_writable(),
                epollrdhup: event.is_read_closed(),
                epollhup: event.is_write_closed(),
                epollerr: event.is_error(),
            };

            // Update the readiness of the source with new readiness data from the event.
            source.readiness.epollerr |= new_readiness.epollerr;
            source.readiness.epollhup |= new_readiness.epollhup;
            source.readiness.epollrdhup |= new_readiness.epollrdhup;
            source.readiness.epollin |= new_readiness.epollin;
            source.readiness.epollout |= new_readiness.epollout;
        });

        // List containing all receivers for all registers sources whose interests are
        // currently fulfilled. This also includes receivers for sources which didn't
        // receive an event from the current poll invocation.
        let ready = self
            .sources
            .values()
            .flat_map(|source| {
                source
                    .receivers
                    .iter()
                    .filter_map(|(key, interest)| {
                        source.readiness.fulfills_interest(interest).then_some(key)
                    })
                    .copied()
                    .map(|receiver| (receiver, source.fd.clone()))
            })
            .collect::<Vec<_>>();

        Ok(ready)
    }

    /// Get whether a source file description is currently registered in the
    /// blocking I/O poll.
    pub fn contains_source(&self, source_id: &FdId) -> bool {
        self.sources.contains_key(source_id)
    }

    /// Register a source file description to the blocking I/O poll.
    pub fn register(&mut self, source_fd: FileDescriptionRef<dyn WithSource>) {
        let poll =
            self.poll.as_ref().expect("Blocking I/O should not be called with isolation enabled");

        let id = source_fd.id();
        let token = Token(id.to_usize());

        // All possible interests.
        // We only care about the readable and writable interests because those are the only
        // interests which are available on all platforms. Internally, mio also
        // registers an error interest.
        let interest = Interest::READABLE | Interest::WRITABLE;

        // Treat errors from registering as fatal. On UNIX hosts this can only
        // fail due to system resource errors (e.g. ENOMEM or ENOSPC) or when the source is already registered.
        source_fd
            .with_source(&mut |source| poll.registry().register(source, token, interest))
            .unwrap();

        let source = BlockingIoSource {
            fd: source_fd,
            readiness: EpollEvents::new(),
            receivers: BTreeMap::default(),
        };

        self.sources
            .try_insert(id, source)
            .unwrap_or_else(|_| panic!("Source should not already be registered"));
    }

    /// Deregister a source file description from the blocking I/O poll.
    pub fn deregister(&mut self, source_id: FdId) {
        let poll =
            self.poll.as_ref().expect("Blocking I/O should not be called with isolation enabled");

        let source = self.sources.remove(&source_id).expect("Source should be registered");

        // Treat errors from deregistering as fatal. On UNIX hosts this can only
        // fail due to system resource errors (e.g. ENOMEM or ENOSPC).
        source.fd.with_source(&mut |source| poll.registry().deregister(source)).unwrap();
    }

    /// Add a new receiver to a registered source.
    ///
    /// As the OS can always produce spurious wake-ups, it's the callers responsibility to
    /// verify the requested I/O interests are really fulfilled when an event for this
    /// receiver is returned from [`BlockingIoManager::poll`].
    ///
    /// It's assumed that the source with id `source_id` is currently registered and that
    /// it doesn't already have the same [`InterestReceiver`] as `receiver` added.
    pub fn add_receiver(
        &mut self,
        source_id: FdId,
        receiver: InterestReceiver,
        interest: BlockingIoInterest,
    ) {
        let source = self.sources.get_mut(&source_id).expect("Source should be registered");

        source
            .receivers
            .try_insert(receiver, interest)
            .expect("Receiver should not already exist for source");
    }

    /// Remove a receiver from a registered source.
    ///
    /// It's assumed that the source with id `source_id` is currently registered and that
    /// the specified receiver exists for this source.
    pub fn remove_receiver(&mut self, source_id: FdId, receiver: InterestReceiver) {
        let source = self.sources.get_mut(&source_id).expect("Source should be registered");
        source.receivers.remove(&receiver).expect("Receiver should exist for source");
    }

    /// Get a reference to the current readiness for a registered source.
    ///
    /// It's assumed that the source with id `source_id` is currently registered.
    pub fn get_source_readiness(&self, source_id: FdId) -> &EpollEvents {
        let source = self.sources.get(&source_id).expect("Source should be registered");
        &source.readiness
    }

    /// Get a mutable reference to the current readiness for a registered source.
    ///
    /// It's assumed that the source with id `source_id` is currently registered.
    pub fn get_source_readiness_mut(&mut self, source_id: FdId) -> &mut EpollEvents {
        let source = self.sources.get_mut(&source_id).expect("Source should be registered");
        &mut source.readiness
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
        source_fd: FileDescriptionRef<dyn WithSource>,
        interest: BlockingIoInterest,
        timeout: Option<(TimeoutClock, TimeoutAnchor, Duration)>,
        callback: DynUnblockCallback<'tcx>,
    ) {
        let this = self.eval_context_mut();
        this.machine.blocking_io.add_receiver(
            source_fd.id(),
            InterestReceiver::UnblockThread(this.machine.threads.active_thread()),
            interest,
        );
        this.block_thread(BlockReason::IO, timeout, callback);
    }
}
