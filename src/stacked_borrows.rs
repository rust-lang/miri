//! Implements "Stacked Borrows".  See <https://github.com/rust-lang/unsafe-code-guidelines/blob/master/wip/stacked-borrows.md>
//! for further information.

use log::trace;
use std::cell::RefCell;
use std::fmt;
use std::num::NonZeroU64;

use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use rustc_hir::Mutability;
use rustc_middle::mir::RetagKind;
use rustc_middle::ty::{
    self,
    layout::{HasParamEnv, LayoutOf},
};
use rustc_span::DUMMY_SP;
use rustc_target::abi::Size;
use std::collections::HashSet;

use crate::*;

pub mod diagnostics;
use diagnostics::{AllocHistory, TagHistory};

pub type PtrId = NonZeroU64;
pub type CallId = NonZeroU64;
pub type AllocExtra = Stacks;

/// Tracking pointer provenance
#[derive(Copy, Clone, Hash, PartialEq, Eq)]
pub enum SbTag {
    Tagged(PtrId),
    Untagged,
}

impl fmt::Debug for SbTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SbTag::Tagged(id) => write!(f, "<{}>", id),
            SbTag::Untagged => write!(f, "<untagged>"),
        }
    }
}

/// Indicates which permission is granted (by this item to some pointers)
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
pub enum Permission {
    /// Grants unique mutable access.
    Unique,
    /// Grants shared mutable access.
    SharedReadWrite,
    /// Grants shared read-only access.
    SharedReadOnly,
    /// Grants no access, but separates two groups of SharedReadWrite so they are not
    /// all considered mutually compatible.
    Disabled,
}

/// An item in the per-location borrow stack.
#[derive(Copy, Clone, Hash, PartialEq, Eq)]
pub struct Item {
    /// The permission this item grants.
    perm: Permission,
    /// The pointers the permission is granted to.
    tag: SbTag,
    /// An optional protector, ensuring the item cannot get popped until `CallId` is over.
    protector: Option<CallId>,
}

impl fmt::Debug for Item {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:?} for {:?}", self.perm, self.tag)?;
        if let Some(call) = self.protector {
            write!(f, " (call {})", call)?;
        }
        write!(f, "]")?;
        Ok(())
    }
}

/// Extra per-location state.
#[derive(Clone, Debug, Eq)]
pub struct Stack {
    /// Used *mostly* as a stack; never empty.
    /// Invariants:
    /// * Above a `SharedReadOnly` there can only be more `SharedReadOnly`.
    /// * Except for `Untagged`, no tag occurs in the stack more than once.
    borrows: Vec<Item>,
}

/// This implementation is primarily used when attempting to merge borrow stacks for adjacent
/// bytes. For adjacent stacks, when the stacks differ at all, they tend to differ either in
/// length or at the end of the borrows array. Iterating in reverse returns faster for `Stack`s
/// that are not equal.
impl PartialEq for Stack {
    fn eq(&self, other: &Self) -> bool {
        let Stack { borrows: lhs } = self;
        let Stack { borrows: rhs } = other;
        lhs.len() == rhs.len() && lhs.iter().rev().zip(rhs.iter().rev()).all(|(l, r)| l == r)
    }
}

/// Extra per-allocation state.
#[derive(Clone, Debug)]
pub struct Stacks {
    // Even reading memory can have effects on the stack, so we need a `RefCell` here.
    stacks: RefCell<RangeMap<Stack>>,
    /// Stores past operations on this allocation
    history: RefCell<AllocHistory>,
}

/// Extra global state, available to the memory access hooks.
#[derive(Debug)]
pub struct GlobalStateInner {
    /// Next unused pointer ID (tag).
    next_ptr_id: PtrId,
    /// Table storing the "base" tag for each allocation.
    /// The base tag is the one used for the initial pointer.
    /// We need this in a separate table to handle cyclic statics.
    base_ptr_ids: FxHashMap<AllocId, SbTag>,
    /// Next unused call ID (for protectors).
    next_call_id: CallId,
    /// Those call IDs corresponding to functions that are still running.
    active_calls: FxHashSet<CallId>,
    /// The pointer ids to trace
    tracked_pointer_tags: HashSet<PtrId>,
    /// The call ids to trace
    tracked_call_ids: HashSet<CallId>,
    /// Whether to track raw pointers.
    tag_raw: bool,
}

/// We need interior mutable access to the global state.
pub type GlobalState = RefCell<GlobalStateInner>;

/// Indicates which kind of access is being performed.
#[derive(Copy, Clone, Hash, PartialEq, Eq, Debug)]
pub enum AccessKind {
    Read,
    Write,
}

