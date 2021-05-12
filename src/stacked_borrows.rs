//! Implements "Stacked Borrows".  See <https://github.com/rust-lang/unsafe-code-guidelines/blob/master/wip/stacked-borrows.md>
//! for further information.

use std::cell::RefCell;
use std::fmt;
use std::num::NonZeroU64;
use std::rc::Rc;

use log::trace;

use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use rustc_hir::Mutability;
use rustc_middle::mir::RetagKind;
use rustc_middle::ty;
use rustc_target::abi::{Align, LayoutOf, Size};

use crate::*;

pub type PtrId = NonZeroU64;
pub type CallId = NonZeroU64;
pub type AllocExtra = Stacks;

/// Tracking pointer provenance
#[derive(Copy, Clone, Hash, PartialEq, Eq)]
pub enum Tag {
    Tagged(PtrId),
    Untagged,
}

impl fmt::Debug for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tag::Tagged(id) => write!(f, "<{}>", id),
            Tag::Untagged => write!(f, "<untagged>"),
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
    tag: Tag,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    /// Used *mostly* as a stack; never empty.
    /// Invariants:
    /// * Above a `SharedReadOnly` there can only be more `SharedReadOnly`.
    /// * Except for `Untagged`, no tag occurs in the stack more than once.
    borrows: Vec<Item>,
}

/// Extra per-allocation state.
#[derive(Clone, Debug)]
pub struct Stacks {
    // Even reading memory can have effects on the stack, so we need a `RefCell` here.
    stacks: RefCell<RangeMap<Stack>>,
    // Pointer to global state
    global: MemoryExtra,
}

/// Extra global state, available to the memory access hooks.
#[derive(Debug)]
pub struct GlobalState {
    /// Next unused pointer ID (tag).
    next_ptr_id: PtrId,
    /// Table storing the "base" tag for each allocation.
    /// The base tag is the one used for the initial pointer.
    /// We need this in a separate table to handle cyclic statics.
    base_ptr_ids: FxHashMap<AllocId, Tag>,
    /// Next unused call ID (for protectors).
    next_call_id: CallId,
    /// Those call IDs corresponding to functions that are still running.
    active_calls: FxHashSet<CallId>,
    /// The pointer id to trace
    tracked_pointer_tag: Option<PtrId>,
    /// The call id to trace
    tracked_call_id: Option<CallId>,
    /// Whether to track raw pointers.
    track_raw: bool,
}
/// Memory extra state gives us interior mutable access to the global state.
pub type MemoryExtra = Rc<RefCell<GlobalState>>;

/// Indicates which kind of access is being performed.
#[derive(Copy, Clone, Hash, PartialEq, Eq)]
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
impl GlobalState {
    pub fn new(
        tracked_pointer_tag: Option<PtrId>,
        tracked_call_id: Option<CallId>,
        track_raw: bool,
    ) -> Self {
        GlobalState {
            next_ptr_id: NonZeroU64::new(1).unwrap(),
            base_ptr_ids: FxHashMap::default(),
            next_call_id: NonZeroU64::new(1).unwrap(),
            active_calls: FxHashSet::default(),
            tracked_pointer_tag,
            tracked_call_id,
            track_raw,
        }
    }

    fn new_ptr(&mut self) -> PtrId {
        let id = self.next_ptr_id;
        if Some(id) == self.tracked_pointer_tag {
            register_diagnostic(NonHaltingDiagnostic::CreatedPointerTag(id));
        }
        self.next_ptr_id = NonZeroU64::new(id.get() + 1).unwrap();
        id
    }

