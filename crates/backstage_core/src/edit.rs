//! Edits made by direct manipulation (dragging on the stage, nudging with
//! the arrow keys), turned into commands. Shared by the stage and the
//! editor so both write the same thing. See ADR 0006.
//!
//! The rule for where a value goes: a property the shown animation has a
//! track for gets a key at the playhead; any other property changes the
//! node's rest value.

use crate::animation::{Animation, Key, Property, Value};
use crate::command::Command;
use crate::eval::clock::local_time;
use crate::eval::{RuntimeState, evaluate_from};
use crate::geom::Vec2;
use crate::id::{AnimId, CompId, NodeId};
use crate::node::Repeat;
use crate::project::{Composition, Project};
use crate::time::Time;
use glam::Affine2;
use std::collections::BTreeMap;

/// The playhead an edit is made at: the animation shown on the edited
/// composition (its default if `None`) and the composition's clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct At {
    pub anim: Option<AnimId>,
    pub clock: Time,
}

impl At {
    /// The animation shown, if the composition has it.
    pub fn animation<'c>(&self, comp: &'c Composition) -> Option<(AnimId, &'c Animation)> {
        let id = self.anim.or(comp.default_animation)?;
        Some((id, comp.animations.get(&id)?))
    }

    /// What `evaluate` needs to show this playhead on the composition: the
    /// chosen animation from time zero, or (with none) its default.
    pub fn state(&self) -> RuntimeState {
        let mut state = RuntimeState::default();
        if let Some(anim) = self.anim {
            state.play(&Vec::new(), anim, Time::ZERO);
        }
        state
    }
}

/// Sets `values` on `node` of composition `comp` at playhead `at`: a key
/// at the animation-local time for each property the shown animation
/// animates (keeping the ease of a key already there), the rest value for
/// the others. `None` if nothing would change.
pub fn set_values(
    project: &Project,
    comp: CompId,
    at: &At,
    node: NodeId,
    values: &[(Property, Value)],
) -> Option<Command> {
    let composition = project.compositions.get(&comp)?;
    let current = composition.nodes.get(&node)?;
    let anim = at.animation(composition);
    let mut commands = Vec::new();
    let mut rest = current.rest;
    for &(property, value) in values {
        match anim.and_then(|(id, a)| Some((id, a, a.track(node, property)?))) {
            Some((anim_id, animation, track)) => {
                let local = local_time(animation, at.clock, Repeat::Natural);
                let existing = track.keys.iter().find(|k| k.at == local);
                if existing.is_some_and(|k| k.value == value) {
                    continue;
                }
                let ease = existing.map(|k| k.ease).unwrap_or_default();
                let key = Key::new(local, value, ease);
                commands.push(Command::SetKey { comp, anim: anim_id, node, property, key });
            }
            None => rest.set(property, value),
        }
    }
    if rest != current.rest {
        commands.push(Command::SetRest { comp, node, rest });
    }
    batch(commands)
}

/// Moves `nodes` of composition `comp` by `delta` stage units at playhead
/// `at`: each from where it is at that moment, by the delta mapped into
/// its parent's space, so it follows the pointer whatever its groups do.
/// A node whose ancestor is also moved is left to follow it. `None` if
/// nothing would change.
pub fn move_nodes(
    project: &Project,
    comp: CompId,
    at: &At,
    nodes: &[NodeId],
    delta: Vec2,
) -> Option<Command> {
    let composition = project.compositions.get(&comp)?;
    let scene = evaluate_from(project, &at.state(), comp, at.clock);
    let parents: BTreeMap<NodeId, NodeId> =
        composition.nodes.iter().flat_map(|(&id, n)| n.children.iter().map(move |&c| (c, id))).collect();
    let moved_with_ancestor = |mut node: NodeId| {
        while let Some(&parent) = parents.get(&node) {
            if nodes.contains(&parent) {
                return true;
            }
            node = parent;
        }
        false
    };
    let mut commands = Vec::new();
    for &node in nodes {
        if moved_with_ancestor(node) {
            continue;
        }
        let Some(state) = scene.nodes.get(&node) else { continue };
        let parent =
            parents.get(&node).and_then(|p| scene.nodes.get(p)).map_or(Affine2::IDENTITY, |s| s.world);
        if parent.matrix2.determinant().abs() < 1e-12 {
            continue; // Scaled to nothing: there's no way to follow.
        }
        let local = parent.matrix2.inverse() * delta;
        let to = state.props.transform.position + local;
        let values = [(Property::X, Value::Number(to.x)), (Property::Y, Value::Number(to.y))];
        if let Some(command) = set_values(project, comp, at, node, &values) {
            match command {
                Command::Batch(inner) => commands.extend(inner),
                single => commands.push(single),
            }
        }
    }
    batch(commands)
}

