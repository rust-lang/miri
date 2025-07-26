use genmc_sys::MemOrdering;

use crate::{AtomicReadOrd, AtomicRwOrd, AtomicWriteOrd};

impl AtomicReadOrd {
    pub(super) fn convert(self) -> MemOrdering {
        match self {
            AtomicReadOrd::Relaxed => MemOrdering::Relaxed,
            AtomicReadOrd::Acquire => MemOrdering::Acquire,
            AtomicReadOrd::SeqCst => MemOrdering::SequentiallyConsistent,
        }
    }
}

impl AtomicWriteOrd {
    pub(super) fn convert(self) -> MemOrdering {
        match self {
            AtomicWriteOrd::Relaxed => MemOrdering::Relaxed,
            AtomicWriteOrd::Release => MemOrdering::Release,
            AtomicWriteOrd::SeqCst => MemOrdering::SequentiallyConsistent,
        }
    }
}

impl AtomicRwOrd {
    /// Split up the atomic success ordering of a read-modify-write operation into GenMC's representation.
    /// Note that both returned orderings are currently identical, because this is what GenMC expects.
    pub(super) fn to_genmc_memory_orderings(self) -> (MemOrdering, MemOrdering) {
        match self {
            AtomicRwOrd::Relaxed => (MemOrdering::Relaxed, MemOrdering::Relaxed),
            AtomicRwOrd::Acquire => (MemOrdering::Acquire, MemOrdering::Acquire),
            AtomicRwOrd::Release => (MemOrdering::Release, MemOrdering::Release),
            AtomicRwOrd::AcqRel => (MemOrdering::AcquireRelease, MemOrdering::AcquireRelease),
            AtomicRwOrd::SeqCst =>
                (MemOrdering::SequentiallyConsistent, MemOrdering::SequentiallyConsistent),
        }
    }
}
