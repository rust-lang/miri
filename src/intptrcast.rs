use std::cell::RefCell;
use std::cmp::max;
use std::collections::hash_map::Entry;

use log::trace;
use rand::Rng;

use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use rustc_span::Span;
use rustc_target::abi::{HasDataLayout, Size};

use crate::*;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ProvenanceMode {
    /// We support `expose_addr`/`from_exposed_addr` via "wildcard" provenance.
    /// However, we want on `from_exposed_addr` to alert the user of the precision loss.
    Default,
    /// Like `Default`, but without the warning.
    Permissive,
    /// We error on `from_exposed_addr`, ensuring no precision loss.
    Strict,
}

pub type GlobalState = RefCell<GlobalStateInner>;

#[derive(Clone, Debug)]
pub struct GlobalStateInner {
    /// This is used as a map between the address of each allocation and its `AllocId`.
    /// It is always sorted
    int_to_ptr_map: Vec<(u64, AllocId)>,
    /// The base address for each allocation.  We cannot put that into
    /// `AllocExtra` because function pointers also have a base address, and
    /// they do not have an `AllocExtra`.
    /// This is the inverse of `int_to_ptr_map`.
    base_addr: FxHashMap<AllocId, u64>,
    /// Whether an allocation has been exposed or not. This cannot be put
    /// into `AllocExtra` for the same reason as `base_addr`.
    exposed: FxHashSet<AllocId>,
    /// The provenance to use for int2ptr casts
    provenance_mode: ProvenanceMode,
}

impl GlobalStateInner {
    pub fn new(config: &MiriConfig) -> Self {
        GlobalStateInner {
            int_to_ptr_map: Vec::default(),
            base_addr: FxHashMap::default(),
            exposed: FxHashSet::default(),
            provenance_mode: config.provenance_mode,
        }
    }
}

impl<'mir, 'tcx> GlobalStateInner {
    // Returns the exposed `AllocId` that corresponds to the specified addr,
    // or `None` if the addr is out of bounds
    fn alloc_id_from_addr(ecx: &MiriEvalContext<'mir, 'tcx>, addr: u64) -> Option<AllocId> {
        let global_state = ecx.machine.intptrcast.borrow();
        assert!(global_state.provenance_mode != ProvenanceMode::Strict);

        let pos = global_state.int_to_ptr_map.binary_search_by_key(&addr, |(addr, _)| *addr);

        // Determine the in-bounds provenance for this pointer.
        // (This is only called on an actual access, so in-bounds is the only possible kind of provenance.)
        let alloc_id = match pos {
            Ok(pos) => Some(global_state.int_to_ptr_map[pos].1),
            Err(0) => None,
            Err(pos) => {
                // This is the largest of the adresses smaller than `int`,
                // i.e. the greatest lower bound (glb)
                let (glb, alloc_id) = global_state.int_to_ptr_map[pos - 1];
                // This never overflows because `addr >= glb`
                let offset = addr - glb;
                // If the offset exceeds the size of the allocation, don't use this `alloc_id`.
                let size = ecx.get_alloc_info(alloc_id).0;
                if offset <= size.bytes() { Some(alloc_id) } else { None }
            }
        }?;

        // We only use this provenance if it has been exposed, *and* is still live.
        if global_state.exposed.contains(&alloc_id) {
            let (_size, _align, kind) = ecx.get_alloc_info(alloc_id);
            match kind {
                AllocKind::LiveData | AllocKind::Function | AllocKind::VTable => {
                    return Some(alloc_id);
                }
                AllocKind::Dead => {}
            }
        }

        None
    }

