//! The editor's view of a scene: nodes hidden in the editor are left out,
//! and outlined ones are marked with their outline colour. Only the editing
//! stage applies this; the player draws the scene as evaluated. See
//! `backstage_core::editor`.

use backstage_core::{Color, Project, Scene, editor};

/// `scene` without hidden items, and each remaining item's outline colour.
/// Items are matched to the root composition's nodes: an item drawn by the
/// root composition by its own node, nested content by the root-level
/// instance it's inside.
pub fn editor_view<'p>(project: &'p Project, scene: Scene<'p>) -> (Scene<'p>, Vec<Option<Color>>) {
    let Some(comp) = project.root_composition() else { return (scene, Vec::new()) };
    let flags = editor::effective(comp);
    let colors = editor::outline_colors(comp);
    let mut items = Vec::with_capacity(scene.items.len());
    let mut outlines = Vec::with_capacity(scene.items.len());
    for item in scene.items {
        let node = item.instance.first().copied().unwrap_or(item.node);
        if flags.get(&node).is_some_and(|f| f.hidden) {
            continue;
        }
        outlines.push(colors.get(&node).copied());
        items.push(item);
    }
    (Scene { items }, outlines)
}

#[cfg(test)]
mod tests {
    use super::editor_view;
    use backstage_core::sample::{self, ids::*};
    use backstage_core::{NodeFlags, NodeId, Project, RuntimeState, Time, editor, evaluate};

    fn flag(project: &mut Project, node: NodeId, flags: NodeFlags) {
        project.compositions.get_mut(&STAGE).unwrap().nodes.get_mut(&node).unwrap().editor = flags;
    }

    /// The root-level node each item belongs to.
    fn owners(project: &Project) -> Vec<NodeId> {
        let scene = evaluate(project, &RuntimeState::default(), Time::ZERO);
        let (scene, _) = editor_view(project, scene);
        scene.items.iter().map(|i| i.instance.first().copied().unwrap_or(i.node)).collect()
    }

    const HIDDEN: NodeFlags = NodeFlags { hidden: true, locked: false, outline: false };
    const OUTLINED: NodeFlags = NodeFlags { hidden: false, locked: false, outline: true };

    #[test]
    fn no_flags_change_nothing() {
        let project = sample::bounce();
        let scene = evaluate(&project, &RuntimeState::default(), Time::ZERO);
        let (view, outlines) = editor_view(&project, scene.clone());
        assert_eq!(view, scene);
        assert!(outlines.iter().all(Option::is_none));
        assert_eq!(outlines.len(), view.items.len());
    }

    #[test]
    fn hidden_nodes_and_instances_disappear_with_their_content() {
        let mut project = sample::bounce();
        let all = owners(&project);
        assert!(all.contains(&GROUND) && all.contains(&FREE_BALL));

        flag(&mut project, GROUND, HIDDEN);
        flag(&mut project, FREE_BALL, HIDDEN);
        let shown = owners(&project);
        assert!(!shown.contains(&GROUND), "a hidden shape");
        assert!(!shown.contains(&FREE_BALL), "a hidden instance's nested content");
        assert_eq!(shown.len(), all.iter().filter(|n| **n != GROUND && **n != FREE_BALL).count());

        let mut project = sample::bounce();
        flag(&mut project, STAGE_ROOT, HIDDEN);
        assert!(owners(&project).is_empty(), "hiding a group hides its subtree");
    }

    #[test]
    fn outlined_items_get_their_nodes_colour() {
        let mut project = sample::bounce();
        flag(&mut project, SYNCED_BALL, OUTLINED);
        let scene = evaluate(&project, &RuntimeState::default(), Time::ZERO);
        let (view, outlines) = editor_view(&project, scene);
        for (item, outline) in view.items.iter().zip(&outlines) {
            let owner = item.instance.first().copied().unwrap_or(item.node);
            let expected = (owner == SYNCED_BALL).then(|| editor::outline_color(SYNCED_BALL));
            assert_eq!(*outline, expected, "{owner:?}");
        }
        assert!(outlines.iter().any(Option::is_some));
    }
}
