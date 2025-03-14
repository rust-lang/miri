use rustc_data_structures::fx::FxHashSet;
use rustc_middle::query::TyCtxtAt;
use rustc_span::Span;

use crate::{AtomicReadOrd, AtomicRwOrd};

#[derive(Default)]
pub struct WarningsCache {
    emitted_compare_exchange_weak: FxHashSet<Span>,
    emitted_compare_exchange_failure_ordering: FxHashSet<(Span, AtomicReadOrd, AtomicReadOrd)>,
}

impl WarningsCache {
    /// Warn about unsupported spurious failures of `compare_exchange_weak`, once per span, returning `true` if the warning was printed.
    pub fn warn_once_compare_exchange_weak<'tcx>(&mut self, tcx: &TyCtxtAt<'tcx>) -> bool {
        if self.emitted_compare_exchange_weak.insert(tcx.span) {
            tcx.dcx().span_warn(tcx.span, "GenMC mode currently does not model spurious failures of `compare_exchange_weak`. This may lead to missed bugs (possible unsoundness)!");
            return true;
        }
        false
    }

    /// Check if the given failure ordering is unsupported by GenMC.
    /// Warning is printed only once per span and ordering combination.
    /// Returns `true` if the warning was printed.
    pub fn warn_once_rmw_failure_ordering<'tcx>(
        &mut self,
        tcx: &TyCtxtAt<'tcx>,
        success_ordering: AtomicRwOrd,
        failure_load_ordering: AtomicReadOrd,
    ) -> bool {
        let (success_load_ordering, _success_store_ordering) =
            success_ordering.split_memory_orderings();
        let is_failure_ordering_weaker = match (success_load_ordering, failure_load_ordering) {
            // Unsound: failure ordering is weaker than success ordering, but GenMC treats them as equally strong.
            // Actual program execution might have behavior not modelled by GenMC:
            (AtomicReadOrd::Acquire, AtomicReadOrd::Relaxed)
            | (AtomicReadOrd::SeqCst, AtomicReadOrd::Relaxed)
            | (AtomicReadOrd::SeqCst, AtomicReadOrd::Acquire) => true,
            // Possible false positives: failure ordering is stronger than success ordering, but GenMC treats them as equally strong.
            // We might explore executions that are not allowed by the program.
            (AtomicReadOrd::Relaxed, AtomicReadOrd::Acquire)
            | (AtomicReadOrd::Relaxed, AtomicReadOrd::SeqCst)
            | (AtomicReadOrd::Acquire, AtomicReadOrd::SeqCst) => false,
            // Correct: failure ordering is equally strong as success ordering:
            (AtomicReadOrd::Relaxed, AtomicReadOrd::Relaxed)
            | (AtomicReadOrd::Acquire, AtomicReadOrd::Acquire)
            | (AtomicReadOrd::SeqCst, AtomicReadOrd::SeqCst) => return false,
        };
        let key = (tcx.span, success_load_ordering, failure_load_ordering);
        if self.emitted_compare_exchange_failure_ordering.insert(key) {
            let error = if is_failure_ordering_weaker {
                "miss bugs related to this memory access (possible unsoundness)!"
            } else {
                "incorrectly detect errors related to this memory access (possible false positives)."
            };
            let msg = format!(
                "GenMC currently does not model the atomic failure ordering for `compare_exchange`. Failure ordering '{failure_load_ordering:?}' is treated like '{success_load_ordering:?}', which means that Miri might {error}",
            );
            // FIXME(genmc): this doesn't print a span:
            tcx.dcx().span_warn(tcx.span, msg);
            return true;
        }
        false
    }
}
