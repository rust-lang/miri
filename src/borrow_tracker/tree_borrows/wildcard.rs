use std::cmp::max;
use std::fmt::Debug;

#[cfg(feature = "expensive-consistency-checks")]
use {
    super::LocationState, crate::borrow_tracker::ProtectorKind,
    rustc_data_structures::fx::FxHashMap,
};

use super::Tree;
use super::tree::{AccessRelatedness, Location, Node};
use super::unimap::{UniIndex, UniValMap};
use crate::borrow_tracker::GlobalState;
use crate::{AccessKind, BorTag, VisitWith};

/// Represensts the maximum access level that is possible.
///
/// Note that we derive Ord and PartialOrd, so the order in which variants are listed below matters:
/// None < Read < Write. Do not change that order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum WildcardAccessLevel {
    #[default]
    None,
    Read,
    Write,
}

/// Were relative to the node the access happened from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WildcardAccessRelatedness {
    /// The access definitively happened through a local node.
    LocalAccess,
    /// The access definitively happened through a foreign node.
    ForeignAccess,
    /// We do not know if the access is foreign or local.
    EitherAccess,
}
impl WildcardAccessRelatedness {
    pub fn to_relatedness(self) -> Option<AccessRelatedness> {
        match self {
            Self::ChildAccess => Some(AccessRelatedness::LocalAccess),
            Self::ForeignAccess => Some(AccessRelatedness::ForeignAccess),
            Self::EitherAccess => None,
        }
    }
}

/// State per location per node keeping track of where relative to this
/// node exposed nodes are and what access permissions they have.
///
/// Designed to be completely determined by its parent, siblings and
/// direct children's max_local_access/max_foreign_access.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct WildcardAccessTracking {
    /// How many of this node's direct children have `max_local_access()==Write`.
    child_writes: u16,
    /// How many of this node's direct children have `max_local_access()>=Read`.
    child_reads: u16,
    /// The maximum access level that could happen from an exposed node
    /// that is foreign to this node.
    ///
    /// This is calculated as the `max()` of parents `max_foreign_access`,
    /// `exposed_as` and its siblings `max_local_access()`.
    max_foreign_access: WildcardAccessLevel,
    /// At what access level this node itself is exposed.
    exposed_as: WildcardAccessLevel,
}
impl Debug for WildcardAccessTracking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WildcardAccessTracking")
            .field("child_r/w", &(self.child_reads, self.child_writes))
            .field("foreign", &self.max_foreign_access)
            .field("exposed_as", &self.exposed_as)
            .finish()
    }
}
impl WildcardAccessTracking {
    /// The maximum access level that could happen from an exposed
    /// node that is a local to this node.
    pub fn max_local_access(&self) -> WildcardAccessLevel {
        use WildcardAccessLevel::*;
        max(
            self.exposed_as,
            if self.child_writes > 0 {
                Write
            } else if self.child_reads > 0 {
                Read
            } else {
                None
            },
        )
    }

    /// From where relative to the node with this wildcard info a read or write access could happen.
    pub fn access_relatedness(&self, kind: AccessKind) -> Option<WildcardAccessRelatedness> {
        match kind {
            AccessKind::Read => self.read_access_relatedness(),
            AccessKind::Write => self.write_access_relatedness(),
        }
    }

    /// From where relative to the node with this wildcard info a read access could happen.
    pub fn read_access_relatedness(&self) -> Option<WildcardAccessRelatedness> {
        let has_foreign = self.max_foreign_access >= WildcardAccessLevel::Read;
        let has_local = self.max_local_access() >= WildcardAccessLevel::Read;
        use WildcardAccessRelatedness as E;
        match (has_foreign, has_local) {
            (true, true) => Some(E::EitherAccess),
            (true, false) => Some(E::ForeignAccess),
            (false, true) => Some(E::LocalAccess),
            (false, false) => None,
        }
    }