/// Where a node is at a playhead, for transforming it: its evaluated
/// transform, and its parent's world transform (stage ← parent space).
struct Placed {
    transform: crate::geom::Transform,
    parent: Affine2,
}

fn placed(project: &Project, comp: CompId, at: &At, node: NodeId) -> Option<Placed> {
    let composition = project.compositions.get(&comp)?;
    let scene = evaluate_from(project, &at.state(), comp, at.clock);
    let state = scene.nodes.get(&node)?;
    let parent_id = composition.nodes.iter().find(|(_, n)| n.children.contains(&node)).map(|(&id, _)| id);
    let parent = parent_id.and_then(|p| scene.nodes.get(&p)).map_or(Affine2::IDENTITY, |s| s.world);
    (parent.matrix2.determinant().abs() > 1e-12)
        .then_some(Placed { transform: state.props.transform, parent })
}

/// A node's rotation and skew, without its scale: `local_matrix` is
/// `T(position) · rotate_skew · S(scale) · T(-pivot)`.
fn rotate_skew(t: &crate::geom::Transform) -> glam::Mat2 {
    let bare = crate::geom::Transform { position: Vec2::ZERO, scale: Vec2::ONE, pivot: Vec2::ZERO, ..*t };
    crate::eval::local_matrix(&bare).matrix2
}

/// Scales `node` by dragging its box: the point `handle` of its local
/// (shape) space goes to stage point `pointer` while `anchor` stays where
/// it is. An axis along which `handle` and `anchor` coincide (an edge
/// handle) keeps its scale. `uniform` keeps the scale's aspect ratio. Its
/// position moves to keep the anchor fixed.
#[allow(clippy::too_many_arguments)]
pub fn scale_node(
    project: &Project,
    comp: CompId,
    at: &At,
    node: NodeId,
    handle: Vec2,
    anchor: Vec2,
    pointer: Vec2,
    uniform: bool,
) -> Option<Command> {
    let Placed { transform: t, parent } = placed(project, comp, at, node)?;
    let m = rotate_skew(&t);
    if m.determinant().abs() < 1e-12 {
        return None;
    }
    let local = |q: Vec2| t.position + m * (t.scale * (q - t.pivot));
    let anchor_at = local(anchor);
    // Where the handle must go, measured from the anchor in the node's
    // rotated (unscaled) axes.
    let wanted = m.inverse() * (parent.inverse().transform_point2(pointer) - anchor_at);
    let span = handle - anchor;
    let mut scale = t.scale;
    if uniform {
        let current = t.scale * span;
        if current.length_squared() > 1e-12 {
            scale *= wanted.dot(current) / current.length_squared();
        }
    } else {
        if span.x.abs() > 1e-6 {
            scale.x = wanted.x / span.x;
        }
        if span.y.abs() > 1e-6 {
            scale.y = wanted.y / span.y;
        }
    }
    // Never quite zero: a node scaled to nothing can't be grabbed again.
    let at_least = |s: f32| if s.abs() < 1e-3 { 1e-3f32.copysign(s) } else { s };
    scale = Vec2::new(at_least(scale.x), at_least(scale.y));
    let position = anchor_at - m * (scale * (anchor - t.pivot));
    let values = [
        (Property::X, Value::Number(position.x)),
        (Property::Y, Value::Number(position.y)),
        (Property::ScaleX, Value::Number(scale.x)),
        (Property::ScaleY, Value::Number(scale.y)),
    ];
    set_values(project, comp, at, node, &values)
}

