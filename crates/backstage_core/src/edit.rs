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
}