    /// From where relative to the node with this wildcard info a write access could happen.
    pub fn write_access_relatedness(&self) -> Option<WildcardAccessRelatedness> {
        let has_foreign = self.max_foreign_access == WildcardAccessLevel::Write;
        let has_local = self.max_local_access() == WildcardAccessLevel::Write;
        use WildcardAccessRelatedness as E;
        match (has_foreign, has_local) {
            (true, true) => Some(E::EitherAccess),
            (true, false) => Some(E::ForeignAccess),
            (false, true) => Some(E::LocalAccess),
            (false, false) => None,
        }
    }

    /// Gets the access tracking information for a new child node of a parent with this
    /// wildcard info.
    /// The new node doesn't have any child reads/writes, but calculates max_foreign_access
    /// from its parent.
    pub fn get_new_child(&self) -> Self {
        Self {
            max_foreign_access: max(self.max_foreign_access, self.max_local_access()),
            ..Default::default()
        }
    }

    /// Pushes the nodes of `children` onto the stack who's `max_foreign_access`
    /// needs to be updated.
    ///
    /// * `children`: A list of nodes with the same parent. `children` doesn't
    ///   necessarily have to contain all children of parent, but can just be
    ///   a subset.
    ///
    /// * `child_reads`, `child_writes`: How many of `children` have `max_local_access()`
    ///   of at least `read`/`write`
    ///
    /// * `new_foreign_access`, `old_foreign_access`:
    ///   The max possible access level, that is foreign to all `children`.
    ///   This can be calculated as the max of the parent's `exposed_as()`, `max_foreign_access`
    ///   and of all `max_local_access()` of any nodes with the same parent, that are
    ///   not listed in `children`.
    ///
    ///   This access level changed from `old` to `new`, which is why we need to
    ///   update `children`.
    fn push_relevant_children(
        stack: &mut Vec<(UniIndex, WildcardAccessLevel)>,
        new_foreign_access: WildcardAccessLevel,
        old_foreign_access: WildcardAccessLevel,
        child_reads: u16,
        child_writes: u16,
        children: impl Iterator<Item = UniIndex>,

        wildcard_accesses: &UniValMap<WildcardAccessTracking>,
    ) {
        use WildcardAccessLevel::*;

        // Nothing changed so we dont need to update anything.
        if new_foreign_access == old_foreign_access {
            return;
        }

        // We need to consider that the children's `max_local_access()` affect each
        // others `max_foreign_access`, but do not affect their own `max_foreign_access`.

        // The new `max_foreign_acces` for children with `max_locala_access()==Write`.
        let write_foreign_access = max(
            new_foreign_access,
            if child_writes > 1 {
                // There exists at least one more child with exposed write access.
                // This means that a foreign write through that node is possible.
                Write
            } else if child_reads > 1 {
                // There exists at least one more child with exposed read access,
                // but no other with write access.
                // This means that a foreign read but no write through that node
                // is possible.
                Read
            } else {
                // There are no other nodes with read or write access.
                // This means no foreign writes through other children are possible.
                None
            },
        );

        // The new `max_foreign_acces` for children with `max_local_access()==Read`.
        let read_foreign_access = max(
            new_foreign_access,
            if child_writes > 0 {
                // There exists at least one child with write access (and it's not this one).
                Write
            } else if child_reads > 1 {
                // There exists at least one more child with exposed read access,
                // but no other with write access.
                Read
            } else {
                // There are no other nodes with read or write access,
                None
            },
        );

        // The new `max_foreign_acces` for children with `max_local_access()==None`.
        let none_foreign_access = max(
            new_foreign_access,
            if child_writes > 0 {
                // There exists at least one child with write access (and it's not this one).
                Write
            } else if child_reads > 0 {
                // There exists at least one child with read access (and it's not this one),
                // but none with write access.
                Read
            } else {
                // No children are exposed as read or write.
                None
            },
        );

        stack.extend(children.filter_map(|child| {
            let state = wildcard_accesses.get(child).cloned().unwrap_or_default();

            let new_foreign_access = match state.max_local_access() {
                Write => write_foreign_access,
                Read => read_foreign_access,
                None => none_foreign_access,
            };

            if new_foreign_access != state.max_foreign_access {
                Some((child, new_foreign_access))
            } else {
                Option::None
            }
        }));
    }

