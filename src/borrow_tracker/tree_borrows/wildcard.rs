use std::fmt::Debug;

use super::Tree;
use super::tree::{AccessRelatedness, Node};
use super::unimap::{UniIndex, UniValMap};
use crate::BorTag;
use crate::borrow_tracker::AccessKind;
#[cfg(feature = "expensive-consistency-checks")]
use crate::borrow_tracker::GlobalState;

/// Represents the maximum access level that is possible.
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
impl WildcardAccessLevel {
    /// Weather this access kind is allowed at this level.
    pub fn allows(self, kind: AccessKind) -> bool {
        let required_level = match kind {
            AccessKind::Read => Self::Read,
            AccessKind::Write => Self::Write,
        };
        required_level <= self
    }
}

/// Where the access happened relative to the current node.
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
            Self::LocalAccess => Some(AccessRelatedness::LocalAccess),
            Self::ForeignAccess => Some(AccessRelatedness::ForeignAccess),
            Self::EitherAccess => None,
        }
    }
}
/// Stores how nodes are related to exposed nodes,
///
/// Only gets allocated if we add an exposed node.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WildcardData(UniValMap<WildcardState>);
impl WildcardData {
    /// Returns the relatedness of a wildcard access to a node.
    ///
    /// This function only considers a single subtree. If the current subtree does not contain
    /// any valid exposed nodes then the function return `None`.
    ///
    /// * `root`: The root of the subtree the node belongs to.
    /// * `id`: The id of the node.
    /// * `kind`: The kind of the wildcard access.
    pub fn access_relatedness(
        &self,
        root: UniIndex,
        id: UniIndex,
        kind: AccessKind,
    ) -> Option<WildcardAccessRelatedness> {
        // All nodes in the tree are local to the root, so we can use the root to get the total
        // number of valid exposed nodes in the tree.
        let root = self.0.get(root).cloned().unwrap_or_default();
        let node = self.0.get(id).cloned().unwrap_or_default();

        let (root, node) = match kind {
            AccessKind::Read => (root.local_reads, node.local_reads),
            AccessKind::Write => (root.local_writes, node.local_writes),
        };

        if root == 0 {
            // we return None if the tree does not contain any valid exposed nodes.
            None
        } else {
            Some(if root == node {
                // If all valid exposed nodes are local to this node then the access is local.
                WildcardAccessRelatedness::LocalAccess
            } else if node == 0 {
                // If the node does not have any exposed nodes as children then the access is foreign.
                WildcardAccessRelatedness::ForeignAccess
            } else {
                // If some but not all of the valid exposed nodes are local then we cannot determine the correct relatedness.
                WildcardAccessRelatedness::EitherAccess
            })
        }
    }
    /// Update the tracking information of a tree, to reflect that the node specified by `id` is
    /// now exposed with `new_exposed_as`.
    ///
    /// Propagates the Willard access information over the tree. This needs to be called every
    /// time the access level of an exposed node changes, to keep the state in sync with
    /// the rest of the tree.
    ///
    /// * `from`: The previous access level of the exposed node.
    ///   Set to `None` if the node was not exposed before.
    /// * `to`: The new access level.
    pub fn update_exposure(
        &mut self,
        nodes: &UniValMap<Node>,
        id: UniIndex,
        from: WildcardAccessLevel,
        to: WildcardAccessLevel,
    ) {
        // If the exposure doesn't change, then we don't need to update anything.
        if from == to {
            return;
        }

        let mut next_id = Some(id);
        while let Some(id) = next_id {
            let node = nodes.get(id).unwrap();
            let mut wc = self.0.entry(id);
            let wc = wc.or_insert(Default::default());

            use WildcardAccessLevel::*;
            match (from, to) {
                (None | Read, Write) => wc.local_writes += 1,
                (Write, None | Read) => wc.local_writes -= 1,
                _ => {}
            }
            match (from, to) {
                (None, Read | Write) => wc.local_reads += 1,
                (Read | Write, None) => wc.local_reads -= 1,
                _ => {}
            }
            next_id = node.parent;
        }
    }
    /// Removes a node from the datastructure.
    ///
    /// The caller needs to ensure that the node does not have any children.
    pub fn remove(&mut self, idx: UniIndex) {
        self.0.remove(idx);
    }
}

