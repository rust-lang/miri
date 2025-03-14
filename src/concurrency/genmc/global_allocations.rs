use std::cmp::max;
use std::collections::hash_map::Entry;
use std::sync::RwLock;

use genmc_sys::{GENMC_GLOBAL_ADDRESSES_MASK, getGlobalAllocStaticMask};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rustc_const_eval::interpret::{
    AllocId, AllocInfo, AllocKind, InterpResult, PointerArithmetic, interp_ok,
};
use rustc_data_structures::fx::FxHashMap;
use rustc_middle::{err_exhaust, throw_exhaust};
use tracing::info;

use crate::alloc_addresses::align_addr;

#[derive(Debug, Default)]
pub struct GlobalAllocationHandler {
    inner: RwLock<GlobalStateInner>,
}

/// This contains more or less a subset of the functionality of `struct GlobalStateInner` in `alloc_addresses`.
#[derive(Clone, Debug)]
struct GlobalStateInner {
    /// This is used as a map between the address of each allocation and its `AllocId`. It is always
    /// sorted by address. We cannot use a `HashMap` since we can be given an address that is offset
    /// from the base address, and we need to find the `AllocId` it belongs to. This is not the
    /// *full* inverse of `base_addr`; dead allocations have been removed.
    #[allow(unused)] // FIXME(GenMC): do we need this?
    int_to_ptr_map: Vec<(u64, AllocId)>,
    /// The base address for each allocation.
    /// This is the inverse of `int_to_ptr_map`.
    base_addr: FxHashMap<AllocId, u64>,
    /// This is used as a memory address when a new pointer is casted to an integer. It
    /// is always larger than any address that was previously made part of a block.
    next_base_addr: u64,
    /// To add some randomness to the allocations
    /// FIXME(GenMC): maybe seed this from the rng in MiriMachine?
    rng: StdRng,
}

impl Default for GlobalStateInner {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalStateInner {
    pub fn new() -> Self {
        assert_eq!(GENMC_GLOBAL_ADDRESSES_MASK, getGlobalAllocStaticMask());
        assert_ne!(GENMC_GLOBAL_ADDRESSES_MASK, 0);
        Self {
            int_to_ptr_map: Vec::default(),
            base_addr: FxHashMap::default(),
            next_base_addr: GENMC_GLOBAL_ADDRESSES_MASK,
            rng: StdRng::seed_from_u64(0),
        }
    }

    fn global_allocate_addr<'tcx>(
        &mut self,
        alloc_id: AllocId,
        info: AllocInfo,
    ) -> InterpResult<'tcx, u64> {
        let entry = match self.base_addr.entry(alloc_id) {
            Entry::Occupied(occupied_entry) => {
                // Looks like some other thread allocated this for us
                // between when we released the read lock and aquired the write lock,
                // so we just return that value.
                return interp_ok(*occupied_entry.get());
            }
            Entry::Vacant(vacant_entry) => vacant_entry,
        };

        // This is either called immediately after allocation (and then cached), or when
        // adjusting `tcx` pointers (which never get freed). So assert that we are looking
        // at a live allocation. This also ensures that we never re-assign an address to an
        // allocation that previously had an address, but then was freed and the address
        // information was removed.
        assert!(!matches!(info.kind, AllocKind::Dead));

        // This allocation does not have a base address yet, pick or reuse one.

        // We are not in native lib mode, so we control the addresses ourselves.

        // We have to pick a fresh address.
        // Leave some space to the previous allocation, to give it some chance to be less aligned.
        // We ensure that `(global_state.next_base_addr + slack) % 16` is uniformly distributed.
        let slack = self.rng.random_range(0..16);
        // From next_base_addr + slack, round up to adjust for alignment.
        let base_addr =
            self.next_base_addr.checked_add(slack).ok_or_else(|| err_exhaust!(AddressSpaceFull))?;
        let base_addr = align_addr(base_addr, info.align.bytes());

        // Remember next base address.  If this allocation is zero-sized, leave a gap of at
        // least 1 to avoid two allocations having the same base address. (The logic in
        // `alloc_id_from_addr` assumes unique addresses, and different function/vtable pointers
        // need to be distinguishable!)
        self.next_base_addr = base_addr
            .checked_add(max(info.size.bytes(), 1))
            .ok_or_else(|| err_exhaust!(AddressSpaceFull))?;

        assert_ne!(0, base_addr & GENMC_GLOBAL_ADDRESSES_MASK);
        assert_ne!(0, self.next_base_addr & GENMC_GLOBAL_ADDRESSES_MASK);
        // Cache the address for future use.
        entry.insert(base_addr);

        interp_ok(base_addr)
    }
}

// FIXME(GenMC): "ExtPriv" or "PrivExt"?
impl<'tcx> EvalContextExtPriv<'tcx> for crate::MiriInterpCx<'tcx> {}
pub(super) trait EvalContextExtPriv<'tcx>: crate::MiriInterpCxExt<'tcx> {
    fn get_global_allocation_address(
        &self,
        global_allocation_handler: &GlobalAllocationHandler,
        alloc_id: AllocId,
    ) -> InterpResult<'tcx, u64> {
        let this = self.eval_context_ref();
        let info = this.get_alloc_info(alloc_id);

        let global_state = global_allocation_handler.inner.read().unwrap();
        if let Some(base_addr) = global_state.base_addr.get(&alloc_id) {
            info!(
                "GenMC: address for global with alloc id {alloc_id:?} was cached: {base_addr} == {base_addr:#x}"
            );
            return interp_ok(*base_addr);
        }

        drop(global_state);
        // We need to upgrade to a write lock. std::sync::RwLock doesn't support this, so we drop the guard and lock again
        // Note that another thread might run in between and allocate the address, but we handle this case in the allocation function.
        let mut global_state = global_allocation_handler.inner.write().unwrap();
        let base_addr = global_state.global_allocate_addr(alloc_id, info)?;
        // Even if `Size` didn't overflow, we might still have filled up the address space.
        if global_state.next_base_addr > this.target_usize_max() {
            throw_exhaust!(AddressSpaceFull);
        }
        info!(
            "GenMC: global with alloc id {alloc_id:?} got address: {base_addr} == {base_addr:#x}"
        );
        interp_ok(base_addr)
    }
}
