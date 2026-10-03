//! The stage's mirror of the editor's selection (ADR 0005). The editor owns
//! it and sends it on every change and connect; the stage only reads it,
//! for drawing selection handles and starting drags (M4).

use backstage_core::{CompId, NodeId, Project};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Selection {
    comp: Option<CompId>,
    /// In order; the last one is primary.
    nodes: Vec<NodeId>,
}

impl Selection {
    pub fn set(comp: CompId, nodes: Vec<NodeId>) -> Self {
        Self { comp: Some(comp), nodes }
    }

    /// The selected nodes to show while `edited` is the shown composition:
    /// none if the selection belongs to another composition, and only
    /// nodes that still exist, in order.
    #[cfg_attr(not(test), expect(dead_code, reason = "M4's selection handles and drags will read it"))]
    pub fn shown(&self, project: &Project, edited: CompId) -> Vec<NodeId> {
        if self.comp != Some(edited) {
            return Vec::new();
        }
        let Some(comp) = project.compositions.get(&edited) else { return Vec::new() };
        self.nodes.iter().copied().filter(|n| comp.nodes.contains_key(n)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Selection;
    use backstage_core::NodeId;
    use backstage_core::sample::{self, ids::*};

    #[test]
    fn shows_only_existing_nodes_of_the_shown_composition_in_order() {
        let project = sample::bounce();
        let gone = NodeId::from_raw(1);
        let selection = Selection::set(STAGE, vec![SYNCED_BALL, gone, GROUND]);
        assert_eq!(selection.shown(&project, STAGE), [SYNCED_BALL, GROUND]);
        assert!(selection.shown(&project, BALL).is_empty(), "another composition");
        // Even when its nodes happen to exist in the shown one.
        assert!(Selection::set(BALL, vec![GROUND]).shown(&project, STAGE).is_empty());
        assert!(Selection::default().shown(&project, STAGE).is_empty());
    }
}