    /// Update the tracking information of a tree, to reflect that the node specified by `id` is
    /// now exposed with `new_exposed_as`.
    ///
    /// Propagates the Willard access information over the tree. This needs to be called every
    /// time the access level of an exposed node changes, to keep the state in sync with
    /// the rest of the tree.
    pub fn update_exposure(
        id: UniIndex,
        new_exposed_as: WildcardAccessLevel,
        nodes: &UniValMap<Node>,
        wildcard_accesses: &mut UniValMap<WildcardAccessTracking>,
    ) {
        let mut entry = wildcard_accesses.entry(id);
        let src_state = entry.or_insert(Default::default());
        let old_exposed_as = src_state.exposed_as;

        // If the exposure doesn't change, then we don't need to update anything.
        if old_exposed_as == new_exposed_as {
            return;
        }
        // Whether we are upgrading or downgrading the allowed access rights.
        let is_upgrade = old_exposed_as < new_exposed_as;

        let src_old_local_access = src_state.max_local_access();

        src_state.exposed_as = new_exposed_as;

        let src_new_local_access = src_state.max_local_access();

        // Stack of nodes for which the max_foreign_access field needs to be updated.
        let mut stack: Vec<(UniIndex, WildcardAccessLevel)> = Vec::new();

        // Update the direct children of this node.
        {
            let node = nodes.get(id).unwrap();
            Self::push_relevant_children(
                &mut stack,
                max(src_state.max_foreign_access, new_exposed_as),
                max(src_state.max_foreign_access, old_exposed_as),
                src_state.child_reads,
                src_state.child_writes,
                node.children.iter().copied(),
                wildcard_accesses,
            );
        }
        // We need to propagate the tracking info up the tree, for this we traverse up the parents.
        // We can skip propagating info to the parent and siblings of a node if its access didn't change.
        {
            // The child from which we came from.
            let mut child = id;
            // This is the `max_local_access()` of the child we came from, before this update...
            let mut old_child_access = src_old_local_access;
            // and after this update.
            let mut new_child_access = src_new_local_access;
            while let Some(parent_id) = nodes.get(child).unwrap().parent {
                let parent_node = nodes.get(parent_id).unwrap();
                let mut entry = wildcard_accesses.entry(parent_id);
                let parent_state = entry.or_insert(Default::default());

                let old_parent_state = parent_state.clone();
                use WildcardAccessLevel::*;
                // Updating this node's tracking state for its children.
                if is_upgrade {
                    if new_child_access == Write {
                        // None -> Write
                        // Read -> Write
                        parent_state.child_writes += 1;
                    }
                    if old_child_access == None {
                        // None -> Read
                        // None -> Write
                        parent_state.child_reads += 1;
                    }
                } else {
                    if old_child_access == Write {
                        // Write -> None
                        // Write -> Read
                        parent_state.child_writes -= 1;
                    }
                    if new_child_access == None {
                        // Read  -> None
                        // Write -> None
                        parent_state.child_reads -= 1;
                    }
                }

                let old_parent_local_access = old_parent_state.max_local_access();
                let new_parent_local_access = parent_state.max_local_access();

                {
                    // We need to update the `max_foreign_access` of `child`'s siblings.
                    // For this we can reuse the `push_relevant_children` function.
                    //
                    // We pass it just the siblings without child itself. Since `child`'s
                    // `max_local_access()` is foreign to all of its siblings we can pass
                    // it as part of the foreign access.

                    // it doesnt matter wether we read these from old or new
                    let constant_factors =
                        max(parent_state.exposed_as, parent_state.max_foreign_access);

                    // `state` contains the correct child_writes/reads counts for just the
                    // siblings excluding `child`.
                    let state = if is_upgrade { &old_parent_state } else { parent_state };

                    Self::push_relevant_children(
                        &mut stack,
                        max(constant_factors, new_child_access),
                        max(constant_factors, old_child_access),
                        state.child_reads,
                        state.child_writes,
                        parent_node.children.iter().copied().filter(|id| child != *id),
                        wildcard_accesses,
                    );
                }
                if old_parent_local_access == new_parent_local_access {
                    // We didn't change `max_local_access()` for parent, so we don't need to propagate further upwards.
                    break;
                }

                old_child_access = old_parent_local_access;
                new_child_access = new_parent_local_access;
                child = parent_id;
            }
        }
        // Traverses up the tree to update max_foreign_access fields of children and cousins who need to be updated.
        while let Some((id, new_access)) = stack.pop() {
            let node = nodes.get(id).unwrap();
            let mut entry = wildcard_accesses.entry(id);
            let state = entry.or_insert(Default::default());

            let old_access = state.max_foreign_access;
            state.max_foreign_access = new_access;

            Self::push_relevant_children(
                &mut stack,
                max(state.exposed_as, new_access),
                max(state.exposed_as, old_access),
                state.child_reads,
                state.child_writes,
                node.children.iter().copied(),
                wildcard_accesses,
            );
        }

        // Calling `update_exposure()` should always result in an internally consistent state.
        #[cfg(feature = "expensive-consistency-checks")]
        Self::verify_internal_consistency(id, nodes, wildcard_accesses);
    }
}