/// State per location per node keeping track of where relative to this
/// node exposed nodes are and what access permissions they have.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct WildcardState {
    /// How many local nodes are exposed with write permissions.
    local_writes: u16,
    /// How many local nodes are exposed with read permissions.
    local_reads: u16,
}
impl Tree {
    /// Marks the tag as exposed & updates the wildcard tracking data structure
    /// to represent its access level.
    /// Also takes as an argument whether the tag is protected or not.
    pub fn expose_tag(&mut self, tag: BorTag, protected: bool) {
        let id = self.tag_mapping.get(&tag).unwrap();
        let node = self.nodes.get_mut(id).unwrap();
        if !node.is_exposed {
            node.is_exposed = true;
            let node = self.nodes.get(id).unwrap();

            // When the first tag gets exposed then we initialize the
            // wildcard state for every node and location in the tree.
            for (_, loc) in self.locations.iter_mut_all() {
                let perm = loc
                    .perms
                    .get(id)
                    .map(|p| p.permission())
                    .unwrap_or_else(|| node.default_location_state().permission());

                let access_type = perm.strongest_allowed_local_access(protected);
                loc.wildcard_accesses.update_exposure(
                    &self.nodes,
                    id,
                    WildcardAccessLevel::None,
                    access_type,
                );
            }
        }
    }

    /// This updates the wildcard tracking data structure to reflect the release of
    /// the protector on `tag`.
    pub(super) fn update_exposure_for_protector_release(&mut self, tag: BorTag) {
        let idx = self.tag_mapping.get(&tag).unwrap();

        // We check if the node is already exposed, as we don't want to expose any
        // nodes which aren't already exposed.
        let node = self.nodes.get(idx).unwrap();
        if node.is_exposed {
            for (_, loc) in self.locations.iter_mut_all() {
                let perm = loc
                    .perms
                    .get(idx)
                    .map(|p| p.permission())
                    .unwrap_or_else(|| node.default_location_state().permission());

                let old_access_type = perm.strongest_allowed_local_access(true);
                let access_type = perm.strongest_allowed_local_access(false);
                loc.wildcard_accesses.update_exposure(
                    &self.nodes,
                    idx,
                    old_access_type,
                    access_type,
                );
            }
        }
    }
}

#[cfg(feature = "expensive-consistency-checks")]
impl Tree {
    /// Checks that the wildcard tracking data structure is internally consistent and
    /// has the correct `exposed_as` values.
    pub fn verify_wildcard_consistency(&self, global: &GlobalState) {
        // We rely on the fact that `roots` is ordered according to tag from low to high.
        assert!(self.roots.is_sorted_by_key(|idx| self.nodes.get(*idx).unwrap().tag));

        let protected_tags = &global.borrow().protected_tags;
        for (_, loc) in self.locations.iter_all() {
            let wildcard_accesses = &loc.wildcard_accesses;
            let perms = &loc.perms;
            for (id, node) in self.nodes.iter() {
                let state = wildcard_accesses.0.get(id).cloned().unwrap_or_default();

                let exposed_as = if node.is_exposed {
                    let perm =
                        perms.get(id).copied().unwrap_or_else(|| node.default_location_state());

                    perm.permission()
                        .strongest_allowed_local_access(protected_tags.contains_key(&node.tag))
                } else {
                    WildcardAccessLevel::None
                };

                let (child_reads, child_writes) = node
                    .children
                    .iter()
                    .copied()
                    .map(|id| wildcard_accesses.0.get(id).cloned().unwrap_or_default())
                    .fold((0, 0), |acc, wc| (acc.0 + wc.local_reads, acc.1 + wc.local_writes));
                let expected_reads =
                    child_reads + Into::<u16>::into(exposed_as >= WildcardAccessLevel::Read);
                let expected_writes =
                    child_writes + Into::<u16>::into(exposed_as >= WildcardAccessLevel::Write);
                assert_eq!(
                    state.local_reads, expected_reads,
                    "expected {:?}'s (id:{id:?}) local_reads to be {expected_reads:?} instead of {:?} (child_reads: {child_reads:?}, exposed_as: {exposed_as:?})",
                    node.tag, state.local_reads
                );
                assert_eq!(
                    state.local_writes, expected_writes,
                    "expected {:?}'s (id:{id:?}) local_writes to be {expected_writes:?} instead of {:?} (child_writes: {child_writes:?}, exposed_as: {exposed_as:?})",
                    node.tag, state.local_writes
                );
            }
        }
    }
}