impl fmt::Display for AccessKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccessKind::Read => write!(f, "read access"),
            AccessKind::Write => write!(f, "write access"),
        }
    }
}

/// Indicates which kind of reference is being created.
/// Used by high-level `reborrow` to compute which permissions to grant to the
/// new pointer.
#[derive(Copy, Clone, Hash, PartialEq, Eq)]
pub enum RefKind {
    /// `&mut` and `Box`.
    Unique { two_phase: bool },
    /// `&` with or without interior mutability.
    Shared,
    /// `*mut`/`*const` (raw pointers).
    Raw { mutable: bool },
}

impl fmt::Display for RefKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefKind::Unique { two_phase: false } => write!(f, "unique"),
            RefKind::Unique { two_phase: true } => write!(f, "unique (two-phase)"),
            RefKind::Shared => write!(f, "shared"),
            RefKind::Raw { mutable: true } => write!(f, "raw (mutable)"),
            RefKind::Raw { mutable: false } => write!(f, "raw (constant)"),
        }
    }
}

/// Utilities for initialization and ID generation
impl GlobalStateInner {
    pub fn new(
        tracked_pointer_tags: HashSet<PtrId>,
        tracked_call_ids: HashSet<CallId>,
        tag_raw: bool,
    ) -> Self {
        GlobalStateInner {
            next_ptr_id: NonZeroU64::new(1).unwrap(),
            base_ptr_ids: FxHashMap::default(),
            next_call_id: NonZeroU64::new(1).unwrap(),
            active_calls: FxHashSet::default(),
            tracked_pointer_tags,
            tracked_call_ids,
            tag_raw,
        }
    }

    fn new_ptr(&mut self) -> PtrId {
        let id = self.next_ptr_id;
        if self.tracked_pointer_tags.contains(&id) {
            register_diagnostic(NonHaltingDiagnostic::CreatedPointerTag(id));
        }
        self.next_ptr_id = NonZeroU64::new(id.get() + 1).unwrap();
        id
    }

    pub fn new_call(&mut self) -> CallId {
        let id = self.next_call_id;
        trace!("new_call: Assigning ID {}", id);
        if self.tracked_call_ids.contains(&id) {
            register_diagnostic(NonHaltingDiagnostic::CreatedCallId(id));
        }
        assert!(self.active_calls.insert(id));
        self.next_call_id = NonZeroU64::new(id.get() + 1).unwrap();
        id
    }

    pub fn end_call(&mut self, id: CallId) {
        assert!(self.active_calls.remove(&id));
    }

    fn is_active(&self, id: CallId) -> bool {
        self.active_calls.contains(&id)
    }

    pub fn base_tag(&mut self, id: AllocId) -> SbTag {
        self.base_ptr_ids.get(&id).copied().unwrap_or_else(|| {
            let tag = SbTag::Tagged(self.new_ptr());
            trace!("New allocation {:?} has base tag {:?}", id, tag);
            self.base_ptr_ids.try_insert(id, tag).unwrap();
            tag
        })
    }

    pub fn base_tag_untagged(&mut self, id: AllocId) -> SbTag {
        trace!("New allocation {:?} has no base tag (untagged)", id);
        let tag = SbTag::Untagged;
        // This must only be done on new allocations.
        self.base_ptr_ids.try_insert(id, tag).unwrap();
        tag
    }
}

/// Error reporting
pub fn err_sb_ub<'tcx>(
    msg: String,
    help: Option<String>,
    history: Option<TagHistory>,
) -> InterpError<'tcx> {
    err_machine_stop!(TerminationInfo::ExperimentalUb {
        msg,
        help,
        url: format!(
            "https://github.com/rust-lang/unsafe-code-guidelines/blob/master/wip/stacked-borrows.md"
        ),
        history
    })
}

// # Stacked Borrows Core Begin

/// We need to make at least the following things true:
///
/// U1: After creating a `Uniq`, it is at the top.
/// U2: If the top is `Uniq`, accesses must be through that `Uniq` or remove it it.
/// U3: If an access happens with a `Uniq`, it requires the `Uniq` to be in the stack.
///
/// F1: After creating a `&`, the parts outside `UnsafeCell` have our `SharedReadOnly` on top.
/// F2: If a write access happens, it pops the `SharedReadOnly`.  This has three pieces:
///     F2a: If a write happens granted by an item below our `SharedReadOnly`, the `SharedReadOnly`
///          gets popped.
///     F2b: No `SharedReadWrite` or `Unique` will ever be added on top of our `SharedReadOnly`.
/// F3: If an access happens with an `&` outside `UnsafeCell`,
///     it requires the `SharedReadOnly` to still be in the stack.