    pub fn new_call(&mut self) -> CallId {
        let id = self.next_call_id;
        trace!("new_call: Assigning ID {}", id);
        if Some(id) == self.tracked_call_id {
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

    pub fn global_base_ptr(&mut self, id: AllocId) -> Tag {
        self.base_ptr_ids.get(&id).copied().unwrap_or_else(|| {
            let tag = Tag::Tagged(self.new_ptr());
            trace!("New allocation {:?} has base tag {:?}", id, tag);
            self.base_ptr_ids.try_insert(id, tag).unwrap();
            tag
        })
    }
}

/// Error reporting
fn err_sb_ub(msg: String) -> InterpError<'static> {
    err_machine_stop!(TerminationInfo::ExperimentalUb {
        msg,
        url: format!(
            "https://github.com/rust-lang/unsafe-code-guidelines/blob/master/wip/stacked-borrows.md"
        ),
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
    fn find_granting(&self, access: AccessKind, tag: Tag) -> Option<usize> {
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
    fn check_protector(item: &Item, tag: Option<Tag>, global: &GlobalState) -> InterpResult<'tcx> {
        if let Tag::Tagged(id) = item.tag {
            if Some(id) == global.tracked_pointer_tag {
                register_diagnostic(NonHaltingDiagnostic::PoppedPointerTag(item.clone()));
            }
        }
        if let Some(call) = item.protector {
            if global.is_active(call) {
                if let Some(tag) = tag {
                    Err(err_sb_ub(format!(
                        "not granting access to tag {:?} because incompatible item is protected: {:?}",
                        tag, item
                    )))?
                } else {
                    Err(err_sb_ub(format!("deallocating while item is protected: {:?}", item)))?
                }
            }
        }
        Ok(())
    }

    /// Test if a memory `access` using pointer tagged `tag` is granted.
    /// If yes, return the index of the item that granted it.
    fn access(
        &mut self,
        access: AccessKind,
        ptr: Pointer<Tag>,
        global: &GlobalState,
    ) -> InterpResult<'tcx> {
        // Two main steps: Find granting item, remove incompatible items above.

        // Step 1: Find granting item.
        let granting_idx = self.find_granting(access, ptr.tag).ok_or_else(|| {
            err_sb_ub(format!(
                "no item granting {} to tag {:?} at {} found in borrow stack.",
                access,
                ptr.tag,
                ptr.erase_tag(),
            ))
        })?;

        // Step 2: Remove incompatible items above them.  Make sure we do not remove protected
        // items.  Behavior differs for reads and writes.
        if access == AccessKind::Write {
            // Remove everything above the write-compatible items, like a proper stack. This makes sure read-only and unique
            // pointers become invalid on write accesses (ensures F2a, and ensures U2 for write accesses).
            let first_incompatible_idx = self.find_first_write_incompatible(granting_idx);
            for item in self.borrows.drain(first_incompatible_idx..).rev() {
                trace!("access: popping item {:?}", item);
                Stack::check_protector(&item, Some(ptr.tag), global)?;
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
                    Stack::check_protector(item, Some(ptr.tag), global)?;
                    item.perm = Permission::Disabled;
                }
            }
        }

        // Done.
        Ok(())
    }

    /// Deallocate a location: Like a write access, but also there must be no
    /// active protectors at all because we will remove all items.
    fn dealloc(&mut self, ptr: Pointer<Tag>, global: &GlobalState) -> InterpResult<'tcx> {
        // Step 1: Find granting item.
        self.find_granting(AccessKind::Write, ptr.tag).ok_or_else(|| {
            err_sb_ub(format!(
                "no item granting write access for deallocation to tag {:?} at {} found in borrow stack",
                ptr.tag, ptr.erase_tag(),
            ))
        })?;

        // Step 2: Remove all items.  Also checks for protectors.
        for item in self.borrows.drain(..).rev() {
            Stack::check_protector(&item, None, global)?;
        }

        Ok(())
    }

    /// Derive a new pointer from one with the given tag.
    /// `weak` controls whether this operation is weak or strong: weak granting does not act as
    /// an access, and they add the new item directly on top of the one it is derived
    /// from instead of all the way at the top of the stack.
    fn grant(
        &mut self,
        derived_from: Pointer<Tag>,
        new: Item,
        global: &GlobalState,
    ) -> InterpResult<'tcx> {
        // Figure out which access `perm` corresponds to.
        let access =
            if new.perm.grants(AccessKind::Write) { AccessKind::Write } else { AccessKind::Read };
        // Now we figure out which item grants our parent (`derived_from`) this kind of access.
        // We use that to determine where to put the new item.
        let granting_idx = self.find_granting(access, derived_from.tag)
            .ok_or_else(|| err_sb_ub(format!(
                "trying to reborrow for {:?} at {}, but parent tag {:?} does not have an appropriate item in the borrow stack",
                new.perm, derived_from.erase_tag(), derived_from.tag,
            )))?;

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
            self.access(access, derived_from, global)?;

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
    fn new(size: Size, perm: Permission, tag: Tag, extra: MemoryExtra) -> Self {
        let item = Item { perm, tag, protector: None };
        let stack = Stack { borrows: vec![item] };

        Stacks { stacks: RefCell::new(RangeMap::new(size, stack)), global: extra }
    }

