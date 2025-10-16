use std::cmp::max;

use super::tree::{AccessRelatedness, Location, Node};
use super::unimap::{UniIndex, UniValMap};
use super::{LocationState, Tree};
use crate::{AccessKind, BorTag};
/// represensts the maximum access level that is possible.
///
/// Note that we derive Ord and PartialOrd, so the order in which variants are listed below matters:
/// None < Read < Write. Do not change that order. See the `test_order` test.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum WildcardAccessLevel {
    #[default]
    None,
    Read,
    Write,
}
/// were relative to the pointer the access happened from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WildcardAccessRelatedness {
    /// the access definitively happened through a child pointer
    ChildAccess,
    /// the access definitively happened through a foreign pointer
    ForeignAccess,
    /// we do not know if the access is foreign or child
    EitherAccess,
}
impl WildcardAccessRelatedness {
    pub fn to_relatedness(self) -> Option<AccessRelatedness> {
        match self {
            Self::ChildAccess => Some(AccessRelatedness::WildcardChildAccess),
            Self::ForeignAccess => Some(AccessRelatedness::WildcardForeignAccess),
            Self::EitherAccess => None,
        }
    }
}

/// state per location per pointer keeping track of where relative to this
/// node exposed pointers are and what access permissions they have
///
/// designed to be completely determined by its parent, siblings and direct
/// childrens max_child_access/max_foreign_access
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WildcardAccessTracking {
    /// how many of this nodes direct children have `max_child_access==Write`
    child_writes: u16,
    /// how many of this nodes direct children have `max_child_access>=Read`
    child_reads: u16,
    /// the maximum access level that could happen from an exposed
    /// pointer that is foreign to this pointer
    ///
    /// this is calculated as the `max()` of parents `max_foreign_access`, `exposed_as` and
    /// its siblings `max_child_access`
    max_foreign_access: WildcardAccessLevel,
}
impl WildcardAccessTracking {
    /// the maximum access level that could happen from an exposed
    /// pointer that is a child of this pointer
    pub fn max_child_access(&self, exposed_as: WildcardAccessLevel) -> WildcardAccessLevel {
        use WildcardAccessLevel::*;
        max(
            exposed_as,
            if self.child_writes > 0 {
                Write
            } else if self.child_reads > 0 {
                Read
            } else {
                None
            },
        )
    }
    /// from where relative to this pointer a read or write access could happen
    pub fn access_relatedness(
        &self,
        kind: AccessKind,
        exposed_as: WildcardAccessLevel,
    ) -> Option<WildcardAccessRelatedness> {
        match kind {
            AccessKind::Read => self.read_access_relatedness(exposed_as),
            AccessKind::Write => self.write_access_relatedness(exposed_as),
        }
    }
    /// from where relative to this pointer a read access could happen
    pub fn read_access_relatedness(
        &self,
        exposed_as: WildcardAccessLevel,
    ) -> Option<WildcardAccessRelatedness> {
        let has_foreign = self.max_foreign_access >= WildcardAccessLevel::Read;
        let has_child = self.child_reads > 0 || exposed_as >= WildcardAccessLevel::Read;
        use WildcardAccessRelatedness as E;
        match (has_foreign, has_child) {
            (true, true) => Some(E::EitherAccess),
            (true, false) => Some(E::ForeignAccess),
            (false, true) => Some(E::ChildAccess),
            (false, false) => None,
        }
    }
    /// from where relative to this pointer a write access could happen
    pub fn write_access_relatedness(
        &self,
        exposed_as: WildcardAccessLevel,
    ) -> Option<WildcardAccessRelatedness> {
        let has_foreign = self.max_foreign_access == WildcardAccessLevel::Write;
        let has_child = self.child_writes > 0 || exposed_as == WildcardAccessLevel::Write;
        use WildcardAccessRelatedness as E;
        match (has_foreign, has_child) {
            (true, true) => Some(E::EitherAccess),
            (true, false) => Some(E::ForeignAccess),
            (false, true) => Some(E::ChildAccess),
            (false, false) => None,
        }
    }
    /// gets the access tracking information for a new child node.
    /// doesnt have any child reads/writes, but calculates max_foreign_access from parent
    pub fn get_new_child(&self, exposed_as: WildcardAccessLevel) -> Self {
        Self {
            max_foreign_access: max(self.max_foreign_access, self.max_child_access(exposed_as)),
            child_reads: 0,
            child_writes: 0,
        }
    }
    /// update the tracking information of a tree, to reflect the access level change from `old_access_type` to `access_type` of an exposed pointer.
    /// propagates the wilcard access information over the tree
    /// this needs to be called every time the access level of an exposed reference changes, to keep the state in sync
    pub fn update_exposure(
        id: UniIndex,
        old_access_type: WildcardAccessLevel,
        access_type: WildcardAccessLevel,
        nodes: &UniValMap<Node>,
        perms: &UniValMap<LocationState>,
        wildcard_accesses: &mut UniValMap<WildcardAccessTracking>,
    ) {
        /// pushes children onto the stack, if their `max_foreign_access` field needs to be updated
        ///
        /// the `max_foreign_access` fields is set based on the max of the parents `max_foreign_access`,
        /// `exposed_as` and its siblings `max_child_access`.
        ///
        /// this function calculates the siblings `max_child_access`, both of the other fields need to be passed as arguments
        ///
        /// * `other_factors`:  we only ever change one of these values. The max value of the other fields we dont change should be passed through the `other_factors` parameter
        /// * `old_access_type`,`access_type`: we change the parameter not covered by `other_factors` from `old_access_type`
        ///   to `access_type`
        fn push_relevant_children(
            stack: &mut Vec<(UniIndex, WildcardAccessLevel)>,
            other_factors: WildcardAccessLevel,
            access_type: WildcardAccessLevel,
            old_access_type: WildcardAccessLevel,
            access: WildcardAccessTracking,
            mut children: impl Iterator<Item = UniIndex>,
            nodes: &UniValMap<Node>,
            perms: &UniValMap<LocationState>,
            wildcard_accesses: &mut UniValMap<WildcardAccessTracking>,
        ) {
            // our change is only visible if the other_factors arent larger
            if max(access_type, old_access_type) < other_factors {
                return;
            }
            // we cannot change the access level lower than `old_access_type`
            let access_type = max(access_type, other_factors);
            let old_access_type = max(old_access_type, other_factors);

            use WildcardAccessLevel::*;
            let child_accesses = if max(access_type, old_access_type) == Write {
                // None -> Write
                // Read -> Write
                // Write -> Read
                // Write -> None
                access.child_writes
            } else {
                // None -> Read
                // Read  -> None
                access.child_reads
            };
            // how many child accesses we have
            if child_accesses == 0 {
                // no children have child_accesses at this access level, so the max_foreign_access field of each
                // is entirely determined by `access_type` and `other factors`
                // this means every child needs to be updated on a change
                stack.extend(children.map(|id| (id, access_type)));
            } else if child_accesses == 1 {
                // there is exactly one child at this child access level, so for all other children the `max_foreign_access`
                // is defined by this child. So we only need to update this one child
                stack.push((
                    children
                        .find(|id| {
                            let access = wildcard_accesses.get(*id).unwrap();
                            let node = nodes.get(*id).unwrap();
                            let exposed_as =
                                node.exposed_as(perms.get(*id).map(|p| p.permission()));
                            access.max_child_access(exposed_as) >= access_type
                        })
                        .unwrap(),
                    access_type,
                ));
            } else {
                // there are multiple children with this access level. they are already foreign to each other so
                // the parents access level doesnt effect them. we dont need to update any other children
            }
        }
        // if the exposure doesnt change, then we dont need to update anything
        if old_access_type == access_type {
            return;
        }

        let mut entry = wildcard_accesses.entry(id);
        let src_access = entry.or_insert(Default::default());

        // wether we are upgrading or downgrading the allowed access rights
        let is_upgrade = old_access_type < access_type;

        // stack to process references for which the max_foreign_access field needs to be updated
        let mut stack: Vec<(UniIndex, WildcardAccessLevel)> = Vec::new();

        // update the direct children of this node
        {
            let node = nodes.get(id).unwrap();
            push_relevant_children(
                &mut stack,
                /* other factors */ src_access.max_foreign_access,
                access_type,
                old_access_type,
                src_access.clone(),
                node.children.iter().copied(),
                nodes,
                perms,
                wildcard_accesses,
            );
        }
        // we need to propagate the tracking info up the tree, for this we traverse up the parents
        // we can skip propagating info to parents & their other children, if their access permissions
        // dont change (for parents child_permissions and for the other children foreign permissions)
        {
            // we need to keep track of how the previous permissions changed
            let mut prev_old_access = old_access_type;
            let mut prev = id;
            while let Some(id) = nodes.get(prev).unwrap().parent {
                let node = nodes.get(id).unwrap();
                let mut entry = wildcard_accesses.entry(id);
                let access = entry.or_insert(Default::default());

                let old_access = access.clone();
                use WildcardAccessLevel::*;
                // updating this nodes tracking data for children
                if is_upgrade {
                    if access_type == Write {
                        // None -> Write
                        // Read -> Write
                        access.child_writes += 1;
                    }
                    if prev_old_access == None {
                        // None -> Read
                        // None -> Write
                        access.child_reads += 1;
                    }
                } else {
                    if prev_old_access == Write {
                        // Write -> None
                        // Write -> Read
                        access.child_writes -= 1;
                    }
                    if access_type == None {
                        // Read  -> None
                        // Write -> None
                        access.child_reads -= 1;
                    }
                }
                let exposed_as = node.exposed_as(perms.get(id).map(|p| p.permission()));
                let old_max_child_access = old_access.max_child_access(exposed_as);
                let new_max_child_access = access.max_child_access(exposed_as);
                {
                    // we need to update all children excluding the child we came from
                    // for this we need access and the children array to exclude prev
                    let access = if is_upgrade { old_access.clone() } else { access.clone() };
                    push_relevant_children(
                        &mut stack,
                        /* other factors */ max(exposed_as, old_access.max_foreign_access),
                        access_type,
                        old_access_type,
                        access,
                        node.children.iter().copied().filter(|id| prev != *id),
                        nodes,
                        perms,
                        wildcard_accesses,
                    );
                }
                if old_max_child_access == new_max_child_access {
                    // child_access didnt change, so we dont need to propagate further upwards
                    break;
                }
                prev_old_access = old_max_child_access;
                prev = id;
            }
        }
        //traverses up the tree to update max_foreign_access fields
        while let Some((id, access_type)) = stack.pop() {
            let node = nodes.get(id).unwrap();
            let mut entry = wildcard_accesses.entry(id);
            let access = entry.or_insert(Default::default());
            // all items on the stack need this updated
            access.max_foreign_access = access_type;
            let exposed_as = node.exposed_as(perms.get(id).map(|p| p.permission()));

            push_relevant_children(
                &mut stack,
                /* other factors */ exposed_as,
                access_type,
                old_access_type,
                access.clone(),
                node.children.iter().copied(),
                nodes,
                perms,
                wildcard_accesses,
            );
        }
        #[cfg(feature = "expensive-consistency-checks")]
        Self::verify_consistency(id, nodes, perms, wildcard_accesses)
    }
    /// verifies that the access tracking state is consistent
    ///
    /// panics if invalid
    #[cfg(feature = "expensive-consistency-checks")]
    pub fn verify_consistency(
        id: UniIndex,
        nodes: &UniValMap<Node>,
        perms: &UniValMap<LocationState>,
        wildcard_accesses: &UniValMap<WildcardAccessTracking>,
    ) {
        // find root node
        let mut root = id;
        while let Some(parent) = nodes.get(root).and_then(|n| n.parent) {
            root = parent;
        }

        // TODO valid if map is empty
        if !wildcard_accesses.contains_idx(root) {
            return;
        }
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let node = nodes.get(id).unwrap();
            stack.extend(node.children.iter());

            let access = wildcard_accesses.get(id).unwrap();

            let expected_max_foreign_access = if let Some(parent) = node.parent {
                let parent_node = nodes.get(parent).unwrap();
                let parent_perm = perms.get(parent).map(LocationState::permission);
                let parent_access = wildcard_accesses.get(parent).unwrap();

                let max_other_children = parent_node
                    .children
                    .iter()
                    .copied()
                    .filter(|child| *child != id)
                    .map(|child| {
                        let node = nodes.get(child).unwrap();
                        let perm = perms.get(child).map(LocationState::permission);
                        let access = wildcard_accesses.get(child).unwrap();
                        access.max_child_access(node.exposed_as(perm))
                    })
                    .fold(WildcardAccessLevel::None, max);
                max_other_children
                    .max(parent_access.max_foreign_access)
                    .max(parent_node.exposed_as(parent_perm))
            } else {
                WildcardAccessLevel::None
            };

            let child_accesses = node.children.iter().copied().map(|child| {
                let node = nodes.get(child).unwrap();
                let perm = perms.get(child).map(LocationState::permission);
                let access = wildcard_accesses.get(child).unwrap();
                access.max_child_access(node.exposed_as(perm))
            });
            let expected_child_reads =
                child_accesses.clone().filter(|a| *a >= WildcardAccessLevel::Read).count();
            let expected_child_writes =
                child_accesses.filter(|a| *a >= WildcardAccessLevel::Write).count();

            assert_eq!(expected_max_foreign_access, access.max_foreign_access);
            let child_reads: usize = access.child_reads.into();
            assert_eq!(expected_child_reads, child_reads);
            let child_writes: usize = access.child_writes.into();
            assert_eq!(expected_child_writes, child_writes);
        }
    }
}
impl Tree {
    pub fn expose_tag(&mut self, tag: BorTag) {
        let id = self.tag_mapping.get(&tag).unwrap();
        let node = self.nodes.get_mut(id).unwrap();
        node.is_exposed = true;
        let node = self.nodes.get(id).unwrap();
        // TODO: only initialize neccessary ranges
        for (_, Location { perms, wildcard_accesses }) in self.rperms.iter_mut_all() {
            let perm = *perms.entry(id).or_insert(node.default_location_state());

            let access_type = perm.permission().strongest_allowed_child_access();
            WildcardAccessTracking::update_exposure(
                id,
                WildcardAccessLevel::None,
                access_type,
                &self.nodes,
                perms,
                wildcard_accesses,
            );
        }
    }
}