/// Core relation on `Permission` to define which accesses are allowed
impl Permission {
    /// This defines for a given permission, whether it permits the given kind of access.
    fn grants(self, access: AccessKind) -> bool {
        // Disabled grants nothing. Otherwise, all items grant read access, and except for SharedReadOnly they grant write access.
        self != Permission::Disabled
            && (access == AccessKind::Read || self != Permission::SharedReadOnly)
    }
}

/// Core per-location operations: access, dealloc, reborrow.
impl<'tcx> Stack {
    /// Find the item granting the given kind of access to the given tag, and return where
    /// it is on the stack.
    fn find_granting(&self, access: AccessKind, tag: SbTag) -> Option<usize> {
        self.borrows
            .iter()
            .enumerate() // we also need to know *where* in the stack
            .rev() // search top-to-bottom
            // Return permission of first item that grants access.
            // We require a permission with the right tag, ensuring U3 and F3.
            .find_map(
                |(idx, item)| {
                    if tag == item.tag && item.perm.grants(access) { Some(idx) } else { None }
                },
            )
    }

    /// Find the first write-incompatible item above the given one --
    /// i.e, find the height to which the stack will be truncated when writing to `granting`.
    fn find_first_write_incompatible(&self, granting: usize) -> usize {
        let perm = self.borrows[granting].perm;
        match perm {
            Permission::SharedReadOnly => bug!("Cannot use SharedReadOnly for writing"),
            Permission::Disabled => bug!("Cannot use Disabled for anything"),
            // On a write, everything above us is incompatible.
            Permission::Unique => granting + 1,
            Permission::SharedReadWrite => {
                // The SharedReadWrite *just* above us are compatible, to skip those.
                let mut idx = granting + 1;
                while let Some(item) = self.borrows.get(idx) {
                    if item.perm == Permission::SharedReadWrite {
                        // Go on.
                        idx += 1;
                    } else {
                        // Found first incompatible!
                        break;
                    }
                }
                idx
            }
        }
    }

    /// Check if the given item is protected.
    ///
    /// The `provoking_access` argument is only used to produce diagnostics.
    /// It is `Some` when we are granting the contained access for said tag, and it is
    /// `None` during a deallocation.
    /// Within `provoking_access, the `AllocRange` refers the entire operation, and
    /// the `Size` refers to the specific location in the `AllocRange` that we are
    /// currently checking.
    fn check_protector(
        item: &Item,
        provoking_access: Option<(SbTag, AllocRange, Size, AccessKind)>, // just for debug printing and error messages
        global: &GlobalStateInner,
        alloc_history: &mut AllocHistory,
    ) -> InterpResult<'tcx> {
        if let SbTag::Tagged(id) = item.tag {
            if global.tracked_pointer_tags.contains(&id) {
                register_diagnostic(NonHaltingDiagnostic::PoppedPointerTag(
                    *item,
                    provoking_access.map(|(tag, _alloc_range, _size, access)| (tag, access)),
                ));
            }
        }
        if let Some(call) = item.protector {
            if global.is_active(call) {
                if let Some((tag, alloc_range, offset, _access)) = provoking_access {
                    Err(err_sb_ub(
                        format!(
                            "not granting access to tag {:?} because incompatible item is protected: {:?}",
                            tag, item
                        ),
                        None,
                        alloc_history.get_logs_relevant_to(
                            tag,
                            alloc_range,
                            offset,
                            Some(item.tag),
                        ),
                    ))?
                } else {
                    Err(err_sb_ub(
                        format!("deallocating while item is protected: {:?}", item),
                        None,
                        None,
                    ))?
                }
            }
        }
        Ok(())
    }

    /// Test if a memory `access` using pointer tagged `tag` is granted.
    /// If yes, return the index of the item that granted it.
    /// `range` refers the entire operation, and `offset` refers to the specific offset into the
    /// allocation that we are currently checking.
    fn access(
        &mut self,
        access: AccessKind,
        tag: SbTag,
        (alloc_id, alloc_range, offset): (AllocId, AllocRange, Size), // just for debug printing and error messages
        global: &mut GlobalStateInner,
        current_span: &mut CurrentSpan<'_, '_, 'tcx>,
        alloc_history: &mut AllocHistory,
    ) -> InterpResult<'tcx> {
        // Two main steps: Find granting item, remove incompatible items above.

        // Step 1: Find granting item.
        let granting_idx = self.find_granting(access, tag).ok_or_else(|| {
            alloc_history.access_error(access, tag, alloc_id, alloc_range, offset, self)
        })?;

        // Step 2: Remove incompatible items above them.  Make sure we do not remove protected
        // items.  Behavior differs for reads and writes.
        if access == AccessKind::Write {
            // Remove everything above the write-compatible items, like a proper stack. This makes sure read-only and unique
            // pointers become invalid on write accesses (ensures F2a, and ensures U2 for write accesses).
            let first_incompatible_idx = self.find_first_write_incompatible(granting_idx);
            for item in self.borrows.drain(first_incompatible_idx..).rev() {
                trace!("access: popping item {:?}", item);
                Stack::check_protector(
                    &item,
                    Some((tag, alloc_range, offset, access)),
                    global,
                    alloc_history,
                )?;
                alloc_history.log_invalidation(item.tag, alloc_range, current_span);
            }
        } else {
            // On a read, *disable* all `Unique` above the granting item.  This ensures U2 for read accesses.
            // The reason this is not following the stack discipline (by removing the first Unique and
            // everything on top of it) is that in `let raw = &mut *x as *mut _; let _val = *x;`, the second statement
            // would pop the `Unique` from the reborrow of the first statement, and subsequently also pop the
            // `SharedReadWrite` for `raw`.
            // This pattern occurs a lot in the standard library: create a raw pointer, then also create a shared
            // reference and use that.
            // We *disable* instead of removing `Unique` to avoid "connecting" two neighbouring blocks of SRWs.
            for idx in ((granting_idx + 1)..self.borrows.len()).rev() {
                let item = &mut self.borrows[idx];
                if item.perm == Permission::Unique {
                    trace!("access: disabling item {:?}", item);
                    Stack::check_protector(
                        item,
                        Some((tag, alloc_range, offset, access)),
                        global,
                        alloc_history,
                    )?;
                    item.perm = Permission::Disabled;
                    alloc_history.log_invalidation(item.tag, alloc_range, current_span);
                }
            }
        }

        // Done.
        Ok(())
    }

    /// Deallocate a location: Like a write access, but also there must be no
    /// active protectors at all because we will remove all items.
    fn dealloc(
        &mut self,
        tag: SbTag,
        (alloc_id, alloc_range, offset): (AllocId, AllocRange, Size), // just for debug printing and error messages
        global: &GlobalStateInner,
        alloc_history: &mut AllocHistory,
    ) -> InterpResult<'tcx> {
        // Step 1: Find granting item.
        self.find_granting(AccessKind::Write, tag).ok_or_else(|| {
            err_sb_ub(format!(
                "no item granting write access for deallocation to tag {:?} at {:?} found in borrow stack",
                tag, alloc_id,
                ),
                None,
                alloc_history.get_logs_relevant_to(tag, alloc_range, offset, None),
            )
        })?;

        // Step 2: Remove all items.  Also checks for protectors.
        for item in self.borrows.drain(..).rev() {
            Stack::check_protector(&item, None, global, alloc_history)?;
        }

        Ok(())
    }

    /// Derive a new pointer from one with the given tag.
    /// `weak` controls whether this operation is weak or strong: weak granting does not act as
    /// an access, and they add the new item directly on top of the one it is derived
    /// from instead of all the way at the top of the stack.
    /// `range` refers the entire operation, and `offset` refers to the specific location in
    /// `range` that we are currently checking.
    fn grant(
        &mut self,
        derived_from: SbTag,
        new: Item,
        (alloc_id, alloc_range, offset): (AllocId, AllocRange, Size), // just for debug printing and error messages
        global: &mut GlobalStateInner,
        current_span: &mut CurrentSpan<'_, '_, 'tcx>,
        alloc_history: &mut AllocHistory,
    ) -> InterpResult<'tcx> {
        // Figure out which access `perm` corresponds to.
        let access =
            if new.perm.grants(AccessKind::Write) { AccessKind::Write } else { AccessKind::Read };
        // Now we figure out which item grants our parent (`derived_from`) this kind of access.
        // We use that to determine where to put the new item.
        let granting_idx = self.find_granting(access, derived_from).ok_or_else(|| {
            alloc_history.grant_error(derived_from, new, alloc_id, alloc_range, offset, self)
        })?;

        // Compute where to put the new item.
        // Either way, we ensure that we insert the new item in a way such that between
        // `derived_from` and the new one, there are only items *compatible with* `derived_from`.
        let new_idx = if new.perm == Permission::SharedReadWrite {
            assert!(
                access == AccessKind::Write,
                "this case only makes sense for stack-like accesses"
            );
            // SharedReadWrite can coexist with "existing loans", meaning they don't act like a write
            // access.  Instead of popping the stack, we insert the item at the place the stack would
            // be popped to (i.e., we insert it above all the write-compatible items).
            // This ensures F2b by adding the new item below any potentially existing `SharedReadOnly`.
            self.find_first_write_incompatible(granting_idx)
        } else {
            // A "safe" reborrow for a pointer that actually expects some aliasing guarantees.
            // Here, creating a reference actually counts as an access.
            // This ensures F2b for `Unique`, by removing offending `SharedReadOnly`.
            self.access(
                access,
                derived_from,
                (alloc_id, alloc_range, offset),
                global,
                current_span,
                alloc_history,
            )?;

            // We insert "as far up as possible": We know only compatible items are remaining
            // on top of `derived_from`, and we want the new item at the top so that we
            // get the strongest possible guarantees.
            // This ensures U1 and F1.
            self.borrows.len()
        };

        // Put the new item there. As an optimization, deduplicate if it is equal to one of its new neighbors.
        if self.borrows[new_idx - 1] == new || self.borrows.get(new_idx) == Some(&new) {
            // Optimization applies, done.
            trace!("reborrow: avoiding adding redundant item {:?}", new);
        } else {
            trace!("reborrow: adding item {:?}", new);
            self.borrows.insert(new_idx, new);
        }

        Ok(())
    }
}
// # Stacked Borrows Core End

