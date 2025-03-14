use genmc_sys::{MemOrdering, RMWBinOp};

use crate::{AtomicFenceOrd, AtomicReadOrd, AtomicRwOrd, AtomicWriteOrd};

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

impl AtomicFenceOrd {
    pub(super) fn convert(self) -> MemOrdering {
        match self {
            AtomicFenceOrd::Acquire => MemOrdering::Acquire,
            AtomicFenceOrd::Release => MemOrdering::Release,
            AtomicFenceOrd::AcqRel => MemOrdering::AcquireRelease,
            AtomicFenceOrd::SeqCst => MemOrdering::SequentiallyConsistent,
        }
    }
}

impl AtomicRwOrd {
    /// Split up an atomic read-write memory ordering into a separate read and write ordering.
    pub(super) fn split_memory_orderings(self) -> (AtomicReadOrd, AtomicWriteOrd) {
        match self {
            AtomicRwOrd::Relaxed => (AtomicReadOrd::Relaxed, AtomicWriteOrd::Relaxed),
            AtomicRwOrd::Acquire => (AtomicReadOrd::Acquire, AtomicWriteOrd::Relaxed),
            AtomicRwOrd::Release => (AtomicReadOrd::Relaxed, AtomicWriteOrd::Release),
            AtomicRwOrd::AcqRel => (AtomicReadOrd::Acquire, AtomicWriteOrd::Release),
            AtomicRwOrd::SeqCst => (AtomicReadOrd::SeqCst, AtomicWriteOrd::SeqCst),
        }
    }

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

pub(super) fn min_max_to_genmc_rmw_op(min: bool, is_signed: bool) -> RMWBinOp {
    match (min, is_signed) {
        (true, true) => RMWBinOp::Min, // TODO GENMC: is there a use for FMin? (Min, UMin, FMin)
        (false, true) => RMWBinOp::Max,
        (true, false) => RMWBinOp::UMin,
        (false, false) => RMWBinOp::UMax,
    }
}

pub(super) fn to_genmc_rmw_op(bin_op: rustc_middle::mir::BinOp, negate: bool) -> RMWBinOp {
    match bin_op {
        rustc_middle::mir::BinOp::Add => RMWBinOp::Add,
        rustc_middle::mir::BinOp::Sub => RMWBinOp::Sub,
        rustc_middle::mir::BinOp::BitOr if !negate => RMWBinOp::Or,
        rustc_middle::mir::BinOp::BitXor if !negate => RMWBinOp::Xor,
        rustc_middle::mir::BinOp::BitAnd if negate => RMWBinOp::Nand,
        rustc_middle::mir::BinOp::BitAnd => RMWBinOp::And,
        _ => {
            panic!("unsupported atomic operation: bin_op: {bin_op:?}, negate: {negate}");
        }
    }
}
