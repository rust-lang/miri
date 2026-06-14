use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::shims::{DynFileDescriptionRef, FdId};
use crate::*;

/// Struct reflecting the readiness of a file description.
#[derive(Debug, Clone, PartialEq)]
pub struct Readiness {
    /// Boolean whether the file description is readable.
    pub readable: bool,
    /// Boolean whether the file description is writable.
    pub writable: bool,
    /// Boolean whether the read end of the file description
    /// is closed.
    pub read_closed: bool,
    /// Boolean whether the write end of the file description
    /// is closed.
    pub write_closed: bool,
    /// Boolean whether the file description has an error.
    pub error: bool,
}

impl std::ops::BitAnd for &Readiness {
    type Output = Readiness;

    fn bitand(self, rhs: &Readiness) -> Self::Output {
        Readiness {
            readable: self.readable && rhs.readable,
            writable: self.writable && rhs.writable,
            read_closed: self.read_closed && rhs.read_closed,
            write_closed: self.write_closed && rhs.write_closed,
            error: self.error && rhs.error,
        }
    }
}

impl std::ops::BitOrAssign for Readiness {
    fn bitor_assign(&mut self, rhs: Self) {
        self.readable |= rhs.readable;
        self.writable |= rhs.writable;
        self.read_closed |= rhs.read_closed;
        self.write_closed |= rhs.write_closed;
        self.error |= rhs.error;
    }
}

impl AsRef<Self> for Readiness {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl Readiness {
    pub const EMPTY: Readiness = Readiness {
        readable: false,
        writable: false,
        read_closed: false,
        write_closed: false,
        error: false,
    };
}

/// A trait which can be implemented for things which need to receive notifications when
/// the readiness of file descriptions change.
pub trait ReadinessConsumer {
    /// The file description with id `fd_id` for which the consumer has
    /// an interest has received a readiness event.
    /// When `force_edge` is [`true`], edge-triggered consumers should
    /// emit an edge even when `readiness` did not change since the last
    /// readiness event.
    fn ready_event<'tcx>(
        &self,
        fd_id: FdId,
        readiness: Readiness,
        force_edge: bool,
        ecx: &mut MiriInterpCx<'tcx>,
    ) -> InterpResult<'tcx>;

    // The file description with id `fd_id` for which this consumer has
    // an interest has been closed.
    // All interests for this file description have been removed from
    // the readiness manager.
    fn fd_closed(&self, _fd_id: FdId) {}
}

/// The identifier of a registered [`ReadinessConsumer`].
/// This id is returned when the consumer is registered to
/// the readiness manager.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct ReadinessConsumerId(usize);

/// Manager storing readiness consumers and their interests in readiness
/// events from different file descriptions.
///
/// The lifecycle is thought that consumers are registered to this manager
/// directly after their creation. They can then register and deregister
/// interests for different file descriptions such that they get notified
/// when the readiness of those file descriptions changes (or when edges are
/// forced). When the consumers are destroyed, they should deregister
/// themselves from the manager.
pub struct ReadinessManager {
    /// The id with which the next readiness consumer will be registered.
    next_consumer_id: usize,
    /// The set of registered readiness consumers indexed by their ids.
    consumers: BTreeMap<ReadinessConsumerId, Rc<Box<dyn ReadinessConsumer>>>,
    /// The registered interests indexed by file description ids.
    interests: BTreeMap<FdId, BTreeSet<ReadinessConsumerId>>,
}

impl ReadinessManager {
    pub fn new() -> Self {
        Self { next_consumer_id: 0, consumers: BTreeMap::new(), interests: BTreeMap::new() }
    }

    /// Register a [`ReadinessConsumer`] to the readiness manager.
    /// The consumer gets assigned a [`ReadinessConsumerId`] which can then
    /// be used to add file description interests to this consumer using
    /// [`ReadinessManager::register_interest`].
    pub fn register_consumer(
        &mut self,
        consumer: impl ReadinessConsumer + 'static,
    ) -> ReadinessConsumerId {
        let id = ReadinessConsumerId(self.next_consumer_id);
        self.next_consumer_id = id.0.strict_add(1);
        self.consumers.insert(id, Rc::new(Box::new(consumer)));
        id
    }

    /// Deregister the consumer with id `consumer_id` from the readiness manager.
    /// This also removes all the interests which were registered for this consumer.
    pub fn deregister_consumer(&mut self, consumer_id: ReadinessConsumerId) {
        self.consumers.remove(&consumer_id).expect("consumer should be registered");
        // Remove all interests of this consumer.
        self.interests.values_mut().for_each(|consumers| consumers.retain(|id| id != &consumer_id));
    }

    /// Add an interest to the consumer with id `consumer_id` for the file
    /// description with id `fd_id`.
    ///
    /// Whilst this interest is registered, the consumer receives updates through
    /// [`ReadinessConsumer::ready_event`] when the readiness of the file description
    /// changes or an edge is forced.
    pub fn register_interest(&mut self, fd_id: FdId, consumer_id: ReadinessConsumerId) {
        if !self.consumers.contains_key(&consumer_id) {
            panic!("consumer should be registered");
        }
        let interests = self.interests.entry(fd_id).or_default();
        if !interests.insert(consumer_id) {
            panic!("consumer has already a registered interested in this file description")
        }
    }

    /// Remove the interest of the consumer with id `consumer_id` into the file description
    /// with id `fd_id`.
    pub fn deregister_interest(&mut self, fd_id: FdId, consumer_id: ReadinessConsumerId) {
        if !self.consumers.contains_key(&consumer_id) {
            panic!("consumer should be registered");
        }
        self.interests.remove(&fd_id);
    }

    /// Remove all readiness interests for the file description with id `fd_id`.
    ///
    /// [`ReadinessConsumer::fd_closed`] will be called on all consumers which
    /// have a registered interest in `fd_id`.
    pub fn deregister_fd_interests(&mut self, fd_id: FdId) {
        let consumers = self.interests.remove(&fd_id).unwrap_or_default();
        for consumer_id in consumers {
            let consumer = self.consumers.get(&consumer_id).expect("consumer should be registered");
            consumer.fd_closed(fd_id);
        }
    }
}

impl<'tcx> EvalContextExt<'tcx> for MiriInterpCx<'tcx> {}
pub trait EvalContextExt<'tcx>: MiriInterpCxExt<'tcx> {
    /// Notify interested consumers about the readiness of the `fd` file description.
    /// When `force_edge` is [`true`], edge-triggered consumers which are interested
    /// in this file description should emit an edge even when the readiness of the
    /// file description did not change.
    fn notify_fd_readiness(
        &mut self,
        fd: DynFileDescriptionRef,
        force_edge: bool,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let fd_id = fd.id();

        let Some(consumers) = this.machine.readiness.interests.get(&fd_id).cloned() else {
            return interp_ok(());
        };

        let current_readiness = fd.readiness()?;
        for consumer_id in &consumers {
            let consumer = this
                .machine
                .readiness
                .consumers
                .get(consumer_id)
                .expect("consumer should be registered")
                .clone();

            consumer.ready_event(fd_id, current_readiness.clone(), force_edge, this)?;
        }

        interp_ok(())
    }
}