/// Map per-stack operations to higher-level per-location-range operations.
impl<'tcx> Stacks {
    /// Creates new stack with initial tag.
    fn new(size: Size, perm: Permission, tag: SbTag) -> Self {
        let item = Item { perm, tag, protector: None };
        let stack = Stack { borrows: vec![item] };

        Stacks {
            stacks: RefCell::new(RangeMap::new(size, stack)),
            history: RefCell::new(AllocHistory::new()),
        }
    }

    /// Call `f` on every stack in the range.
    fn for_each(
        &self,
        range: AllocRange,
        mut f: impl FnMut(Size, &mut Stack, &mut AllocHistory) -> InterpResult<'tcx>,
    ) -> InterpResult<'tcx> {
        let mut stacks = self.stacks.borrow_mut();
        let history = &mut *self.history.borrow_mut();
        for (offset, stack) in stacks.iter_mut(range.start, range.size) {
            f(offset, stack, history)?;
        }
        Ok(())
    }

    /// Call `f` on every stack in the range.
    fn for_each_mut(
        &mut self,
        range: AllocRange,
        mut f: impl FnMut(Size, &mut Stack, &mut AllocHistory) -> InterpResult<'tcx>,
    ) -> InterpResult<'tcx> {
        let stacks = self.stacks.get_mut();
        let history = &mut *self.history.borrow_mut();
        for (offset, stack) in stacks.iter_mut(range.start, range.size) {
            f(offset, stack, history)?;
        }
        Ok(())
    }
}

