use std::cell::RefMut;
use std::collections::BTreeMap;
use std::io;
use std::ops::BitOrAssign;
use std::time::Duration;

use mio::event::Source;
use mio::{Events, Interest, Poll, Token};

use crate::shims::{EpollEvalContextExt, FdId, FileDescription, FileDescriptionRef};
use crate::*;

/// Capacity of the event queue which can be polled at a time.
/// Since we don't expect many simultaneous blocking I/O events
/// this value can be set rather low.
const IO_EVENT_CAPACITY: usize = 16;

/// Trait for file descriptions that contain a mio [`Source`].
pub trait SourceFileDescription: FileDescription {
    /// Invoke `f` on the source inside `self`.
    fn with_source(&self, f: &mut dyn FnMut(&mut dyn Source) -> io::Result<()>) -> io::Result<()>;

    /// Get a mutable reference to the readiness of the source.
    fn get_readiness_mut(&self) -> RefMut<'_, BlockingIoSourceReadiness>;
}

/// An interest receiver defines the action that should be taken when
/// the associated [`Interest`] is fulfilled.
#[derive(Debug, PartialEq, Clone, Copy, Eq, PartialOrd, Ord)]
pub enum InterestReceiver {
    /// The specified thread should be unblocked.
    UnblockThread(ThreadId),
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

/// Struct reflecting the readiness of a source file description.
#[derive(Debug)]
pub struct BlockingIoSourceReadiness {
    /// Boolean whether the source is currently readable.
    pub readable: bool,
    /// Boolean whether the source is currently writable.
    pub writable: bool,
    /// Boolean whether the read end of the source has been
    /// closed.
    pub read_closed: bool,
    /// Boolean whether the write end of the source has been
    /// closed.
    pub write_closed: bool,
    /// Boolean whether the source currently has an error.
    pub error: bool,
}

impl BlockingIoSourceReadiness {
    pub fn empty() -> Self {
        Self {
            readable: false,
            writable: false,
            read_closed: false,
            write_closed: false,
            error: false,
        }
    }

    pub fn fulfills_interest(&self, interest: &BlockingIoInterest) -> bool {
        match interest {
            BlockingIoInterest::Read => self.readable || self.error,
            BlockingIoInterest::Write => self.writable || self.error,
            BlockingIoInterest::ReadWrite => self.readable || self.writable || self.error,
        }
    }
}

impl BitOrAssign for BlockingIoSourceReadiness {
    fn bitor_assign(&mut self, rhs: Self) {
        self.readable |= rhs.readable;
        self.writable |= rhs.writable;
        self.read_closed |= rhs.read_closed;
        self.write_closed |= rhs.write_closed;
        self.error |= rhs.error;
    }
}

impl From<&mio::event::Event> for BlockingIoSourceReadiness {
    fn from(event: &mio::event::Event) -> Self {
        Self {
            readable: event.is_readable(),
            writable: event.is_writable(),
            read_closed: event.is_read_closed(),
            write_closed: event.is_write_closed(),
            error: event.is_error(),
        }
    }
}

struct BlockingIoSource {
    /// The source file description which is registered into the poll.
    fd: FileDescriptionRef<dyn SourceFileDescription>,
    /// The registered receivers for this file description.
    receivers: BTreeMap<InterestReceiver, BlockingIoInterest>,
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
    /// registered receivers.
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
    /// Returns the list of [`InterestReceiver`]s whose interests are currently fulfilled together with
    /// the file description they're for. Note that the events are returned in a level-triggered way,
    /// which means that [`InterestReceiver`]s whose interests were fulfilled before the poll will be
    /// returned again.
    pub fn poll<'tcx>(
        ecx: &mut MiriInterpCx<'tcx>,
        timeout: Option<Duration>,
    ) -> InterpResult<'tcx, Result<Vec<InterestReceiver>, io::Error>> {
        let poll = ecx
            .machine
            .blocking_io
            .poll
            .as_mut()
            .expect("Blocking I/O should not be called with isolation enabled");

        // Poll for new I/O events from OS and store them in the events buffer.
        if let Err(err) = poll.poll(&mut ecx.machine.blocking_io.events, timeout) {
            return interp_ok(Err(err));
        };

        let event_fds = ecx
            .machine
            .blocking_io
            .events
            .iter()
            .map(|event| {
                let token = event.token();
                // We know all tokens are valid `FdId`.
                let fd_id = FdId::new_unchecked(token.0);
                let source = ecx
                    .machine
                    .blocking_io
                    .sources
                    .get(&fd_id)
                    .expect("Source should be registered");
                let fd = source.fd.clone();

                assert_eq!(fd.id(), fd_id);
                // Update the readiness of the source.
                *fd.get_readiness_mut() |= BlockingIoSourceReadiness::from(event);
                fd
            })
            .collect::<Vec<_>>();

        for fd in event_fds.into_iter() {
            ecx.update_epoll_active_events(fd, false)?;
        }

        // List containing all receivers for all registered sources whose interests are
        // currently fulfilled. This also includes receivers for sources which didn't
        // receive an event from the current poll invocation.
        let ready = ecx
            .machine
            .blocking_io
            .sources
            .values()
            .flat_map(|source| {
                source
                    .receivers
                    .iter()
                    .filter_map(|(key, interest)| {
                        source.fd.get_readiness_mut().fulfills_interest(interest).then_some(key)
                    })
                    .copied()
            })
            .collect::<Vec<_>>();

        interp_ok(Ok(ready))
    }

    /// Get whether a source file description is currently registered in the
    /// blocking I/O poll.
    /// This can also be used to check whether a file description is a host
    /// I/O source.
    pub fn contains_source(&self, source_id: &FdId) -> bool {
        self.sources.contains_key(source_id)
    }

    /// Register a source file description to the blocking I/O poll.
    pub fn register(&mut self, source_fd: FileDescriptionRef<dyn SourceFileDescription>) {
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

        let source = BlockingIoSource { fd: source_fd, receivers: BTreeMap::default() };

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
        source_id: FdId,
        interest: BlockingIoInterest,
        timeout: Option<(TimeoutClock, TimeoutAnchor, Duration)>,
        callback: DynUnblockCallback<'tcx>,
    ) {
        let this = self.eval_context_mut();
        this.machine.blocking_io.add_receiver(
            source_id,
            InterestReceiver::UnblockThread(this.machine.threads.active_thread()),
            interest,
        );
        this.block_thread(BlockReason::IO, timeout, callback);
    }
}