impl Tree {
    /// Marks the tag as exposed & updates the wildcard tracking data structure
    /// to represent its access level.
    pub fn expose_tag(&mut self, tag: BorTag, global: &GlobalState) {
        let id = self.tag_mapping.get(&tag).unwrap();
        let node = self.nodes.get_mut(id).unwrap();
        node.is_exposed = true;
        let node = self.nodes.get(id).unwrap();
        let protected_tags = &global.borrow().protected_tags;
        let protected = protected_tags.contains_key(&tag);
        // TODO: Only initialize neccessary ranges.
        for (_, Location { perms, wildcard_accesses }) in self.rperms.iter_mut_all() {
            let perm = *perms.entry(id).or_insert(node.default_location_state());

            let access_type = perm.permission().strongest_allowed_child_access(protected);
            WildcardAccessTracking::update_exposure(
                id,
                access_type,
                &self.nodes,
                wildcard_accesses,
            );
        }
    }
    pub(super) fn visit_wildcards(&self, visit: &mut VisitWith<'_>) {
        for (_, Location { perms, .. }) in self.rperms.iter_all() {
            let mut stack = vec![self.root];
            let mut has_exposed = false;
            while let Some(id) = stack.pop() {
                let node = self.nodes.get(id).unwrap();

                if node.is_exposed
                    && let Some(perm) = perms.get(id)
                {
                    // If the node is already protected, then it gets already visited
                    // through the protector. So we can assume `protected=false`.
                    let access_level = perm.permission().strongest_allowed_child_access(false);
                    if access_level != WildcardAccessLevel::None {
                        visit(None, Some(node.tag))
                    }
                }
                has_exposed |= node.is_exposed;
                stack.extend(&node.children);
            }
            // If there are no exposed nodes, then we don't need to check other
            // locations of this allocation.
            if !has_exposed {
                break;
            }
        }
    }
}

#[cfg(feature = "expensive-consistency-checks")]
impl Tree {
    /// Checks that the wildcard tracking datastructure is internally consistent and has
    /// the correct `exposed_as` values.
    pub fn verify_wildcard_consistency(&self, global: &GlobalState) {
        for (_, Location { perms, wildcard_accesses }) in self.rperms.iter_all() {
            WildcardAccessTracking::verify_internal_consistency(
                self.root,
                &self.nodes,
                wildcard_accesses,
            );

            WildcardAccessTracking::verify_external_consistency(
                self.root,
                &self.nodes,
                perms,
                wildcard_accesses,
                &global.borrow().protected_tags,
            );
        }
    }
}