/// Glue code to connect with Miri Machine Hooks
impl Stacks {
    pub fn new_allocation(
        id: AllocId,
        size: Size,
        state: &GlobalState,
        kind: MemoryKind<MiriMemoryKind>,
        mut current_span: CurrentSpan<'_, '_, '_>,
    ) -> Self {
        let mut extra = state.borrow_mut();
        let (base_tag, perm) = match kind {
            // New unique borrow. This tag is not accessible by the program,
            // so it will only ever be used when using the local directly (i.e.,
            // not through a pointer). That is, whenever we directly write to a local, this will pop
            // everything else off the stack, invalidating all previous pointers,
            // and in particular, *all* raw pointers.
            MemoryKind::Stack => (extra.base_tag(id), Permission::Unique),
            // `Global` memory can be referenced by global pointers from `tcx`.
            // Thus we call `global_base_ptr` such that the global pointers get the same tag
            // as what we use here.
            // `ExternStatic` is used for extern statics, so the same reasoning applies.
            // The others are various forms of machine-managed special global memory, and we can get
            // away with precise tracking there.
            // The base pointer is not unique, so the base permission is `SharedReadWrite`.
            MemoryKind::CallerLocation
            | MemoryKind::Machine(
                MiriMemoryKind::Global
                | MiriMemoryKind::ExternStatic
                | MiriMemoryKind::Tls
                | MiriMemoryKind::Runtime
                | MiriMemoryKind::Machine,
            ) => (extra.base_tag(id), Permission::SharedReadWrite),
            // Heap allocations we only track precisely when raw pointers are tagged, for now.
            MemoryKind::Machine(
                MiriMemoryKind::Rust | MiriMemoryKind::C | MiriMemoryKind::WinHeap,
            ) => {
                let tag =
                    if extra.tag_raw { extra.base_tag(id) } else { extra.base_tag_untagged(id) };
                (tag, Permission::SharedReadWrite)
            }
        };
        let stacks = Stacks::new(size, perm, base_tag);
        stacks.history.borrow_mut().log_creation(
            None,
            base_tag,
            alloc_range(Size::ZERO, size),
            &mut current_span,
        );
        stacks
    }