    /// Call `f` on every stack in the range.
    fn for_each(
        &self,
        ptr: Pointer<Tag>,
        size: Size,
        f: impl Fn(Pointer<Tag>, &mut Stack, &GlobalState) -> InterpResult<'tcx>,
    ) -> InterpResult<'tcx> {
        let global = self.global.borrow();
        let mut stacks = self.stacks.borrow_mut();
        for (offset, stack) in stacks.iter_mut(ptr.offset, size) {
            let mut cur_ptr = ptr;
            cur_ptr.offset = offset;
            f(cur_ptr, stack, &*global)?;
        }
        Ok(())
    }
}

/// Glue code to connect with Miri Machine Hooks
impl Stacks {
    pub fn new_allocation(
        id: AllocId,
        size: Size,
        extra: MemoryExtra,
        kind: MemoryKind<MiriMemoryKind>,
    ) -> (Self, Tag) {
        let (tag, perm) = match kind {
            // New unique borrow. This tag is not accessible by the program,
            // so it will only ever be used when using the local directly (i.e.,
            // not through a pointer). That is, whenever we directly write to a local, this will pop
            // everything else off the stack, invalidating all previous pointers,
            // and in particular, *all* raw pointers.
            MemoryKind::Stack => (Tag::Tagged(extra.borrow_mut().new_ptr()), Permission::Unique),
            // `Global` memory can be referenced by global pointers from `tcx`.
            // Thus we call `global_base_ptr` such that the global pointers get the same tag
            // as what we use here.
            // `ExternStatic` is used for extern statics, and thus must also be listed here.
            // `Env` we list because we can get away with precise tracking there.
            // The base pointer is not unique, so the base permission is `SharedReadWrite`.
            MemoryKind::Machine(
                MiriMemoryKind::Global
                | MiriMemoryKind::ExternStatic
                | MiriMemoryKind::Tls
                | MiriMemoryKind::Env,
            ) => (extra.borrow_mut().global_base_ptr(id), Permission::SharedReadWrite),
            // Everything else we handle like raw pointers for now.
            _ => {
                let mut extra = extra.borrow_mut();
                let tag =
                    if extra.track_raw { Tag::Tagged(extra.new_ptr()) } else { Tag::Untagged };
                (tag, Permission::SharedReadWrite)
            }
        };
        (Stacks::new(size, perm, tag, extra), tag)
    }

    #[inline(always)]
    pub fn memory_read<'tcx>(&self, ptr: Pointer<Tag>, size: Size) -> InterpResult<'tcx> {
        trace!("read access with tag {:?}: {:?}, size {}", ptr.tag, ptr.erase_tag(), size.bytes());
        self.for_each(ptr, size, |ptr, stack, global| stack.access(AccessKind::Read, ptr, global))
    }

    #[inline(always)]
    pub fn memory_written<'tcx>(&mut self, ptr: Pointer<Tag>, size: Size) -> InterpResult<'tcx> {
        trace!("write access with tag {:?}: {:?}, size {}", ptr.tag, ptr.erase_tag(), size.bytes());
        self.for_each(ptr, size, |ptr, stack, global| stack.access(AccessKind::Write, ptr, global))
    }