#[cfg(feature = "expensive-consistency-checks")]
impl WildcardAccessTracking {
    /// Verifies that the access tracking state is self consistent.
    ///
    /// Panics if invalid.
    pub fn verify_internal_consistency(
        id: UniIndex,
        nodes: &UniValMap<Node>,
        wildcard_accesses: &UniValMap<WildcardAccessTracking>,
    ) {
        // Find the root node.
        let mut root = id;
        while let Some(parent) = nodes.get(root).unwrap().parent {
            root = parent;
        }

        // Checks if accesses is empty.
        if !wildcard_accesses.is_initialized() {
            return;
        }

        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let node = nodes.get(id).unwrap();
            stack.extend(node.children.iter());

            let access = wildcard_accesses.get(id).unwrap();

            let expected_max_foreign_access = if let Some(parent) = node.parent {
                let parent_node = nodes.get(parent).unwrap();
                let parent_state = wildcard_accesses.get(parent).unwrap();

                let max_sibling_access = parent_node
                    .children
                    .iter()
                    .copied()
                    .filter(|child| *child != id)
                    .map(|child| {
                        let state = wildcard_accesses.get(child).unwrap();
                        state.max_local_access()
                    })
                    .fold(WildcardAccessLevel::None, max);

                max_sibling_access.max(parent_state.max_foreign_access).max(parent_state.exposed_as)
            } else {
                WildcardAccessLevel::None
            };

            let child_accesses = node.children.iter().copied().map(|child| {
                let state = wildcard_accesses.get(child).unwrap();
                state.max_local_access()
            });
            let expected_child_reads =
                child_accesses.clone().filter(|a| *a >= WildcardAccessLevel::Read).count();
            let expected_child_writes =
                child_accesses.filter(|a| *a >= WildcardAccessLevel::Write).count();

            assert_eq!(
                expected_max_foreign_access, access.max_foreign_access,
                "expected {:?}'s (id:{id:?}) max_foreign_access to be {:?} instead of {:?}",
                node.tag, expected_max_foreign_access, access.max_foreign_access
            );
            let child_reads: usize = access.child_reads.into();
            assert_eq!(
                expected_child_reads, child_reads,
                "expected {:?}'s (id:{id:?}) child_reads to be {} instead of {}",
                node.tag, expected_child_reads, child_reads
            );
            let child_writes: usize = access.child_writes.into();
            assert_eq!(
                expected_child_writes, child_writes,
                "expected {:?}'s (id:{id:?}) child_writes to be {} instead of {}",
                node.tag, expected_child_writes, child_writes
            );
        }
    }

    /// Ensures that the access tracking state is consistent with the rest of the tree.
    /// This means that for every node the `exposed_as` field is equal to the strongest
    /// possible access through that node.
    pub fn verify_external_consistency(
        id: UniIndex,
        nodes: &UniValMap<Node>,
        perms: &UniValMap<LocationState>,
        wildcard_accesses: &UniValMap<WildcardAccessTracking>,
        protected_tags: &FxHashMap<BorTag, ProtectorKind>,
    ) {
        // Find the root node.
        let mut root = id;
        while let Some(parent) = nodes.get(root).unwrap().parent {
            root = parent;
        }

        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let node = nodes.get(id).unwrap();
            if node.is_exposed {
                let state = wildcard_accesses.get(id).unwrap();
                let perm = perms.get(id).unwrap();
                let access_level = perm
                    .permission()
                    .strongest_allowed_child_access(protected_tags.contains_key(&node.tag));
                assert_eq!(
                    access_level, state.exposed_as,
                    "tag {:?} (id:{id:?}) should be exposed as {access_level:?} but is exposed as {:?}",
                    node.tag, state.exposed_as
                );
            } else {
                if let Some(state) = wildcard_accesses.get(id)
                    && state.exposed_as != WildcardAccessLevel::None
                {
                    panic!(
                        "tag {:?} (id:{id:?}) should be exposed as None but is exposed as {:?}",
                        node.tag, state.exposed_as
                    )
                }
            }
            stack.extend(&node.children);
        }
    }
}