/// Turns `node` about its pivot by the angle the pointer swept from stage
/// point `from` to `to` around it. `snap` rounds the resulting rotation to
/// a multiple of that many degrees (Shift: 15).
pub fn rotate_node(
    project: &Project,
    comp: CompId,
    at: &At,
    node: NodeId,
    from: Vec2,
    to: Vec2,
    snap: Option<f32>,
) -> Option<Command> {
    let Placed { transform: t, parent } = placed(project, comp, at, node)?;
    let into_parent = parent.inverse();
    let angle = |p: Vec2| {
        let d = into_parent.transform_point2(p) - t.position;
        d.y.atan2(d.x)
    };
    let swept = (angle(to) - angle(from)).to_degrees();
    let mut rotation = t.rotation + swept;
    if let Some(step) = snap.filter(|s| *s > 0.0) {
        rotation = (rotation / step).round() * step;
    }
    // Keep it in -180..=180 so values stay readable after many turns.
    rotation = (rotation + 180.0).rem_euclid(360.0) - 180.0;
    set_values(project, comp, at, node, &[(Property::Rotation, Value::Number(rotation))])
}

/// Turns and scales several nodes together, as one: each node's pivot
/// point turns by `angle` degrees and scales by `scale` about stage point
/// `about`, and the node's own rotation and scale follow. A node whose
/// ancestor is also transformed is left to follow it. Exact when the
/// nodes' parents neither skew nor scale unevenly. `None` if nothing would
/// change.
pub fn transform_nodes(
    project: &Project,
    comp: CompId,
    at: &At,
    nodes: &[NodeId],
    about: Vec2,
    angle: f32,
    scale: f32,
) -> Option<Command> {
    let composition = project.compositions.get(&comp)?;
    let scene = evaluate_from(project, &at.state(), comp, at.clock);
    let parents: BTreeMap<NodeId, NodeId> =
        composition.nodes.iter().flat_map(|(&id, n)| n.children.iter().map(move |&c| (c, id))).collect();
    let moved_with_ancestor = |mut node: NodeId| {
        while let Some(&parent) = parents.get(&node) {
            if nodes.contains(&parent) {
                return true;
            }
            node = parent;
        }
        false
    };
    let turn = Affine2::from_translation(about)
        * Affine2::from_scale_angle_translation(Vec2::splat(scale), angle.to_radians(), Vec2::ZERO)
        * Affine2::from_translation(-about);
    let mut commands = Vec::new();
    for &node in nodes {
        if moved_with_ancestor(node) {
            continue;
        }
        let Some(state) = scene.nodes.get(&node) else { continue };
        let parent =
            parents.get(&node).and_then(|p| scene.nodes.get(p)).map_or(Affine2::IDENTITY, |s| s.world);
        if parent.matrix2.determinant().abs() < 1e-12 {
            continue;
        }
        let t = state.props.transform;
        // The pivot is where the position puts it in the parent's space.
        let pivot = parent.transform_point2(t.position);
        let position = parent.inverse().transform_point2(turn.transform_point2(pivot));
        let values = [
            (Property::X, Value::Number(position.x)),
            (Property::Y, Value::Number(position.y)),
            (Property::Rotation, Value::Number(t.rotation + angle)),
            (Property::ScaleX, Value::Number(t.scale.x * scale)),
            (Property::ScaleY, Value::Number(t.scale.y * scale)),
        ];
        if let Some(command) = set_values(project, comp, at, node, &values) {
            match command {
                Command::Batch(inner) => commands.extend(inner),
                single => commands.push(single),
            }
        }
    }
    batch(commands)
}