    #[inline(always)]
    pub fn memory_deallocated<'tcx>(
        &mut self,
        ptr: Pointer<Tag>,
        size: Size,
    ) -> InterpResult<'tcx> {
        trace!("deallocation with tag {:?}: {:?}, size {}", ptr.tag, ptr.erase_tag(), size.bytes());
        self.for_each(ptr, size, |ptr, stack, global| stack.dealloc(ptr, global))
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
        new_tag: Tag,
        protect: bool,
    ) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let protector = if protect { Some(this.frame().extra.call_id) } else { None };
        let ptr = place.ptr.assert_ptr();
        trace!(
            "reborrow: {} reference {:?} derived from {:?} (pointee {}): {:?}, size {}",
            kind,
            new_tag,
            ptr.tag,
            place.layout.ty,
            ptr.erase_tag(),
            size.bytes()
        );

        // Get the allocation. It might not be mutable, so we cannot use `get_mut`.
        let extra = &this.memory.get_raw(ptr.alloc_id)?.extra;
        let stacked_borrows =
            extra.stacked_borrows.as_ref().expect("we should have Stacked Borrows data");
        // Update the stacks.
        // Make sure that raw pointers and mutable shared references are reborrowed "weak":
        // There could be existing unique pointers reborrowed from them that should remain valid!
        let perm = match kind {
            RefKind::Unique { two_phase: false } => Permission::Unique,
            RefKind::Unique { two_phase: true } => Permission::SharedReadWrite,
            RefKind::Raw { mutable: true } => Permission::SharedReadWrite,
            RefKind::Shared | RefKind::Raw { mutable: false } => {
                // Shared references and *const are a whole different kind of game, the
                // permission is not uniform across the entire range!
                // We need a frozen-sensitive reborrow.
                return this.visit_freeze_sensitive(place, size, |cur_ptr, size, frozen| {
                    // We are only ever `SharedReadOnly` inside the frozen bits.
                    let perm = if frozen {
                        Permission::SharedReadOnly
                    } else {
                        Permission::SharedReadWrite
                    };
                    let item = Item { perm, tag: new_tag, protector };
                    stacked_borrows.for_each(cur_ptr, size, |cur_ptr, stack, global| {
                        stack.grant(cur_ptr, item, global)
                    })
                });
            }
        };
        let item = Item { perm, tag: new_tag, protector };
        stacked_borrows.for_each(ptr, size, |ptr, stack, global| stack.grant(ptr, item, global))
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
        // `reborrow` relies on getting a `Pointer` and everything being in-bounds,
        // so let's ensure that. However, we do not care about alignment.
        // We can see dangling ptrs in here e.g. after a Box's `Unique` was
        // updated using "self.0 = ..." (can happen in Box::from_raw) so we cannot ICE; see miri#1050.
        let place = this.mplace_access_checked(place, Some(Align::from_bytes(1).unwrap()))?;
        // Nothing to do for ZSTs. We use `is_bits` here because we *do* need to retag even ZSTs
        // when there actually is a tag (to avoid inheriting a tag that would let us access more
        // than 0 bytes).
        if size == Size::ZERO && place.ptr.is_bits() {
            return Ok(*val);
        }

        // Compute new borrow.
        let new_tag = {
            let mut mem_extra = this.memory.extra.stacked_borrows.as_ref().unwrap().borrow_mut();
            match kind {
                // Give up tracking for raw pointers.
                RefKind::Raw { .. } if !mem_extra.track_raw => Tag::Untagged,
                // All other pointers are properly tracked.
                _ => Tag::Tagged(mem_extra.new_ptr()),
            }
        };

        // Reborrow.
        this.reborrow(&place, size, kind, new_tag, protect)?;
        let new_place = place.replace_tag(new_tag);

        // Return new pointer.
        Ok(ImmTy::from_immediate(new_place.to_ref(), val.layout))
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
                ty::Ref(_, _, Mutability::Mut) => Some((
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
    /// explicit. Also see https://github.com/rust-lang/rust/issues/71117.
    fn retag_return_place(&mut self) -> InterpResult<'tcx> {
        let this = self.eval_context_mut();
        let return_place = if let Some(return_place) = this.frame_mut().return_place {
            return_place
        } else {
            // No return place, nothing to do.
            return Ok(());
        };
        if return_place.layout.is_zst() {
            // There may not be any memory here, nothing to do.
            return Ok(());
        }
        // We need this to be in-memory to use tagged pointers.
        let return_place = this.force_allocation(&return_place)?;

        // We have to turn the place into a pointer to use the existing code.
        // (The pointer type does not matter, so we use a raw pointer.)
        let ptr_layout = this.layout_of(this.tcx.mk_mut_ptr(return_place.layout.ty))?;
        let val = ImmTy::from_immediate(return_place.to_ref(), ptr_layout);
        // Reborrow it.
        let val = this.retag_reference(
            &val,
            RefKind::Unique { two_phase: false },
            /*protector*/ true,
        )?;
        // And use reborrowed pointer for return place.
        let return_place = this.ref_to_mplace(&val)?;
        this.frame_mut().return_place = Some(return_place.into());

        Ok(())
    }
}
