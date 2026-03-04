use std::cell::RefCell;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::time::Duration;

use mio::event::{Event, Source};
use mio::{Events, Interest, Poll, Token};
use rustc_data_structures::fx::{FxHashMap, FxHashSet};

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
    /// Handle to a list of ready I/O events.
    ///
    /// This is a vector under the hood which stores all events returned from
    /// a call to [`mio::Poll::poll`]. This vector is cleared when calling
    /// [`mio::Poll::poll`] again.
    events: RefCell<Events>,
    /// List of events returned by the last [`mio::Poll::poll`] which haven't
    /// yet been handled.
    unhandled: RefCell<Vec<Event>>,
    /// Map between tokens (threads) which are currently blocked and the underlying
    /// I/O sources.
    sources: RefCell<FxHashMap<Token, BlockingIoSourceFd>>,
    /// Set of tokens for which we ignore the events. Those tokens are only still
    /// registered since deregistering them failed.
    ignored: RefCell<FxHashSet<Token>>,
}

impl BlockingIoManager {
    pub fn new() -> Result<Self, io::Error> {
        let manager = Self {
            poll: RefCell::new(Poll::new()?),
            events: RefCell::new(Events::with_capacity(IO_EVENT_CAPACITY)),
            unhandled: RefCell::new(Vec::new()),
            sources: RefCell::new(FxHashMap::default()),
            ignored: RefCell::new(FxHashSet::default()),
        };
        Ok(manager)
    }

    /// Non-blockingly poll whether an I/O event is ready.
    ///
    /// If there are still unhandled events from the last OS poll, return one of those.
    /// Otherwise, perform a new non-blocking OS poll and return a new event if
    /// any exist.
    pub fn poll_next(&self) -> Result<Option<Event>, io::Error> {
        let mut unhandled = self.unhandled.borrow_mut();

        if let Some(next) = unhandled.pop() {
            // We still have an unhandled event and thus don't need to poll again.
            return Ok(Some(next));
        };

        let mut events = self.events.borrow_mut();
        self.poll.borrow_mut().poll(&mut events, Some(Duration::ZERO))?;

        // Since [`mio::Events`] only exposes the events through an iterator, we can only
        // access individual elements by storing a cursor ourselves, creating a new iterator
        // and advancing it on every access. At this point it's cleaner to just clone the vector.
        *unhandled = events
            .iter()
            .flat_map(|event| {
                let token = event.token();
                let ignored = self.ignored.borrow().contains(&token);

                // Deregister this source as we only want to receive one event per token.
                if let Err(_err) = self.deregister(token) {
                    // FIXME: We probably want to do something with this error.
                    if ignored {
                        // Filter out the event as the token was already ignored before.
                        return None;
                    }
                } else if ignored {
                    // Filter out the event as the token was already ignored before.
                    return None;
                }
                Some(event.clone())
            })
            .collect();

        Ok(unhandled.pop())
    }

    /// Register a blocking I/O source for a thread together with it's poll interests.
    ///
    /// The source will be deregistered automatically once an event for it is received.
    ///
    /// **Important**: As the OS can always produce spurious wake-ups, it's the callers
    /// responsibility to verify the requested I/O operation is really ready and to re-register
    /// if it's not.
    pub fn register(
        &self,
        mut source: BlockingIoSourceFd,
        thread: ThreadId,
        interests: Interest,
    ) -> Result<(), io::Error> {
        #[allow(clippy::as_conversions)]
        let token = Token(thread.to_u32() as usize);
        let mut sources = self.sources.borrow_mut();
        let mut ignored = self.ignored.borrow_mut();

        if sources.contains_key(&token) && ignored.contains(&token) {
            // This token should've already been removed and is thus ignored.
            // We can now attempt to re-register it with it's new interests.
            self.poll.borrow().registry().reregister(&mut source, token, interests)?;
            ignored.remove(&token);
        } else {
            assert!(
                !sources.contains_key(&token),
                "A thread cannot be registered twice at the same time"
            );

            self.poll.borrow().registry().register(&mut source, token, interests)?;
            sources.insert(token, source);
        }

        Ok(())
    }

    /// Deregister the event source for a token.
    ///
    /// Should the deregistration fail, further events from the token will be ignored.
    fn deregister(&self, token: Token) -> Result<(), io::Error> {
        let mut sources = self.sources.borrow_mut();
        let Some(mut source) = sources.remove(&token) else {
            panic!("Attempt to deregister a token which isn't registered")
        };

        let mut ignored = self.ignored.borrow_mut();
        if let Err(err) = self.poll.borrow().registry().deregister(&mut source) {
            // Re-insert source as we weren't able to deregister it.
            sources.insert(token, source);
            ignored.insert(token);
            Err(err)?;
        }

        // Ensure token isn't part of the ignored list.
        ignored.remove(&token);

        Ok(())
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
            source.as_source_fd(),
            this.machine.threads.active_thread(),
            interests,
        )?;
        this.block_thread(BlockReason::IO { kind }, timeout, callback);
        Ok(())
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
