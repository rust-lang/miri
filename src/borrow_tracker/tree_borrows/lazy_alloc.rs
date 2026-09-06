//! Lazy allocation of Tree Borrows trees, gated behind the `lazy-alloc` feature.
//!
//! Most allocations are never accessed through a pointer that Tree Borrows
//! tracks, remaining as singleton nodes, so building a [`Tree`] for every
//! allocation is wasted work. With this feature enabled, the tree is only
//! initialized once the first child node is created.
//!
//! Each method here is a thin wrapper that forwards to the
//! corresponding [`Tree`] method once the tree exists.

use std::cell::Cell;

use rustc_abi::Size;
use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use rustc_span::Span;

use super::Tree;
use super::diagnostics::AccessCause;
use super::perms::Permission;
use super::tree::LocationState;
use crate::borrow_tracker::{AccessKind, GlobalState, GlobalStateInner, ProtectorKind};
use crate::*;

/// Per-allocation state for Tree Borrows, with the tree created on demand.
#[derive(Debug, Clone)]
pub enum LazyTree {
    /// The tree does not exist yet. We keep everything needed to build it, plus
    /// `exposed`, which records if `expose_tag` was called before the tree was built.
    ///
    /// The root tag is taken here rather than in `init` so that materializing the tree needs
    /// no access to the global state; the eager `Tree::new_allocation` takes it at this point
    /// too, and `adjust_alloc_root_pointer` asks for it for essentially every allocation
    /// anyway, so nothing is gained by deferring it.
    Uninit {
        root_tag: BorTag,
        size: Size,
        span: Span,
        exposed: bool,
    },
    Init(Box<Tree>),
}

impl LazyTree {
    /// Create the tree if it does not exist yet. The common case of an already-initialized tree
    /// must be cheap, so only the check is here.
    #[inline]
    pub fn ensure_init(&mut self) {
        if matches!(self, LazyTree::Uninit { .. }) {
            self.init();
        }
    }

    /// The cold half of [`LazyTree::ensure_init`], kept out of line so that it does not bloat
    /// the retag path.
    #[cold]
    #[inline(never)]
    fn init(&mut self) {
        let LazyTree::Uninit { root_tag, size, span, exposed } = *self else { return };
        let mut tree = Tree::new(root_tag, size, span);
        if exposed {
            tree.expose_tag(root_tag, false);
        }
        *self = LazyTree::Init(Box::new(tree));
    }

    /// The tree, if it has been materialized.
    fn get_mut(&mut self) -> Option<&mut Tree> {
        match self {
            LazyTree::Init(tree) => Some(tree),
            LazyTree::Uninit { .. } => None,
        }
    }
}

