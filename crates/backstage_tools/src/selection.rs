//! Keeping the stage's mirror of the selection current (ADR 0005). The
//! editor owns the selection; this decides when to send it: on any change,
//! and again after a stage (re)connects.

use backstage_core::{CompId, NodeId};
use backstage_protocol::{PickMode, ToStage};

/// The selection after a pick on the stage (ADR 0005), or `None` if the
/// pick was made in a composition other than the `edited` one (the editor
/// moved on before it arrived) and is ignored.
pub fn apply_pick(
    current: &[NodeId],
    edited: CompId,
    comp: CompId,
    nodes: &[NodeId],
    mode: PickMode,
) -> Option<Vec<NodeId>> {
    (comp == edited).then(|| mode.apply(current, nodes))
}

#[derive(Debug, Default)]
pub struct SelectionSync {
    /// What the current stage was last sent; `None` before anything was.
    sent: Option<(CompId, Vec<NodeId>)>,
}

impl SelectionSync {
    /// The message to send if the selection (nodes of `comp`, primary last)
    /// differs from what the stage has.
    pub fn update(&mut self, comp: CompId, nodes: &[NodeId]) -> Option<ToStage> {
        if self.sent.as_ref().is_some_and(|(c, n)| *c == comp && n == nodes) {
            return None;
        }
        self.sent = Some((comp, nodes.to_vec()));
        Some(ToStage::Selection { comp, nodes: nodes.to_vec() })
    }

    /// A new stage knows nothing: the next update sends again.
    pub fn reset(&mut self) {
        self.sent = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{SelectionSync, apply_pick};
    use backstage_core::sample::ids::*;
    use backstage_protocol::{PickMode, ToStage};

    #[test]
    fn picks_apply_only_to_the_edited_composition() {
        assert_eq!(apply_pick(&[GROUND], STAGE, STAGE, &[EYES], PickMode::Replace), Some(vec![EYES]));
        assert_eq!(apply_pick(&[GROUND], STAGE, STAGE, &[EYES], PickMode::Toggle), Some(vec![GROUND, EYES]));
        assert_eq!(apply_pick(&[GROUND], STAGE, STAGE, &[], PickMode::Replace), Some(vec![]));
        assert_eq!(
            apply_pick(&[GROUND], STAGE, BALL, &[BALL_BODY], PickMode::Replace),
            None,
            "left it since"
        );
    }

    #[test]
    fn sends_changes_once_and_again_after_a_reset() {
        let mut sync = SelectionSync::default();
        assert_eq!(sync.update(STAGE, &[]), Some(ToStage::Selection { comp: STAGE, nodes: vec![] }), "first");
        assert_eq!(sync.update(STAGE, &[]), None, "unchanged");
        assert_eq!(
            sync.update(STAGE, &[GROUND]),
            Some(ToStage::Selection { comp: STAGE, nodes: vec![GROUND] })
        );
        assert_eq!(sync.update(STAGE, &[GROUND]), None);
        assert!(sync.update(STAGE, &[EYES, GROUND]).is_some(), "different nodes");
        assert!(sync.update(STAGE, &[GROUND, EYES]).is_some(), "different order: a different primary");
        assert!(sync.update(BALL, &[GROUND, EYES]).is_some(), "another composition");
        sync.reset();
        assert!(sync.update(BALL, &[GROUND, EYES]).is_some(), "a new stage gets it again");
    }
}