    pub fn expose_ptr(
        ecx: &mut MiriEvalContext<'mir, 'tcx>,
        alloc_id: AllocId,
        sb: SbTag,
    ) -> InterpResult<'tcx> {
        let global_state = ecx.machine.intptrcast.get_mut();
        // In strict mode, we don't need this, so we can save some cycles by not tracking it.
        if global_state.provenance_mode != ProvenanceMode::Strict {
            trace!("Exposing allocation id {alloc_id:?}");
            global_state.exposed.insert(alloc_id);
            if ecx.machine.stacked_borrows.is_some() {
                ecx.expose_tag(alloc_id, sb)?;
            }
        }
        Ok(())
    }

    pub fn ptr_from_addr_transmute(
        _ecx: &MiriEvalContext<'mir, 'tcx>,
        addr: u64,
    ) -> Pointer<Option<Provenance>> {
        trace!("Transmuting {:#x} to a pointer", addr);

        // We consider transmuted pointers to be "invalid" (`None` provenance).
        Pointer::new(None, Size::from_bytes(addr))
    }

    pub fn ptr_from_addr_cast(
        ecx: &MiriEvalContext<'mir, 'tcx>,
        addr: u64,
    ) -> InterpResult<'tcx, Pointer<Option<Provenance>>> {
        trace!("Casting {:#x} to a pointer", addr);

        let global_state = ecx.machine.intptrcast.borrow();

        match global_state.provenance_mode {
            ProvenanceMode::Default => {
                // The first time this happens at a particular location, print a warning.
                thread_local! {
                    // `Span` is non-`Send`, so we use a thread-local instead.
                    static PAST_WARNINGS: RefCell<FxHashSet<Span>> = RefCell::default();
                }
                PAST_WARNINGS.with_borrow_mut(|past_warnings| {
                    let first = past_warnings.is_empty();
                    if past_warnings.insert(ecx.cur_span()) {
                        // Newly inserted, so first time we see this span.
                        register_diagnostic(NonHaltingDiagnostic::Int2Ptr { details: first });
                    }
                });
            }
            ProvenanceMode::Strict => {
                throw_machine_stop!(TerminationInfo::Int2PtrWithStrictProvenance);
            }
            ProvenanceMode::Permissive => {}
        }

        // This is how wildcard pointers are born.
        Ok(Pointer::new(Some(Provenance::Wildcard), Size::from_bytes(addr)))
    }

    fn alloc_base_addr(ecx: &MiriEvalContext<'mir, 'tcx>, alloc_id: AllocId) -> u64 {
        // TODO avoid hole punch
        // TODO avoid bytes hole punch
        // TODO avoid leaked address hack
        let base_addr: u64 = match ecx.get_alloc_raw(alloc_id) {
            Ok(ref alloc) => {
                let temp = alloc.bytes.as_ptr() as u64;
                assert!(temp % 16 == 0);
                temp
            }
            // Grabbing u128 for max alignment
            Err(_) => Box::leak(Box::new(0u128)) as *const u128 as u64,
        };
        // With our hack, base_addr should always be fully aligned
        let mut global_state = ecx.machine.intptrcast.borrow_mut();
        let global_state = &mut *global_state;

        match global_state.base_addr.entry(alloc_id) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                // There is nothing wrong with a raw pointer being cast to an integer only after
                // it became dangling.  Hence we allow dead allocations.
                let (size, align, _kind) = ecx.get_alloc_info(alloc_id);

                // This allocation does not have a base address yet, assign its bytes base.
                entry.insert(base_addr);
                trace!(
                    "Assigning base address {:#x} to allocation {:?} (size: {}, align: {})",
                    base_addr,
                    alloc_id,
                    size.bytes(),
                    align.bytes(),
                );

                // TODO replace int_to_ptr_map's data structure, since even if we binary search for
                // the insert location, the insertion is still linear (due to copies)
                // I've done it the dumb obviously correct way for now.
                global_state.int_to_ptr_map.retain(|&(ref a,_)| a != &base_addr);
                global_state.int_to_ptr_map.push((base_addr, alloc_id));
                global_state.int_to_ptr_map.sort();

                base_addr
            }
        }
    }

    /// Convert a relative (tcx) pointer to an absolute address.
    pub fn rel_ptr_to_addr(ecx: &MiriEvalContext<'mir, 'tcx>, ptr: Pointer<AllocId>) -> u64 {
        let (alloc_id, offset) = ptr.into_parts(); // offset is relative (AllocId provenance)
        let base_addr = GlobalStateInner::alloc_base_addr(ecx, alloc_id);

        // Add offset with the right kind of pointer-overflowing arithmetic.
        let dl = ecx.data_layout();
        dl.overflowing_offset(base_addr, offset.bytes()).0
    }

    /// When a pointer is used for a memory access, this computes where in which allocation the
    /// access is going.
    pub fn abs_ptr_to_rel(
        ecx: &MiriEvalContext<'mir, 'tcx>,
        ptr: Pointer<Provenance>,
    ) -> Option<(AllocId, Size)> {
        let (tag, addr) = ptr.into_parts(); // addr is absolute (Tag provenance)

        let alloc_id = if let Provenance::Concrete { alloc_id, .. } = tag {
            alloc_id
        } else {
            // A wildcard pointer.
            GlobalStateInner::alloc_id_from_addr(ecx, addr.bytes())?
        };

        let base_addr = GlobalStateInner::alloc_base_addr(ecx, alloc_id);

        // Wrapping "addr - base_addr"
        let dl = ecx.data_layout();
        #[allow(clippy::cast_possible_wrap)] // we want to wrap here
        let neg_base_addr = (base_addr as i64).wrapping_neg();
        Some((
            alloc_id,
            Size::from_bytes(dl.overflowing_signed_offset(addr.bytes(), neg_base_addr).0),
        ))
    }

    /// Shifts `addr` to make it aligned with `align` by rounding `addr` to the smallest multiple
    /// of `align` that is larger or equal to `addr`
    fn align_addr(addr: u64, align: u64) -> u64 {
        match addr % align {
            0 => addr,
            rem => addr.checked_add(align).unwrap() - rem,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align_addr() {
        assert_eq!(GlobalStateInner::align_addr(37, 4), 40);
        assert_eq!(GlobalStateInner::align_addr(44, 4), 44);
    }
}
