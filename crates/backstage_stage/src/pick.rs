//! Picking on the editor's stage: which node of the edited composition a
//! click selects, and the bounds its selection box goes around. Works on
//! the scene as the editor shows it (`view::editor_view`), so hidden nodes
//! never hit.
//!
//! - A click picks the outermost group holding the hit: the child of the
//!   composition's root. `deep` (Ctrl) picks the node itself.
//! - Content inside an instance always picks the instance; it belongs to
//!   another composition.
//! - Locked nodes are skipped, so the click reaches what's under them.

use backstage_core::{Composition, DrawItem, NodeFlags, NodeId, Scene, Vec2};
use backstage_render::pick::{Rect, hit_item, item_bounds};
use std::collections::BTreeMap;

/// The edited composition's structure, for mapping items to nodes.
pub struct Picker<'c> {
    comp: &'c Composition,
    flags: BTreeMap<NodeId, NodeFlags>,
    parents: BTreeMap<NodeId, NodeId>,
}

impl<'c> Picker<'c> {
    pub fn new(comp: &'c Composition) -> Self {
        let parents =
            comp.nodes.iter().flat_map(|(&id, node)| node.children.iter().map(move |&c| (c, id))).collect();
        Self { comp, flags: backstage_core::editor::effective(comp), parents }
    }

    /// The node of this composition that drew `item`: itself, or the
    /// top-level instance it's inside.
    fn owner(item: &DrawItem) -> NodeId {
        item.instance.first().copied().unwrap_or(item.node)
    }

    /// What a click on `owner` selects.
    fn target(&self, owner: NodeId, deep: bool) -> NodeId {
        if deep {
            return owner;
        }
        let mut node = owner;
        while let Some(&parent) = self.parents.get(&node) {
            if parent == self.comp.root {
                break;
            }
            node = parent;
        }
        node
    }

    /// The node a click at stage point `at` selects, if any: the topmost
    /// unlocked item hit, within `slop` stage units of a stroke.
    pub fn pick(&self, scene: &Scene, at: Vec2, deep: bool, slop: f32) -> Option<NodeId> {
        scene
            .items
            .iter()
            .rev()
            .filter(|item| !self.flags.get(&Self::owner(item)).is_some_and(|f| f.locked))
            .find(|item| hit_item(item, at, slop))
            .map(|item| self.target(Self::owner(item), deep))
    }

    /// What a marquee over stage rect `rect` selects: every unlocked node
    /// whose items' bounds touch it, picked by the same rule as a click
    /// (`deep`: Ctrl), each once, bottom to top.
    pub fn in_rect(&self, scene: &Scene, rect: Rect, deep: bool) -> Vec<NodeId> {
        let mut nodes = Vec::new();
        for item in &scene.items {
            if self.flags.get(&Self::owner(item)).is_some_and(|f| f.locked) {
                continue;
            }
            if !item_bounds(item).is_some_and(|b| b.intersects(rect)) {
                continue;
            }
            let node = self.target(Self::owner(item), deep);
            // Painter's order: a node drawn again higher up moves up.
            nodes.retain(|n| *n != node);
            nodes.push(node);
        }
        nodes
    }

    /// Bounds on the stage of everything `node` draws (its whole subtree),
    /// or `None` if it draws nothing.
    pub fn bounds(&self, scene: &Scene, node: NodeId) -> Option<Rect> {
        scene
            .items
            .iter()
            .filter(|item| self.is_within(Self::owner(item), node))
            .filter_map(item_bounds)
            .reduce(Rect::union)
    }

