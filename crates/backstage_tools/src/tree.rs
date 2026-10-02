//! The edited composition's node tree as rows, shared by the Layers panel
//! and the timeline so both always show the same rows. Which nodes are
//! expanded is editor session state, not part of the document.

use backstage_core::{CompId, Composition, NodeFlags, NodeId, NodeKind, editor};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub node: NodeId,
    pub name: String,
    /// 0 for the root node's children.
    pub depth: usize,
    /// Has children of its own, so it gets an expander. (An instance's
    /// content is another composition; entering it is a separate step.)
    pub has_children: bool,
    pub expanded: bool,
    /// The node's own hide/lock/outline flags.
    pub flags: NodeFlags,
    /// Those combined with its ancestors' (what actually applies).
    pub effective: NodeFlags,
    /// For an instance, the composition it shows (double-click enters it).
    pub enters: Option<CompId>,
}

/// The rows to show: depth first in child order below the root node,
/// descending only into expanded nodes. IDs in `expanded` that aren't in
/// the composition are ignored.
pub fn visible_rows(comp: &Composition, expanded: &BTreeSet<NodeId>) -> Vec<TreeRow> {
    let effective = editor::effective(comp);
    let mut rows = Vec::new();
    let mut stack: Vec<(NodeId, usize)> = comp
        .root_node()
        .map(|root| root.children.iter().rev().map(|&c| (c, 0)).collect())
        .unwrap_or_default();
    while let Some((id, depth)) = stack.pop() {
        let Some(node) = comp.nodes.get(&id) else { continue };
        let has_children = !node.children.is_empty();
        let open = has_children && expanded.contains(&id);
        rows.push(TreeRow {
            node: id,
            name: node.name.clone(),
            depth,
            has_children,
            expanded: open,
            flags: node.editor,
            effective: effective.get(&id).copied().unwrap_or_default(),
            enters: match &node.kind {
                NodeKind::Instance(i) => Some(i.comp),
                _ => None,
            },
        });
        if open {
            stack.extend(node.children.iter().rev().map(|&c| (c, depth + 1)));
        }
    }
    rows
}

/// Every node that can be expanded: the starting state for a document.
pub fn expandable(comp: &Composition) -> BTreeSet<NodeId> {
    comp.nodes
        .iter()
        .filter(|(id, n)| **id != comp.root && !n.children.is_empty())
        .map(|(id, _)| *id)
        .collect()
}

/// `node` and all its descendants.
pub fn subtree(comp: &Composition, node: NodeId) -> BTreeSet<NodeId> {
    let mut out = BTreeSet::new();
    let mut stack = vec![node];
    while let Some(id) = stack.pop() {
        if out.insert(id)
            && let Some(n) = comp.nodes.get(&id)
        {
            stack.extend(&n.children);
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use backstage_core::sample::{self, ids::*};
    use backstage_core::{Node, NodeKind, Project};

    pub const GROUP: NodeId = NodeId::from_raw(0x5747_1999);

    /// The sample with `ground` and `eyes` moved into a new group `scenery`,
    /// which takes `ground`'s place.
    pub fn grouped() -> Project {
        let mut project = sample::bounce();
        let stage = project.compositions.get_mut(&STAGE).unwrap();
        stage.nodes.insert(GROUP, Node::new("scenery", NodeKind::Group).with_children(vec![GROUND, EYES]));
        let root = stage.nodes.get_mut(&STAGE_ROOT).unwrap();
        root.children = vec![GROUP, FREE_BALL, SYNCED_BALL];
        project.validate().unwrap();
        project
    }

    fn summary(rows: &[TreeRow]) -> Vec<(NodeId, usize, bool, bool)> {
        rows.iter().map(|r| (r.node, r.depth, r.has_children, r.expanded)).collect()
    }

    #[test]
    fn the_samples_top_level_nodes_have_no_expanders() {
        let project = sample::bounce();
        let comp = &project.compositions[&STAGE];
        let rows = visible_rows(comp, &expandable(comp));
        assert_eq!(
            summary(&rows),
            [
                (GROUND, 0, false, false),
                (EYES, 0, false, false),
                (FREE_BALL, 0, false, false),
                (SYNCED_BALL, 0, false, false)
            ],
            "instances have content, but no children of their own"
        );
        assert_eq!(rows[0].name, "ground");
        assert_eq!(rows[0].enters, None);
        assert_eq!(rows.iter().find(|r| r.node == FREE_BALL).unwrap().enters, Some(BALL));
        assert!(expandable(comp).is_empty());
    }

    #[test]
    fn groups_expand_and_collapse() {
        let project = grouped();
        let comp = &project.compositions[&STAGE];
        assert_eq!(expandable(comp), BTreeSet::from([GROUP]));

        let open = visible_rows(comp, &BTreeSet::from([GROUP]));
        assert_eq!(
            summary(&open),
            [
                (GROUP, 0, true, true),
                (GROUND, 1, false, false),
                (EYES, 1, false, false),
                (FREE_BALL, 0, false, false),
                (SYNCED_BALL, 0, false, false)
            ]
        );
        let closed = visible_rows(comp, &BTreeSet::new());
        assert_eq!(
            summary(&closed),
            [(GROUP, 0, true, false), (FREE_BALL, 0, false, false), (SYNCED_BALL, 0, false, false)]
        );
        let stale = BTreeSet::from([GROUP, NodeId::from_raw(1), GROUND]);
        assert_eq!(visible_rows(comp, &stale), open, "unknown or childless IDs change nothing");
    }

    #[test]
    fn rows_carry_own_and_inherited_flags() {
        let mut project = grouped();
        let stage = project.compositions.get_mut(&STAGE).unwrap();
        stage.nodes.get_mut(&GROUP).unwrap().editor.locked = true;
        let comp = &project.compositions[&STAGE];
        let rows = visible_rows(comp, &BTreeSet::from([GROUP]));
        let ground = rows.iter().find(|r| r.node == GROUND).unwrap();
        assert!(!ground.flags.locked && ground.effective.locked, "inherited from the group");
        let ball = rows.iter().find(|r| r.node == FREE_BALL).unwrap();
        assert_eq!(ball.effective, NodeFlags::default());
    }

    #[test]
    fn subtrees_include_the_node_and_its_descendants() {
        let project = grouped();
        let comp = &project.compositions[&STAGE];
        assert_eq!(subtree(comp, GROUP), BTreeSet::from([GROUP, GROUND, EYES]));
        assert_eq!(subtree(comp, FREE_BALL), BTreeSet::from([FREE_BALL]));
    }
}