/// One command for several: `None` for none, the command itself for one.
fn batch(mut commands: Vec<Command>) -> Option<Command> {
    match commands.len() {
        0 => None,
        1 => commands.pop(),
        _ => Some(Command::Batch(commands)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Ease;
    use crate::sample::{self, ids::*};

    fn at(secs: i64) -> At {
        At { anim: None, clock: Time::from_secs(secs) }
    }

    /// Applies `command`, checks its inverse restores the project exactly,
    /// and returns the edited project.
    fn applied(project: &Project, command: &Command) -> Project {
        let mut edited = project.clone();
        let inverse = command.apply(&mut edited).unwrap();
        let mut undone = edited.clone();
        inverse.apply(&mut undone).unwrap();
        assert_eq!(&undone, project, "the inverse restores it");
        edited
    }

    fn rest(project: &Project, node: NodeId) -> Vec2 {
        project.compositions[&STAGE].nodes[&node].rest.transform.position
    }

    #[test]
    fn unanimated_nodes_move_their_rest_values() {
        let project = sample::bounce();
        let command = move_nodes(&project, STAGE, &at(0), &[GROUND], Vec2::new(10.0, -5.0)).unwrap();
        assert!(matches!(command, Command::SetRest { node: GROUND, .. }), "{command:?}");
        let edited = applied(&project, &command);
        assert_eq!(rest(&edited, GROUND), rest(&project, GROUND) + Vec2::new(10.0, -5.0));
        assert_eq!(move_nodes(&project, STAGE, &at(0), &[GROUND], Vec2::ZERO), None, "nothing to do");
    }

    #[test]
    fn animated_properties_get_a_key_at_the_playhead() {
        // The synced ball's X is keyed 0 s → 4 s on `main`; its Y isn't.
        let project = sample::bounce();
        let playhead = at(1);
        let command = move_nodes(&project, STAGE, &playhead, &[SYNCED_BALL], Vec2::new(10.0, 20.0)).unwrap();
        let Command::Batch(parts) = &command else { panic!("{command:?}") };
        let [Command::SetKey { property: Property::X, key, anim, .. }, Command::SetRest { rest, .. }] =
            &parts[..]
        else {
            panic!("{parts:?}")
        };
        assert_eq!((*anim, key.at, key.ease), (STAGE_MAIN, Time::from_secs(1), Ease::Linear), "a new key");
        let before = evaluate_from(&project, &playhead.state(), STAGE, playhead.clock).nodes[&SYNCED_BALL];
        assert_eq!(key.value, Value::Number(before.props.transform.position.x + 10.0), "from where it is");
        assert_eq!(rest.transform.position.y, rest_y(&project) + 20.0);

        let edited = applied(&project, &command);
        let after = evaluate_from(&edited, &playhead.state(), STAGE, playhead.clock).nodes[&SYNCED_BALL];
        let moved = after.props.transform.position - before.props.transform.position;
        assert!(moved.abs_diff_eq(Vec2::new(10.0, 20.0), 1e-3), "it lands where it was dragged: {moved}");
    }

    fn rest_y(project: &Project) -> f32 {
        rest(project, SYNCED_BALL).y
    }

    #[test]
    fn a_key_already_at_the_playhead_keeps_its_ease() {
        // At 0 s the synced ball has a key, eased in and out.
        let project = sample::bounce();
        let values = [(Property::X, Value::Number(50.0))];
        let command = set_values(&project, STAGE, &at(0), SYNCED_BALL, &values).unwrap();
        let Command::SetKey { key, .. } = command else { panic!("{command:?}") };
        let track =
            project.compositions[&STAGE].animations[&STAGE_MAIN].track(SYNCED_BALL, Property::X).unwrap();
        assert_eq!((key.at, key.ease), (Time::ZERO, track.keys[0].ease));
        assert_ne!(key.ease, Ease::Linear);
        let same = [(Property::X, track.keys[0].value)];
        assert_eq!(set_values(&project, STAGE, &at(0), SYNCED_BALL, &same), None, "unchanged");
    }

    #[test]
    fn an_animation_the_composition_lacks_keys_nothing() {
        let project = sample::bounce();
        let elsewhere = At { anim: Some(AnimId::from_raw(1)), clock: Time::from_secs(1) };
        let command =
            set_values(&project, STAGE, &elsewhere, SYNCED_BALL, &[(Property::X, Value::Number(5.0))]);
        assert!(matches!(command, Some(Command::SetRest { .. })), "{command:?}");
    }

    /// The sample with the stage's root group transformed by `transform`.
    fn with_root(transform: impl FnOnce(&mut crate::geom::Transform)) -> Project {
        let mut project = sample::bounce();
        let comp = project.compositions.get_mut(&STAGE).unwrap();
        transform(&mut comp.nodes.get_mut(&STAGE_ROOT).unwrap().rest.transform);
        project
    }

    #[test]
    fn the_delta_is_mapped_into_the_parents_space() {
        let scaled = with_root(|t| t.scale = Vec2::splat(2.0));
        let command = move_nodes(&scaled, STAGE, &at(0), &[GROUND], Vec2::new(10.0, 4.0)).unwrap();
        let moved = rest(&applied(&scaled, &command), GROUND) - rest(&scaled, GROUND);
        assert!(moved.abs_diff_eq(Vec2::new(5.0, 2.0), 1e-4), "halved inside a 2× group: {moved}");

        // Turned 90° clockwise (y down): stage right is the group's up.
        let turned = with_root(|t| t.rotation = 90.0);
        let command = move_nodes(&turned, STAGE, &at(0), &[GROUND], Vec2::new(10.0, 0.0)).unwrap();
        let moved = rest(&applied(&turned, &command), GROUND) - rest(&turned, GROUND);
        assert!(moved.abs_diff_eq(Vec2::new(0.0, -10.0), 1e-4), "the axes swap: {moved}");

        let flat = with_root(|t| t.scale = Vec2::new(1.0, 0.0));
        assert_eq!(move_nodes(&flat, STAGE, &at(0), &[GROUND], Vec2::ONE), None, "scaled to nothing");
    }

    #[test]
    fn several_nodes_move_in_one_batch_and_children_follow_their_parent() {
        let project = sample::bounce();
        let delta = Vec2::new(3.0, 0.0);
        let command = move_nodes(&project, STAGE, &at(0), &[GROUND, FREE_BALL], delta).unwrap();
        let Command::Batch(parts) = &command else { panic!("{command:?}") };
        assert_eq!(parts.len(), 2);
        let edited = applied(&project, &command);
        assert_eq!(rest(&edited, GROUND), rest(&project, GROUND) + delta);
        assert_eq!(rest(&edited, FREE_BALL), rest(&project, FREE_BALL) + delta);

        // The root and a child: only the root moves (the child goes along).
        let command = move_nodes(&project, STAGE, &at(0), &[GROUND, STAGE_ROOT], delta).unwrap();
        assert!(matches!(command, Command::SetRest { node: STAGE_ROOT, .. }), "{command:?}");
    }

    /// The stage point a node's local point `q` is drawn at.
    fn drawn_at(project: &Project, node: NodeId, q: Vec2) -> Vec2 {
        evaluate_from(project, &at(0).state(), STAGE, Time::ZERO).nodes[&node].world.transform_point2(q)
    }

    // The ground is a 550 × 60 rect at (0, 340) in its own space.
    const TOP_LEFT: Vec2 = Vec2::new(0.0, 340.0);
    const BOTTOM_RIGHT: Vec2 = Vec2::new(550.0, 400.0);

    #[test]
    fn scaling_keeps_the_anchor_and_brings_the_handle_to_the_pointer() {
        for project in
            [sample::bounce(), with_root(|t| t.rotation = 30.0), with_root(|t| t.scale = Vec2::splat(2.0))]
        {
            let anchor = drawn_at(&project, GROUND, TOP_LEFT);
            let pointer = drawn_at(&project, GROUND, BOTTOM_RIGHT) + Vec2::new(40.0, 25.0);
            let command =
                scale_node(&project, STAGE, &at(0), GROUND, BOTTOM_RIGHT, TOP_LEFT, pointer, false).unwrap();
            let edited = applied(&project, &command);
            assert!(drawn_at(&edited, GROUND, TOP_LEFT).abs_diff_eq(anchor, 1e-2), "the anchor stays");
            assert!(drawn_at(&edited, GROUND, BOTTOM_RIGHT).abs_diff_eq(pointer, 1e-2), "the handle follows");
        }
    }

    #[test]
    fn edge_handles_scale_one_axis_and_shift_keeps_the_ratio() {
        let project = sample::bounce();
        let scale = |p: &Project| p.compositions[&STAGE].nodes[&GROUND].rest.transform.scale;
        // The right edge's middle, dragged right and down: only X changes.
        let (right, left) = (Vec2::new(550.0, 370.0), Vec2::new(0.0, 370.0));
        let command =
            scale_node(&project, STAGE, &at(0), GROUND, right, left, Vec2::new(825.0, 500.0), false).unwrap();
        let s = scale(&applied(&project, &command));
        assert!((s.x - 1.5).abs() < 1e-4 && s.y == 1.0, "{s}");

        let pointer = Vec2::new(600.0, 500.0);
        let command =
            scale_node(&project, STAGE, &at(0), GROUND, BOTTOM_RIGHT, TOP_LEFT, pointer, true).unwrap();
        let s = scale(&applied(&project, &command));
        assert!((s.x - s.y).abs() < 1e-5 && s.x > 1.0, "uniform: {s}");
    }

    #[test]
    fn scaling_about_the_pivot_keeps_the_position() {
        // The free ball's pivot is its origin, where its position puts it.
        let project = sample::bounce();
        let handle = Vec2::new(30.0, 30.0);
        let pointer = drawn_at(&project, FREE_BALL, Vec2::new(60.0, 60.0));
        let command =
            scale_node(&project, STAGE, &at(0), FREE_BALL, handle, Vec2::ZERO, pointer, false).unwrap();
        let edited = applied(&project, &command);
        let t = edited.compositions[&STAGE].nodes[&FREE_BALL].rest.transform;
        assert_eq!(t.position, project.compositions[&STAGE].nodes[&FREE_BALL].rest.transform.position);
        assert!(t.scale.abs_diff_eq(Vec2::splat(2.0), 1e-4), "{t:?}");
    }

    #[test]
    fn rotating_turns_about_the_pivot_and_snaps() {
        let project = sample::bounce();
        let pivot = drawn_at(&project, FREE_BALL, Vec2::ZERO);
        let rotation = |p: &Project| p.compositions[&STAGE].nodes[&FREE_BALL].rest.transform.rotation;
        // A quarter turn clockwise (y down): from the right to below.
        let (right, below) = (pivot + Vec2::new(50.0, 0.0), pivot + Vec2::new(0.0, 50.0));
        let command = rotate_node(&project, STAGE, &at(0), FREE_BALL, right, below, None).unwrap();
        let edited = applied(&project, &command);
        assert!((rotation(&edited) - 90.0).abs() < 1e-3, "{}", rotation(&edited));
        assert!(drawn_at(&edited, FREE_BALL, Vec2::ZERO).abs_diff_eq(pivot, 1e-3), "the pivot stays");

        // Four quarter turns come back around.
        let mut turned = project.clone();
        for _ in 0..4 {
            let command = rotate_node(&turned, STAGE, &at(0), FREE_BALL, right, below, None).unwrap();
            turned = applied(&turned, &command);
        }
        assert!(rotation(&turned).abs() < 1e-3, "{}", rotation(&turned));

        let a_bit = pivot + Vec2::new(50.0, 10.0); // about 11°
        let snapped = rotate_node(&project, STAGE, &at(0), FREE_BALL, right, a_bit, Some(15.0)).unwrap();
        assert_eq!(rotation(&applied(&project, &snapped)), 15.0);
        let unsnapped = rotate_node(&project, STAGE, &at(0), FREE_BALL, right, a_bit, None).unwrap();
        assert!((rotation(&applied(&project, &unsnapped)) - 11.31).abs() < 0.01);
    }

    #[test]
    fn handles_key_animated_properties_too() {
        // The synced ball's X is animated: scaling about a corner moves it,
        // so X gets a key while the scale changes its rest values.
        let project = sample::bounce();
        let pointer = drawn_at(&project, SYNCED_BALL, Vec2::splat(60.0));
        let command = scale_node(
            &project,
            STAGE,
            &at(1),
            SYNCED_BALL,
            Vec2::splat(30.0),
            Vec2::splat(-30.0),
            pointer,
            true,
        )
        .unwrap();
        let Command::Batch(parts) = &command else { panic!("{command:?}") };
        assert!(
            parts.iter().any(|c| matches!(c, Command::SetKey { property: Property::X, .. })),
            "{parts:?}"
        );
        applied(&project, &command);
    }

    #[test]
    fn several_nodes_turn_and_scale_together_as_one() {
        let project = sample::bounce();
        let nodes = [FREE_BALL, GROUND];
        let about = Vec2::new(300.0, 200.0);
        let command = transform_nodes(&project, STAGE, &at(0), &nodes, about, 90.0, 2.0).unwrap();
        let edited = applied(&project, &command);
        // Every point of every node lands where the same turn and scale
        // about `about` takes it: the pivot, and a point well away from it.
        let turn = Affine2::from_translation(about)
            * Affine2::from_scale_angle_translation(Vec2::splat(2.0), 90f32.to_radians(), Vec2::ZERO)
            * Affine2::from_translation(-about);
        for node in nodes {
            for q in [Vec2::ZERO, Vec2::new(30.0, -12.0)] {
                let before = drawn_at(&project, node, q);
                let after = drawn_at(&edited, node, q);
                assert!(after.abs_diff_eq(turn.transform_point2(before), 1e-2), "{node:?} {q}: {after}");
            }
        }
        assert_eq!(transform_nodes(&project, STAGE, &at(0), &nodes, about, 0.0, 1.0), None, "no change");
    }
}
