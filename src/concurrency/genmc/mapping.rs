use genmc_sys::{MemOrdering, RMWBinOp};

use crate::{AtomicReadOrd, AtomicRwOrd, AtomicWriteOrd};

// This file contains functionality to convert between Miri enums and their GenMC counterparts.

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

/// Convert a possibly signed Miri min/max operation to its GenMC counterpart.
pub(super) fn min_max_to_genmc_rmw_op(min: bool, is_signed: bool) -> RMWBinOp {
    // FIXME(genmc): is there a use for FMin/FMax? GenMC has (Min, UMin, FMin)
    match (min, is_signed) {
        (true, true) => RMWBinOp::Min,
        (false, true) => RMWBinOp::Max,
        (true, false) => RMWBinOp::UMin,
        (false, false) => RMWBinOp::UMax,
    }
}

/// Convert a possibly negated binary operation to its GenMC counterpart.
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