    /// Whether `node` is `ancestor` or below it.
    fn is_within(&self, mut node: NodeId, ancestor: NodeId) -> bool {
        loop {
            if node == ancestor {
                return true;
            }
            match self.parents.get(&node) {
                Some(&parent) => node = parent,
                None => return false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Picker;
    use backstage_core::sample::{self, ids::*};
    use backstage_core::{
        Command, Node, NodeFlags, NodeId, NodeKind, Project, RuntimeState, Scene, Time, Vec2, evaluate,
    };
    use backstage_render::pick::item_bounds;

    const GROUP: NodeId = NodeId::from_raw(0x9999);

    /// The sample with the ground moved into a new group under the root.
    fn grouped() -> Project {
        let mut project = sample::bounce();
        let add = Command::AddNode {
            comp: STAGE,
            parent: STAGE_ROOT,
            index: 0,
            id: GROUP,
            node: Node::new("group", NodeKind::Group),
        };
        let mv = Command::MoveNode { comp: STAGE, node: GROUND, parent: GROUP, index: 0 };
        Command::Batch(vec![add, mv]).apply(&mut project).unwrap();
        project
    }

    fn scene(project: &Project) -> Scene<'_> {
        evaluate(project, &RuntimeState::default(), Time::ZERO)
    }

    /// The centre of what `node` draws.
    fn centre(project: &Project, node: NodeId) -> Vec2 {
        let scene = scene(project);
        let b = Picker::new(&project.compositions[&STAGE]).bounds(&scene, node).unwrap();
        (b.min + b.max) / 2.0
    }

    fn pick(project: &Project, at: Vec2, deep: bool) -> Option<NodeId> {
        let scene = scene(project);
        Picker::new(&project.compositions[&STAGE]).pick(&scene, at, deep, 0.0)
    }

    #[test]
    fn a_click_picks_the_top_level_node_it_hits() {
        let project = sample::bounce();
        assert_eq!(pick(&project, centre(&project, GROUND), false), Some(GROUND));
        assert_eq!(pick(&project, centre(&project, FREE_BALL), false), Some(FREE_BALL), "an instance");
        assert_eq!(pick(&project, Vec2::new(5.0, 5.0), false), None, "empty stage");
    }

    #[test]
    fn content_inside_an_instance_picks_the_instance_even_deep() {
        let project = sample::bounce();
        assert_eq!(pick(&project, centre(&project, FREE_BALL), true), Some(FREE_BALL));
    }

    #[test]
    fn groups_are_picked_whole_unless_deep() {
        let project = grouped();
        let at = centre(&project, GROUND);
        assert_eq!(pick(&project, at, false), Some(GROUP), "the outermost group");
        assert_eq!(pick(&project, at, true), Some(GROUND), "Ctrl: the node itself");
    }

    #[test]
    fn locked_nodes_let_clicks_through() {
        let mut project = grouped();
        let at = centre(&project, GROUND);
        let lock = NodeFlags { locked: true, ..Default::default() };
        Command::SetNodeFlags { comp: STAGE, node: GROUP, flags: lock }.apply(&mut project).unwrap();
        assert_eq!(pick(&project, at, false), None, "locked through its group");
        assert_eq!(pick(&project, at, true), None);

        // Something locked on top: the click reaches what's under it.
        let mut project = sample::bounce();
        let ball = centre(&project, FREE_BALL);
        let top = Command::MoveNode { comp: STAGE, node: GROUND, parent: STAGE_ROOT, index: 3 };
        // The strip is drawn at y 340..400; centre it on the ball.
        let over = ball - Vec2::new(275.0, 370.0);
        let over = backstage_core::Props::at(over.x, over.y);
        Command::Batch(vec![top, Command::SetRest { comp: STAGE, node: GROUND, rest: over }])
            .apply(&mut project)
            .unwrap();
        let ground = Picker::new(&project.compositions[&STAGE]).bounds(&scene(&project), GROUND).unwrap();
        assert!(ground.min.y <= ball.y && ball.y <= ground.max.y, "the ground now covers the ball");
        assert_eq!(pick(&project, ball, false), Some(GROUND), "the ground is on top");
        Command::SetNodeFlags { comp: STAGE, node: GROUND, flags: lock }.apply(&mut project).unwrap();
        assert_eq!(pick(&project, ball, false), Some(FREE_BALL), "locked: the ball under it");
    }

    fn marquee(project: &Project, min: Vec2, max: Vec2, deep: bool) -> Vec<NodeId> {
        let scene = scene(project);
        Picker::new(&project.compositions[&STAGE]).in_rect(
            &scene,
            backstage_render::pick::Rect { min, max },
            deep,
        )
    }

    #[test]
    fn a_marquee_selects_what_it_touches_bottom_to_top() {
        let project = sample::bounce();
        let all = marquee(&project, Vec2::ZERO, Vec2::new(550.0, 400.0), false);
        assert_eq!(all, [GROUND, EYES, FREE_BALL, SYNCED_BALL], "painter's order");
        let above_ground = marquee(&project, Vec2::new(10.0, 10.0), Vec2::new(540.0, 330.0), false);
        assert_eq!(above_ground, [EYES, FREE_BALL, SYNCED_BALL]);
        // Just touching the free ball's edge is enough.
        let ball = centre(&project, FREE_BALL);
        let edge = marquee(&project, ball, ball + Vec2::splat(200.0), false);
        assert!(edge.contains(&FREE_BALL), "{edge:?}");
        assert_eq!(marquee(&project, Vec2::new(1.0, 1.0), Vec2::new(5.0, 5.0), false), [], "nothing there");
    }

    #[test]
    fn a_marquee_follows_the_pick_rules() {
        let mut project = grouped();
        let everything = (Vec2::ZERO, Vec2::new(550.0, 400.0));
        assert!(marquee(&project, everything.0, everything.1, false).contains(&GROUP), "the outermost group");
        let deep = marquee(&project, everything.0, everything.1, true);
        assert!(deep.contains(&GROUND) && !deep.contains(&GROUP), "Ctrl: the nodes themselves");
        let lock = NodeFlags { locked: true, ..Default::default() };
        Command::SetNodeFlags { comp: STAGE, node: GROUP, flags: lock }.apply(&mut project).unwrap();
        let unlocked = marquee(&project, everything.0, everything.1, false);
        assert!(!unlocked.contains(&GROUP) && !unlocked.is_empty(), "locked: left out, {unlocked:?}");
    }

    #[test]
    fn bounds_cover_a_nodes_whole_subtree() {
        let project = grouped();
        let scene = scene(&project);
        let picker = Picker::new(&project.compositions[&STAGE]);
        let ground = scene.items.iter().find(|i| i.node == GROUND).and_then(item_bounds).unwrap();
        assert_eq!(picker.bounds(&scene, GROUP), Some(ground), "the group around its only child");
        assert_eq!(picker.bounds(&scene, GROUND), Some(ground));
        let all = picker.bounds(&scene, STAGE_ROOT).unwrap();
        let ball = picker.bounds(&scene, FREE_BALL).unwrap();
        assert!(all.min.cmple(ball.min).all() && all.max.cmpge(ball.max).all(), "the root holds everything");
        assert_eq!(picker.bounds(&scene, NodeId::from_raw(1)), None, "no such node");
    }
}