    #[inline(always)]
    pub fn memory_read<'tcx>(
        &self,
        alloc_id: AllocId,
        tag: SbTag,
        range: AllocRange,
        state: &GlobalState,
        mut current_span: CurrentSpan<'_, '_, 'tcx>,
    ) -> InterpResult<'tcx> {
        trace!(
            "read access with tag {:?}: {:?}, size {}",
            tag,
            Pointer::new(alloc_id, range.start),
            range.size.bytes()
        );
        let mut state = state.borrow_mut();
        self.for_each(range, |offset, stack, history| {
            stack.access(
                AccessKind::Read,
                tag,
                (alloc_id, range, offset),
                &mut state,
                &mut current_span,
                history,
            )
        })
    }

    #[inline(always)]
    pub fn memory_written<'tcx>(
        &mut self,
        alloc_id: AllocId,
        tag: SbTag,
        range: AllocRange,
        state: &GlobalState,
        mut current_span: CurrentSpan<'_, '_, 'tcx>,
    ) -> InterpResult<'tcx> {
        trace!(
            "write access with tag {:?}: {:?}, size {}",
            tag,
            Pointer::new(alloc_id, range.start),
            range.size.bytes()
        );
        let mut state = state.borrow_mut();
        self.for_each_mut(range, |offset, stack, history| {
            stack.access(
                AccessKind::Write,
                tag,
                (alloc_id, range, offset),
                &mut state,
                &mut current_span,
                history,
            )
        })
    }

    #[inline(always)]
    pub fn memory_deallocated<'tcx>(
        &mut self,
        alloc_id: AllocId,
        tag: SbTag,
        range: AllocRange,
        state: &GlobalState,
    ) -> InterpResult<'tcx> {
        trace!("deallocation with tag {:?}: {:?}, size {}", tag, alloc_id, range.size.bytes());
        let state = state.borrow();
        self.for_each_mut(range, |offset, stack, history| {
            stack.dealloc(tag, (alloc_id, range, offset), &state, history)
        })?;
        Ok(())
    }
}