/// Wrappers for the `Tree` methods in `super`.
impl<'tcx> LazyTree {
    /// Create a new dummy allocation.
    pub fn new_allocation(
        id: AllocId,
        size: Size,
        state: &mut GlobalStateInner,
        _kind: MemoryKind,
        machine: &MiriMachine<'tcx>,
    ) -> Self {
        let root_tag = state.root_ptr_tag(id, machine); // Fresh tag for the root
        LazyTree::Uninit {
            root_tag,
            size,
            span: machine.current_user_relevant_span(),
            exposed: false,
        }
    }

    /// Wrapper for `Tree::before_memory_access`.
    pub fn before_memory_access(
        &mut self,
        access_kind: AccessKind,
        alloc_id: AllocId,
        prov: ProvenanceExtra,
        range: AllocRange,
        machine: &MiriMachine<'tcx>,
    ) -> InterpResult<'tcx> {
        match self.get_mut() {
            Some(tree) => tree.before_memory_access(access_kind, alloc_id, prov, range, machine),
            None => interp_ok(()),
        }
    }

    /// Wrapper for `Tree::before_memory_deallocation`.
    pub fn before_memory_deallocation(
        &mut self,
        alloc_id: AllocId,
        prov: ProvenanceExtra,
        size: Size,
        machine: &MiriMachine<'tcx>,
    ) -> InterpResult<'tcx> {
        match self.get_mut() {
            Some(tree) => tree.before_memory_deallocation(alloc_id, prov, size, machine),
            None => interp_ok(()),
        }
    }

    /// Wrapper for `Tree::release_protector`.
    pub fn release_protector(
        &mut self,
        machine: &MiriMachine<'tcx>,
        global: &GlobalState,
        tag: BorTag,
        alloc_id: AllocId, // diagnostics
    ) -> InterpResult<'tcx> {
        match self.get_mut() {
            Some(tree) => tree.release_protector(machine, global, tag, alloc_id),
            None => interp_ok(()),
        }
    }

    /// Wrapper for `Tree::perform_access`.
    pub fn perform_access(
        &mut self,
        prov: ProvenanceExtra,
        access_range: AllocRange,
        access_kind: AccessKind,
        access_cause: AccessCause,
        global: &GlobalState,
        alloc_id: AllocId,
        span: Span,
        visits_since_gc: &Cell<u32>,
    ) -> InterpResult<'tcx> {
        match self.get_mut() {
            Some(tree) =>
                tree.perform_access(
                    prov,
                    access_range,
                    access_kind,
                    access_cause,
                    global,
                    alloc_id,
                    span,
                    visits_since_gc,
                ),
            None => interp_ok(()),
        }
    }

    /// Wrapper for `Tree::new_child`.
    pub(super) fn new_child(
        &mut self,
        base_offset: Size,
        parent_prov: ProvenanceExtra,
        new_tag: BorTag,
        inside_perms: DedupRangeMap<LocationState>,
        outside_perm: Permission,
        protected: bool,
        span: Span,
    ) -> InterpResult<'tcx> {
        match self.get_mut() {
            Some(tree) =>
                tree.new_child(
                    base_offset,
                    parent_prov,
                    new_tag,
                    inside_perms,
                    outside_perm,
                    protected,
                    span,
                ),
            // `tb_retag_reference` calls `ensure_init` before this, so by the
            // time we get here the tree always exists.
            None => unreachable!("`new_child` called on an uninitialized tree"),
        }
    }

    /// Wrapper for `Tree::remove_unreachable_tags`.
    /// Returns `(live, dead)` node counts; an absent tree has neither.
    pub fn remove_unreachable_tags(
        &mut self,
        live_tags: &FxHashSet<BorTag>,
        min_nodes: usize,
        max_compact: usize,
    ) -> (usize, usize) {
        match self.get_mut() {
            Some(tree) => tree.remove_unreachable_tags(live_tags, min_nodes, max_compact),
            None => (0, 0),
        }
    }

    /// Wrapper for `Tree::expose_tag`.
    pub fn expose_tag(&mut self, tag: BorTag, protected: bool) {
        match self {
            LazyTree::Init(tree) => tree.expose_tag(tag, protected),
            // Record the exposure; `ensure_init` applies it to the root once
            // the tree is built.
            LazyTree::Uninit { exposed, .. } => *exposed = true,
        }
    }

    /// Wrapper for `Tree::print_tree`.
    pub fn print_tree(
        &self,
        protected_tags: &FxHashMap<BorTag, ProtectorKind>,
        show_unnamed: bool,
    ) -> InterpResult<'tcx> {
        match self {
            LazyTree::Init(tree) => tree.print_tree(protected_tags, show_unnamed),
            LazyTree::Uninit { .. } => interp_ok(()),
        }
    }

    /// Wrapper for `Tree::give_pointer_debug_name`.
    pub fn give_pointer_debug_name(
        &mut self,
        tag: BorTag,
        nth_parent: u8,
        name: &str,
    ) -> InterpResult<'tcx> {
        match self.get_mut() {
            Some(tree) => tree.give_pointer_debug_name(tag, nth_parent, name),
            None => interp_ok(()),
        }
    }
}

/// Wrapper for `Tree::visit_provenance`.
impl VisitProvenance for LazyTree {
    fn visit_provenance(&self, visit: &mut VisitWith<'_>) {
        // An uninitialized tree holds no tags that need to be kept alive.
        if let LazyTree::Init(tree) = self {
            tree.visit_provenance(visit);
        }
    }
}