/// Retagging/reborrowing.  There is some policy in here, such as which permissions
/// to grant for which references, and when to add protectors.
impl<'mir, 'tcx: 'mir> EvalContextPrivExt<'mir, 'tcx> for crate::MiriEvalContext<'mir, 'tcx> {}
trait EvalContextPrivExt<'mir, 'tcx: 'mir>: crate::MiriEvalContextExt<'mir, 'tcx> {
    fn reborrow(
        &mut self,
        place: &MPlaceTy<'tcx, Tag>,
        size: Size,
        kind: RefKind,
        new_tag: SbTag,
        protect: bool,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let current_span = &mut this.machine.current_span();

        let log_creation = |this: &MiriEvalContext<'mir, 'tcx>,
                            current_span: &mut CurrentSpan<'_, 'mir, 'tcx>,
                            alloc_id,
                            base_offset,
                            orig_tag|
         -> InterpResult<'tcx> {
            let extra = this.get_alloc_extra(alloc_id)?;
            let stacked_borrows =
                extra.stacked_borrows.as_ref().expect("we should have Stacked Borrows data");
            let mut alloc_history = stacked_borrows.history.borrow_mut();
            alloc_history.log_creation(
                Some(orig_tag),
                new_tag,
                alloc_range(base_offset, size),
                current_span,
            );
            if protect {
                alloc_history.log_protector(orig_tag, new_tag, current_span);
            }
            Ok(())
        };

        if size == Size::ZERO {
            // Don't update any stacks for a zero-sized access; borrow stacks are per-byte and this
            // touches no bytes so there is no stack to put this tag in.
            // However, if the pointer for this operation points at a real allocation we still
            // record where it was created so that we can issue a helpful diagnostic if there is an
            // attempt to use it for a non-zero-sized access.
            // Dangling slices are a common case here; it's valid to get their length but with raw
            // pointer tagging for example all calls to get_unchecked on them are invalid.
            if let Ok((alloc_id, base_offset, orig_tag)) = this.ptr_try_get_alloc_id(place.ptr) {
                log_creation(this, current_span, alloc_id, base_offset, orig_tag)?;
            }

            trace!(
                "reborrow of size 0: {} reference {:?} derived from {:?} (pointee {})",
                kind,
                new_tag,
                place.ptr,
                place.layout.ty,
            );
            return Ok(());
        }
        let (alloc_id, base_offset, orig_tag) = this.ptr_get_alloc_id(place.ptr)?;
        log_creation(this, current_span, alloc_id, base_offset, orig_tag)?;

        // Ensure we bail out if the pointer goes out-of-bounds (see miri#1050).
        let (alloc_size, _) =
            this.get_alloc_size_and_align(alloc_id, AllocCheck::Dereferenceable)?;
        if base_offset + size > alloc_size {
            throw_ub!(PointerOutOfBounds {
                alloc_id,
                alloc_size,
                ptr_offset: this.machine_usize_to_isize(base_offset.bytes()),
                ptr_size: size,
                msg: CheckInAllocMsg::InboundsTest
            });
        }

        let protector = if protect { Some(this.frame().extra.call_id) } else { None };
        trace!(
            "reborrow: {} reference {:?} derived from {:?} (pointee {}): {:?}, size {}",
            kind,
            new_tag,
            orig_tag,
            place.layout.ty,
            Pointer::new(alloc_id, base_offset),
            size.bytes()
        );

        // Update the stacks.
        // Make sure that raw pointers and mutable shared references are reborrowed "weak":
        // There could be existing unique pointers reborrowed from them that should remain valid!
        let perm = match kind {
            RefKind::Unique { two_phase: false }
                if place.layout.ty.is_unpin(this.tcx.at(DUMMY_SP), this.param_env()) =>
            {
                // Only if the type is unpin do we actually enforce uniqueness
                Permission::Unique
            }
            RefKind::Unique { .. } => {
                // Two-phase references and !Unpin references are treated as SharedReadWrite
                Permission::SharedReadWrite
            }
            RefKind::Raw { mutable: true } => Permission::SharedReadWrite,
            RefKind::Shared | RefKind::Raw { mutable: false } => {
                // Shared references and *const are a whole different kind of game, the
                // permission is not uniform across the entire range!
                // We need a frozen-sensitive reborrow.
                // We have to use shared references to alloc/memory_extra here since
                // `visit_freeze_sensitive` needs to access the global state.
                let extra = this.get_alloc_extra(alloc_id)?;
                let stacked_borrows =
                    extra.stacked_borrows.as_ref().expect("we should have Stacked Borrows data");
                this.visit_freeze_sensitive(place, size, |mut range, frozen| {
                    // Adjust range.
                    range.start += base_offset;
                    // We are only ever `SharedReadOnly` inside the frozen bits.
                    let perm = if frozen {
                        Permission::SharedReadOnly
                    } else {
                        Permission::SharedReadWrite
                    };
                    let item = Item { perm, tag: new_tag, protector };
                    let mut global = this.machine.stacked_borrows.as_ref().unwrap().borrow_mut();
                    stacked_borrows.for_each(range, |offset, stack, history| {
                        stack.grant(
                            orig_tag,
                            item,
                            (alloc_id, range, offset),
                            &mut *global,
                            current_span,
                            history,
                        )
                    })
                })?;
                return Ok(());
            }
        };
        // Here we can avoid `borrow()` calls because we have mutable references.
        // Note that this asserts that the allocation is mutable -- but since we are creating a
        // mutable pointer, that seems reasonable.
        let (alloc_extra, machine) = this.get_alloc_extra_mut(alloc_id)?;
        let stacked_borrows =
            alloc_extra.stacked_borrows.as_mut().expect("we should have Stacked Borrows data");
        let item = Item { perm, tag: new_tag, protector };
        let range = alloc_range(base_offset, size);
        let mut global = machine.stacked_borrows.as_ref().unwrap().borrow_mut();
        let current_span = &mut machine.current_span(); // `get_alloc_extra_mut` invalidated our old `current_span`
        stacked_borrows.for_each_mut(range, |offset, stack, history| {
            stack.grant(
                orig_tag,
                item,
                (alloc_id, range, offset),
                &mut global,
                current_span,
                history,
            )
        })?;

        Ok(())
    }

    /// Retags an indidual pointer, returning the retagged version.
    /// `mutbl` can be `None` to make this a raw pointer.
    fn retag_reference(
        &mut self,
        val: &ImmTy<'tcx, Tag>,
        kind: RefKind,
        protect: bool,
    ) -> InterpResult<'tcx, ImmTy<'tcx, Tag>> {
        let this = self.eval_context_mut();
        // We want a place for where the ptr *points to*, so we get one.
        let place = this.ref_to_mplace(val)?;
        let size = this.size_and_align_of_mplace(&place)?.map(|(size, _)| size);
        // FIXME: If we cannot determine the size (because the unsized tail is an `extern type`),
        // bail out -- we cannot reasonably figure out which memory range to reborrow.
        // See https://github.com/rust-lang/unsafe-code-guidelines/issues/276.
        let size = match size {
            Some(size) => size,
            None => return Ok(*val),
        };

        // Compute new borrow.
        let new_tag = {
            let mem_extra = this.machine.stacked_borrows.as_mut().unwrap().get_mut();
            match kind {
                // Give up tracking for raw pointers.
                RefKind::Raw { .. } if !mem_extra.tag_raw => SbTag::Untagged,
                // All other pointers are properly tracked.
                _ => SbTag::Tagged(mem_extra.new_ptr()),
            }
        };

        // Reborrow.
        this.reborrow(&place, size, kind, new_tag, protect)?;

        // Adjust pointer.
        let new_place = place.map_provenance(|p| {
            p.map(|t| {
                // TODO: Fix this eventually
                if let Tag::Concrete(t) = t {
                    Tag::Concrete(ConcreteTag { sb: new_tag, ..t })
                } else {
                    t
                }
            })
        });

        // Return new pointer.
        Ok(ImmTy::from_immediate(new_place.to_ref(this), val.layout))
    }
}

impl<'mir, 'tcx: 'mir> EvalContextExt<'mir, 'tcx> for crate::MiriEvalContext<'mir, 'tcx> {}
pub trait EvalContextExt<'mir, 'tcx: 'mir>: crate::MiriEvalContextExt<'mir, 'tcx> {
    fn retag(&mut self, kind: RetagKind, place: &PlaceTy<'tcx, Tag>) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        // Determine mutability and whether to add a protector.
        // Cannot use `builtin_deref` because that reports *immutable* for `Box`,
        // making it useless.
        fn qualify(ty: ty::Ty<'_>, kind: RetagKind) -> Option<(RefKind, bool)> {
            match ty.kind() {
                // References are simple.
                ty::Ref(_, _, Mutability::Mut) =>
                    Some((
                        RefKind::Unique { two_phase: kind == RetagKind::TwoPhase },
                        kind == RetagKind::FnEntry,
                    )),
                ty::Ref(_, _, Mutability::Not) =>
                    Some((RefKind::Shared, kind == RetagKind::FnEntry)),
                // Raw pointers need to be enabled.
                ty::RawPtr(tym) if kind == RetagKind::Raw =>
                    Some((RefKind::Raw { mutable: tym.mutbl == Mutability::Mut }, false)),
                // Boxes do not get a protector: protectors reflect that references outlive the call
                // they were passed in to; that's just not the case for boxes.
                ty::Adt(..) if ty.is_box() => Some((RefKind::Unique { two_phase: false }, false)),
                _ => None,
            }
        }

        // We only reborrow "bare" references/boxes.
        // Not traversing into fields helps with <https://github.com/rust-lang/unsafe-code-guidelines/issues/125>,
        // but might also cost us optimization and analyses. We will have to experiment more with this.
        if let Some((mutbl, protector)) = qualify(place.layout.ty, kind) {
            // Fast path.
            let val = this.read_immediate(&this.place_to_op(place)?)?;
            let val = this.retag_reference(&val, mutbl, protector)?;
            this.write_immediate(*val, place)?;
        }

        Ok(())
    }

    /// After a stack frame got pushed, retag the return place so that we are sure
    /// it does not alias with anything.
    ///
    /// This is a HACK because there is nothing in MIR that would make the retag
    /// explicit. Also see <https://github.com/rust-lang/rust/issues/71117>.
    fn retag_return_place(&mut self) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let return_place = this.frame_mut().return_place;
        if return_place.layout.is_zst() {
            // There may not be any memory here, nothing to do.
            return Ok(());
        }
        // We need this to be in-memory to use tagged pointers.
        let return_place = this.force_allocation(&return_place)?;

        // We have to turn the place into a pointer to use the existing code.
        // (The pointer type does not matter, so we use a raw pointer.)
        let ptr_layout = this.layout_of(this.tcx.mk_mut_ptr(return_place.layout.ty))?;
        let val = ImmTy::from_immediate(return_place.to_ref(this), ptr_layout);
        // Reborrow it.
        let val = this.retag_reference(
            &val,
            RefKind::Unique { two_phase: false },
            /*protector*/ true,
        )?;
        // And use reborrowed pointer for return place.
        let return_place = this.ref_to_mplace(&val)?;
        this.frame_mut().return_place = return_place.into();

        Ok(())
    }
}
