//! Edit commands: every change to a [`Project`] is one of these.
//!
//! Applying a command returns its *inverse*, built from the state it
//! replaced, so undo is just applying the inverse. A command that fails
//! leaves the project untouched. Commands address everything by ID, and new
//! objects get their IDs from whoever builds the command, never from
//! `apply`, so replaying the same commands always gives the same project.
//! See `docs/adr/0002-stage-process-isolation.md`.

use crate::animation::{Key, LoopMode, Property, Track};
use crate::id::{AnimId, CompId, NodeId};
use crate::node::{Node, Props};
use crate::project::{Composition, EditorPrefs, Project, ProjectSettings};
use crate::time::Time;
use crate::validate::ValidationError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Replaces a node's rest properties.
    SetRest {
        comp: CompId,
        node: NodeId,
        rest: Props,
    },
    /// Sets the key at `key.at`, replacing one already there. Creates the
    /// track (at the end) if the animation has none for this property.
    SetKey {
        comp: CompId,
        anim: AnimId,
        node: NodeId,
        property: Property,
        key: Key,
    },
    /// Removes the key at `at`. Removing a track's last key removes the
    /// track.
    RemoveKey {
        comp: CompId,
        anim: AnimId,
        node: NodeId,
        property: Property,
        at: Time,
    },
    InsertTrack {
        comp: CompId,
        anim: AnimId,
        index: usize,
        track: Track,
    },
    RemoveTrack {
        comp: CompId,
        anim: AnimId,
        node: NodeId,
        property: Property,
    },
    /// Adds a node under `parent` at `index` in its children. The node must
    /// have no children yet; add those with more `AddNode`s.
    AddNode {
        comp: CompId,
        parent: NodeId,
        index: usize,
        id: NodeId,
        node: Node,
    },
    /// Removes a node, its subtree, and every track that targets them.
    RemoveNode {
        comp: CompId,
        node: NodeId,
    },
    /// Moves a node to `parent` at `index`, counted after it has been taken
    /// out of its old place.
    MoveNode {
        comp: CompId,
        node: NodeId,
        parent: NodeId,
        index: usize,
    },
    RenameNode {
        comp: CompId,
        node: NodeId,
        name: String,
    },
    RenameAnimation {
        comp: CompId,
        anim: AnimId,
        name: String,
    },
    SetAnimationTiming {
        comp: CompId,
        anim: AnimId,
        duration: Time,
        looping: LoopMode,
        step: Option<Time>,
    },
    SetSettings(ProjectSettings),
    SetEditorPrefs(EditorPrefs),
    /// Applies all or nothing, in order. Its inverse undoes them in reverse.
    Batch(Vec<Command>),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CommandError {
    #[error("composition {0} does not exist")]
    MissingComposition(CompId),
    #[error("{comp}: node {node} does not exist")]
    MissingNode { comp: CompId, node: NodeId },
    #[error("{comp}: animation {anim} does not exist")]
    MissingAnimation { comp: CompId, anim: AnimId },
    #[error("{comp}/{anim}: no {property:?} track for {node}")]
    MissingTrack { comp: CompId, anim: AnimId, node: NodeId, property: Property },
    #[error("{comp}/{anim}: no {property:?} key for {node} at {at}")]
    MissingKey { comp: CompId, anim: AnimId, node: NodeId, property: Property, at: Time },
    #[error("{comp}: node {node} already exists")]
    NodeExists { comp: CompId, node: NodeId },
    #[error("{comp}/{anim}: {property:?} track for {node} already exists")]
    TrackExists { comp: CompId, anim: AnimId, node: NodeId, property: Property },
    #[error("{comp}: new node {node} must not list children")]
    NodeHasChildren { comp: CompId, node: NodeId },
    #[error("{comp}: root node {node} cannot be removed or moved")]
    RootNode { comp: CompId, node: NodeId },
    #[error("{comp}: cannot move {node} into its own subtree")]
    MoveIntoOwnSubtree { comp: CompId, node: NodeId },
    #[error("index {index} is past the end ({len})")]
    IndexOutOfRange { index: usize, len: usize },
    #[error("the change would make the project invalid:\n{}", .0.iter().map(|e| format!("  - {e}")).collect::<Vec<_>>().join("\n"))]
    Invalid(Vec<ValidationError>),
    #[error("command {index} of the batch failed: {source}")]
    InBatch { index: usize, source: Box<CommandError> },
}

impl Command {
    /// Applies the command and returns its inverse. On error the project is
    /// unchanged. Assumes the project was valid, and keeps it valid.
    pub fn apply(&self, project: &mut Project) -> Result<Command, CommandError> {
        let Command::Batch(commands) = self else {
            let inverse = self.apply_unchecked(project)?;
            if let Some(comp) = self.composition()
                && let Err(errors) = project.validate_composition(comp)
            {
                undo(project, &inverse);
                return Err(CommandError::Invalid(errors));
            }
            return Ok(inverse);
        };
        let mut inverses = Vec::with_capacity(commands.len());
        for (index, command) in commands.iter().enumerate() {
            match command.apply(project) {
                Ok(inverse) => inverses.push(inverse),
                Err(e) => {
                    inverses.iter().rev().for_each(|inverse| undo(project, inverse));
                    return Err(CommandError::InBatch { index, source: Box::new(e) });
                }
            }
        }
        inverses.reverse();
        Ok(Command::Batch(inverses))
    }

    /// The composition a single command edits, which is all that needs
    /// revalidating after it.
    fn composition(&self) -> Option<CompId> {
        use Command::*;
        match self {
            SetRest { comp, .. }
            | SetKey { comp, .. }
            | RemoveKey { comp, .. }
            | InsertTrack { comp, .. }
            | RemoveTrack { comp, .. }
            | AddNode { comp, .. }
            | RemoveNode { comp, .. }
            | MoveNode { comp, .. }
            | RenameNode { comp, .. }
            | RenameAnimation { comp, .. }
            | SetAnimationTiming { comp, .. } => Some(*comp),
            SetSettings(_) | SetEditorPrefs(_) | Batch(_) => None,
        }
    }

    /// Makes the change without validating the result. Checks everything it
    /// needs to look up *before* changing anything, so errors leave the
    /// project untouched.
    fn apply_unchecked(&self, project: &mut Project) -> Result<Command, CommandError> {
        use Command::*;
        Ok(match self {
            SetRest { comp, node, rest } => {
                let n = node_mut(composition_mut(project, *comp)?, *node)?;
                SetRest { comp: *comp, node: *node, rest: std::mem::replace(&mut n.rest, *rest) }
            }
            SetKey { comp, anim, node, property, key } => {
                let (comp, anim, node, property) = (*comp, *anim, *node, *property);
                let c = composition_mut(project, comp)?;
                if !c.nodes.contains_key(&node) {
                    return Err(CommandError::MissingNode { comp, node });
                }
                let a = animation_mut(c, anim)?;
                let Some(track) = a.tracks.iter_mut().find(|t| t.node == node && t.property == property)
                else {
                    a.tracks.push(Track { node, property, keys: vec![*key] });
                    return Ok(RemoveTrack { comp, anim, node, property });
                };
                match track.keys.binary_search_by_key(&key.at, |k| k.at) {
                    Ok(i) => {
                        let old = std::mem::replace(&mut track.keys[i], *key);
                        SetKey { comp, anim, node, property, key: old }
                    }
                    Err(i) => {
                        track.keys.insert(i, *key);
                        RemoveKey { comp, anim, node, property, at: key.at }
                    }
                }
            }
            RemoveKey { comp, anim, node, property, at } => {
                let (comp, anim, node, property, at) = (*comp, *anim, *node, *property, *at);
                let a = animation_mut(composition_mut(project, comp)?, anim)?;
                let index = track_index(a, node, property).ok_or(CommandError::MissingTrack {
                    comp,
                    anim,
                    node,
                    property,
                })?;
                let keys = &mut a.tracks[index].keys;
                let i = keys.binary_search_by_key(&at, |k| k.at).map_err(|_| CommandError::MissingKey {
                    comp,
                    anim,
                    node,
                    property,
                    at,
                })?;
                if keys.len() == 1 {
                    InsertTrack { comp, anim, index, track: a.tracks.remove(index) }
                } else {
                    SetKey { comp, anim, node, property, key: keys.remove(i) }
                }
            }
            InsertTrack { comp, anim, index, track } => {
                let (comp, anim, node, property) = (*comp, *anim, track.node, track.property);
                let a = animation_mut(composition_mut(project, comp)?, anim)?;
                if track_index(a, node, property).is_some() {
                    return Err(CommandError::TrackExists { comp, anim, node, property });
                }
                check_index(*index, a.tracks.len())?;
                a.tracks.insert(*index, track.clone());
                RemoveTrack { comp, anim, node, property }
            }
            RemoveTrack { comp, anim, node, property } => {
                let (comp, anim, node, property) = (*comp, *anim, *node, *property);
                let a = animation_mut(composition_mut(project, comp)?, anim)?;
                let index = track_index(a, node, property).ok_or(CommandError::MissingTrack {
                    comp,
                    anim,
                    node,
                    property,
                })?;
                InsertTrack { comp, anim, index, track: a.tracks.remove(index) }
            }
            AddNode { comp, parent, index, id, node } => {
                let (comp, id) = (*comp, *id);
                let c = composition_mut(project, comp)?;
                if c.nodes.contains_key(&id) {
                    return Err(CommandError::NodeExists { comp, node: id });
                }
                if !node.children.is_empty() {
                    return Err(CommandError::NodeHasChildren { comp, node: id });
                }
                let p = node_mut(c, *parent)?;
                check_index(*index, p.children.len())?;
                p.children.insert(*index, id);
                c.nodes.insert(id, node.clone());
                RemoveNode { comp, node: id }
            }
            RemoveNode { comp, node } => {
                let (comp, node) = (*comp, *node);
                let c = composition_mut(project, comp)?;
                let (parent, index) = place_of(c, node)?;
                // The inverse re-adds the subtree top-down, then puts the
                // tracks back at their old indices (ascending, so each index
                // is right when it's inserted).
                let mut restore = Vec::new();
                readd(c, node, parent, index, &mut restore);
                let removed: BTreeSet<NodeId> = restore
                    .iter()
                    .map(|cmd| match cmd {
                        AddNode { id, .. } => *id,
                        _ => unreachable!(),
                    })
                    .collect();
                for (&anim, a) in &mut c.animations {
                    let mut index = 0;
                    a.tracks.retain(|track| {
                        let keep = !removed.contains(&track.node);
                        if !keep {
                            restore.push(InsertTrack { comp, anim, index, track: track.clone() });
                        }
                        index += 1;
                        keep
                    });
                }
                c.nodes.get_mut(&parent).expect("parent exists").children.remove(index);
                for id in &removed {
                    c.nodes.remove(id);
                }
                Command::batch(restore)
            }
            MoveNode { comp, node, parent, index } => {
                let (comp, node, parent) = (*comp, *node, *parent);
                let c = composition_mut(project, comp)?;
                let (old_parent, old_index) = place_of(c, node)?;
                let new_parent =
                    c.nodes.get(&parent).ok_or(CommandError::MissingNode { comp, node: parent })?;
                if subtree(c, node).contains(&parent) {
                    return Err(CommandError::MoveIntoOwnSubtree { comp, node });
                }
                let len = new_parent.children.len() - usize::from(parent == old_parent);
                check_index(*index, len)?;
                c.nodes.get_mut(&old_parent).expect("parent exists").children.remove(old_index);
                c.nodes.get_mut(&parent).expect("checked above").children.insert(*index, node);
                MoveNode { comp, node, parent: old_parent, index: old_index }
            }
            RenameNode { comp, node, name } => {
                let n = node_mut(composition_mut(project, *comp)?, *node)?;
                RenameNode { comp: *comp, node: *node, name: std::mem::replace(&mut n.name, name.clone()) }
            }
            RenameAnimation { comp, anim, name } => {
                let a = animation_mut(composition_mut(project, *comp)?, *anim)?;
                RenameAnimation {
                    comp: *comp,
                    anim: *anim,
                    name: std::mem::replace(&mut a.name, name.clone()),
                }
            }
            SetAnimationTiming { comp, anim, duration, looping, step } => {
                let a = animation_mut(composition_mut(project, *comp)?, *anim)?;
                SetAnimationTiming {
                    comp: *comp,
                    anim: *anim,
                    duration: std::mem::replace(&mut a.duration, *duration),
                    looping: std::mem::replace(&mut a.looping, *looping),
                    step: std::mem::replace(&mut a.step, *step),
                }
            }
            SetSettings(settings) => SetSettings(std::mem::replace(&mut project.settings, *settings)),
            SetEditorPrefs(prefs) => SetEditorPrefs(std::mem::replace(&mut project.editor, *prefs)),
            Batch(commands) => {
                let mut inverses = Vec::with_capacity(commands.len());
                for (index, command) in commands.iter().enumerate() {
                    match command.apply_unchecked(project) {
                        Ok(inverse) => inverses.push(inverse),
                        Err(e) => {
                            inverses.iter().rev().for_each(|inverse| undo(project, inverse));
                            return Err(CommandError::InBatch { index, source: Box::new(e) });
                        }
                    }
                }
                inverses.reverse();
                Batch(inverses)
            }
        })
    }

    /// A batch, or the command itself if there is only one.
    fn batch(mut commands: Vec<Command>) -> Command {
        if commands.len() == 1 { commands.pop().unwrap() } else { Command::Batch(commands) }
    }
}

/// Applies an inverse to restore a state that was valid before, so it needs
/// no validation and can't fail.
fn undo(project: &mut Project, inverse: &Command) {
    inverse.apply_unchecked(project).expect("an inverse always applies to the state it came from");
}

fn composition_mut(project: &mut Project, comp: CompId) -> Result<&mut Composition, CommandError> {
    project.compositions.get_mut(&comp).ok_or(CommandError::MissingComposition(comp))
}

fn node_mut(c: &mut Composition, node: NodeId) -> Result<&mut Node, CommandError> {
    let comp = c.id;
    c.nodes.get_mut(&node).ok_or(CommandError::MissingNode { comp, node })
}

fn animation_mut(c: &mut Composition, anim: AnimId) -> Result<&mut crate::Animation, CommandError> {
    let comp = c.id;
    c.animations.get_mut(&anim).ok_or(CommandError::MissingAnimation { comp, anim })
}

fn track_index(a: &crate::Animation, node: NodeId, property: Property) -> Option<usize> {
    a.tracks.iter().position(|t| t.node == node && t.property == property)
}

fn check_index(index: usize, len: usize) -> Result<(), CommandError> {
    if index <= len { Ok(()) } else { Err(CommandError::IndexOutOfRange { index, len }) }
}

/// The parent of a non-root node, and its index among the parent's
/// children.
fn place_of(c: &Composition, node: NodeId) -> Result<(NodeId, usize), CommandError> {
    let comp = c.id;
    if node == c.root {
        return Err(CommandError::RootNode { comp, node });
    }
    c.nodes
        .iter()
        .find_map(|(&parent, p)| p.children.iter().position(|&n| n == node).map(|i| (parent, i)))
        .ok_or(CommandError::MissingNode { comp, node })
}

/// `node` and everything under it.
fn subtree(c: &Composition, node: NodeId) -> BTreeSet<NodeId> {
    let mut out = BTreeSet::new();
    let mut stack = vec![node];
    while let Some(id) = stack.pop() {
        if out.insert(id)
            && let Some(n) = c.nodes.get(&id)
        {
            stack.extend(&n.children);
        }
    }
    out
}

/// `AddNode` commands that rebuild the subtree at `id`, parents first.
fn readd(c: &Composition, id: NodeId, parent: NodeId, index: usize, out: &mut Vec<Command>) {
    let node = &c.nodes[&id];
    out.push(Command::AddNode {
        comp: c.id,
        parent,
        index,
        id,
        node: Node { children: Vec::new(), ..node.clone() },
    });
    for (i, &child) in node.children.iter().enumerate() {
        readd(c, child, id, i, out);
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, Command::*, CommandError as E};
    use crate::animation::{Ease, Key, LoopMode, Property, Track, Value};
    use crate::geom::Color;
    use crate::id::{AnimId, CompId, NodeId};
    use crate::node::{Instance, Node, NodeKind, Props, TimeMode};
    use crate::project::{EditorPrefs, Project, ProjectSettings};
    use crate::sample::{self, ids::*};
    use crate::time::{Time, TimeGrid};
    use crate::validate::ValidationError;

    const NEW: NodeId = NodeId::from_raw(0x00ee_0001);

    fn key(secs: (i64, i64), v: f32) -> Key {
        Key::new(Time::from_ratio(secs.0, secs.1), Value::Number(v), Ease::Linear)
    }

    /// Applies `command` to the sample and checks the round trip: the result
    /// is valid, the inverse restores the sample exactly, and the inverse's
    /// inverse (redo) gets the edited project back. Returns the edited
    /// project and the inverse.
    fn round_trip(command: Command) -> (Project, Command) {
        let original = sample::bounce();
        let mut p = original.clone();
        let inverse = command.apply(&mut p).unwrap_or_else(|e| panic!("{command:?} failed: {e}"));
        p.validate().unwrap();
        let edited = p.clone();
        let redo = inverse.apply(&mut p).unwrap();
        assert_eq!(p, original, "undo didn't restore the project");
        redo.apply(&mut p).unwrap();
        assert_eq!(p, edited, "redo didn't restore the edit");
        (edited, inverse)
    }

    /// Applies `command` to the sample, expects it to fail, and checks the
    /// project is untouched.
    fn fails(command: Command) -> E {
        let original = sample::bounce();
        let mut p = original.clone();
        let err = command.apply(&mut p).expect_err("expected the command to fail");
        assert_eq!(p, original, "a failed command changed the project");
        err
    }

    fn bounce_y(p: &Project) -> &Track {
        p.compositions[&BALL].animations[&BALL_BOUNCE].track(BALL_BODY, Property::Y).unwrap()
    }

    #[test]
    fn set_rest() {
        let rest = Props::at(1.0, 2.0);
        let (p, inverse) = round_trip(SetRest { comp: STAGE, node: GROUND, rest });
        assert_eq!(p.compositions[&STAGE].nodes[&GROUND].rest, rest);
        assert!(matches!(inverse, SetRest { node: GROUND, .. }));
    }

    #[test]
    fn set_key_replaces_inserts_or_creates_a_track() {
        let at = |p: &Project| bounce_y(p).keys[0].at;
        let original = sample::bounce();

        let replaced = Key { value: Value::Number(-5.0), ..bounce_y(&original).keys[0] };
        let cmd =
            SetKey { comp: BALL, anim: BALL_BOUNCE, node: BALL_BODY, property: Property::Y, key: replaced };
        let (p, inverse) = round_trip(cmd);
        assert_eq!(bounce_y(&p).keys[0], replaced);
        assert_eq!(bounce_y(&p).keys.len(), bounce_y(&original).keys.len());
        assert!(matches!(inverse, SetKey { .. }));

        let between = key((1, 100), 3.0);
        assert!(between.at > at(&original) && between.at < bounce_y(&original).keys[1].at);
        let cmd =
            SetKey { comp: BALL, anim: BALL_BOUNCE, node: BALL_BODY, property: Property::Y, key: between };
        let (p, inverse) = round_trip(cmd);
        assert_eq!(bounce_y(&p).keys[1], between);
        assert!(matches!(inverse, RemoveKey { .. }));

        let cmd =
            SetKey { comp: BALL, anim: BALL_BOUNCE, node: BALL_BODY, property: Property::X, key: between };
        let (p, inverse) = round_trip(cmd);
        let tracks = &p.compositions[&BALL].animations[&BALL_BOUNCE].tracks;
        assert_eq!(tracks.last().unwrap().property, Property::X);
        assert!(matches!(inverse, RemoveTrack { property: Property::X, .. }));
    }

    #[test]
    fn remove_key_and_last_key_removes_the_track() {
        let original = sample::bounce();
        let at = bounce_y(&original).keys[1].at;
        let (p, inverse) = round_trip(RemoveKey {
            comp: BALL,
            anim: BALL_BOUNCE,
            node: BALL_BODY,
            property: Property::Y,
            at,
        });
        assert_eq!(bounce_y(&p).keys.len(), bounce_y(&original).keys.len() - 1);
        assert!(matches!(inverse, SetKey { .. }));

        // Remove every key but one, then the last one.
        let mut p = original.clone();
        let keys = bounce_y(&original).keys.clone();
        for k in &keys[1..] {
            RemoveKey { comp: BALL, anim: BALL_BOUNCE, node: BALL_BODY, property: Property::Y, at: k.at }
                .apply(&mut p)
                .unwrap();
        }
        let last = RemoveKey {
            comp: BALL,
            anim: BALL_BOUNCE,
            node: BALL_BODY,
            property: Property::Y,
            at: keys[0].at,
        };
        let before = p.clone();
        let inverse = last.apply(&mut p).unwrap();
        assert!(p.compositions[&BALL].animations[&BALL_BOUNCE].track(BALL_BODY, Property::Y).is_none());
        assert!(matches!(inverse, InsertTrack { index: 0, .. }));
        inverse.apply(&mut p).unwrap();
        assert_eq!(p, before);
    }

    #[test]
    fn insert_and_remove_track() {
        let (p, inverse) = round_trip(RemoveTrack {
            comp: BALL,
            anim: BALL_SQUASH,
            node: BALL_BODY,
            property: Property::ScaleX,
        });
        assert!(p.compositions[&BALL].animations[&BALL_SQUASH].track(BALL_BODY, Property::ScaleX).is_none());
        assert!(matches!(inverse, InsertTrack { .. }));

        let track = Track { node: BALL_BODY, property: Property::Rotation, keys: vec![key((0, 1), 45.0)] };
        let (p, _) =
            round_trip(InsertTrack { comp: BALL, anim: BALL_SQUASH, index: 0, track: track.clone() });
        assert_eq!(p.compositions[&BALL].animations[&BALL_SQUASH].tracks[0], track);
    }

    #[test]
    fn add_node() {
        let node = Node::new("new", NodeKind::Group).with_rest(Props::at(5.0, 5.0));
        let (p, inverse) = round_trip(AddNode { comp: STAGE, parent: STAGE_ROOT, index: 1, id: NEW, node });
        let stage = &p.compositions[&STAGE];
        assert_eq!(stage.nodes[&STAGE_ROOT].children[1], NEW);
        assert_eq!(stage.nodes[&NEW].name, "new");
        assert_eq!(inverse, RemoveNode { comp: STAGE, node: NEW });
    }

    #[test]
    fn remove_node_takes_its_subtree_and_tracks() {
        // BALL_BODY is keyed by three tracks across two animations.
        let (p, inverse) = round_trip(RemoveNode { comp: BALL, node: BALL_BODY });
        let ball = &p.compositions[&BALL];
        assert!(!ball.nodes.contains_key(&BALL_BODY));
        assert!(ball.animations.values().all(|a| a.tracks.is_empty()));
        let Batch(restore) = inverse else { panic!("expected a batch, got {inverse:?}") };
        assert_eq!(restore.len(), 4, "one AddNode and three InsertTracks");

        // A group with a child: add one under GROUND, then remove GROUND.
        let mut p = sample::bounce();
        AddNode { comp: STAGE, parent: GROUND, index: 0, id: NEW, node: Node::new("child", NodeKind::Group) }
            .apply(&mut p)
            .unwrap();
        let before = p.clone();
        let inverse = RemoveNode { comp: STAGE, node: GROUND }.apply(&mut p).unwrap();
        assert!(!p.compositions[&STAGE].nodes.contains_key(&NEW));
        inverse.apply(&mut p).unwrap();
        assert_eq!(p, before);
    }

    #[test]
    fn move_node_reorders_and_reparents() {
        let children = |p: &Project, n| p.compositions[&STAGE].nodes[&n].children.clone();
        // STAGE_ROOT's children are [GROUND, EYES, FREE_BALL, SYNCED_BALL].
        let (p, inverse) = round_trip(MoveNode { comp: STAGE, node: GROUND, parent: STAGE_ROOT, index: 3 });
        assert_eq!(children(&p, STAGE_ROOT), vec![EYES, FREE_BALL, SYNCED_BALL, GROUND]);
        assert_eq!(inverse, MoveNode { comp: STAGE, node: GROUND, parent: STAGE_ROOT, index: 0 });

        let (p, _) = round_trip(MoveNode { comp: STAGE, node: EYES, parent: GROUND, index: 0 });
        assert_eq!(children(&p, GROUND), vec![EYES]);
        assert_eq!(children(&p, STAGE_ROOT), vec![GROUND, FREE_BALL, SYNCED_BALL]);
    }

    #[test]
    fn renames_timing_settings_and_prefs() {
        let (p, _) = round_trip(RenameNode { comp: STAGE, node: GROUND, name: "floor".into() });
        assert_eq!(p.compositions[&STAGE].nodes[&GROUND].name, "floor");

        let (p, _) = round_trip(RenameAnimation { comp: BALL, anim: BALL_BOUNCE, name: "hop".into() });
        assert_eq!(p.compositions[&BALL].animations[&BALL_BOUNCE].name, "hop");

        let (p, _) = round_trip(SetAnimationTiming {
            comp: BALL,
            anim: BALL_BOUNCE,
            duration: Time::from_secs(2),
            looping: LoopMode::PingPong,
            step: Some(Time::from_ratio(1, 12)),
        });
        let a = &p.compositions[&BALL].animations[&BALL_BOUNCE];
        assert_eq!(
            (a.duration, a.looping, a.step),
            (Time::from_secs(2), LoopMode::PingPong, Some(Time::from_ratio(1, 12)))
        );

        let settings = ProjectSettings {
            stage_width: 320,
            stage_height: 180,
            background: Color::BLACK,
            pixel_art: true,
        };
        let (p, _) = round_trip(SetSettings(settings));
        assert_eq!(p.settings, settings);

        let prefs = EditorPrefs {
            time_grid: TimeGrid::new(30),
            time_snap: false,
            spatial_grid: 8.0,
            spatial_snap: true,
        };
        let (p, _) = round_trip(SetEditorPrefs(prefs));
        assert_eq!(p.editor, prefs);
    }

    #[test]
    fn batches_apply_in_order_and_undo_in_reverse() {
        let (p, inverse) = round_trip(Batch(vec![
            AddNode {
                comp: STAGE,
                parent: STAGE_ROOT,
                index: 0,
                id: NEW,
                node: Node::new("n", NodeKind::Group),
            },
            RenameNode { comp: STAGE, node: NEW, name: "renamed".into() },
            MoveNode { comp: STAGE, node: NEW, parent: GROUND, index: 0 },
        ]));
        assert_eq!(p.compositions[&STAGE].nodes[&NEW].name, "renamed");
        let Batch(inverses) = inverse else { panic!() };
        assert!(matches!(inverses[0], MoveNode { .. }));
        assert!(matches!(inverses[2], RemoveNode { .. }));
    }

    #[test]
    fn a_failing_batch_changes_nothing() {
        let err = fails(Batch(vec![
            RenameNode { comp: STAGE, node: GROUND, name: "floor".into() },
            AddNode {
                comp: STAGE,
                parent: STAGE_ROOT,
                index: 0,
                id: NEW,
                node: Node::new("n", NodeKind::Group),
            },
            RemoveNode { comp: STAGE, node: STAGE_ROOT },
        ]));
        assert!(matches!(err, E::InBatch { index: 2, ref source } if matches!(**source, E::RootNode { .. })));
    }

    #[test]
    fn lookups_that_fail() {
        let nowhere = CompId::from_raw(1);
        let missing = NodeId::from_raw(1);
        let no_anim = AnimId::from_raw(1);
        let y = Property::Y;
        let k = key((0, 1), 0.0);

        assert!(matches!(
            fails(RenameNode { comp: nowhere, node: GROUND, name: "x".into() }),
            E::MissingComposition(_)
        ));
        assert!(matches!(
            fails(SetRest { comp: STAGE, node: missing, rest: Props::default() }),
            E::MissingNode { .. }
        ));
        assert!(matches!(
            fails(SetKey { comp: BALL, anim: BALL_BOUNCE, node: missing, property: y, key: k }),
            E::MissingNode { .. }
        ));
        assert!(matches!(
            fails(SetKey { comp: BALL, anim: no_anim, node: BALL_BODY, property: y, key: k }),
            E::MissingAnimation { .. }
        ));
        assert!(matches!(
            fails(RemoveKey {
                comp: BALL,
                anim: BALL_BOUNCE,
                node: BALL_BODY,
                property: Property::X,
                at: Time::ZERO
            }),
            E::MissingTrack { .. }
        ));
        assert!(matches!(
            fails(RemoveKey {
                comp: BALL,
                anim: BALL_BOUNCE,
                node: BALL_BODY,
                property: y,
                at: Time::from_flicks(7)
            }),
            E::MissingKey { .. }
        ));
        assert!(matches!(
            fails(RemoveTrack { comp: BALL, anim: BALL_BOUNCE, node: BALL_BODY, property: Property::X }),
            E::MissingTrack { .. }
        ));
        let track = bounce_y(&sample::bounce()).clone();
        assert!(matches!(
            fails(InsertTrack { comp: BALL, anim: BALL_BOUNCE, index: 0, track: track.clone() }),
            E::TrackExists { .. }
        ));
        let track = Track { property: Property::X, ..track };
        assert!(matches!(
            fails(InsertTrack { comp: BALL, anim: BALL_BOUNCE, index: 5, track }),
            E::IndexOutOfRange { index: 5, len: 1 }
        ));
        assert!(matches!(
            fails(RenameAnimation { comp: BALL, anim: no_anim, name: "x".into() }),
            E::MissingAnimation { .. }
        ));
    }

    #[test]
    fn structural_edits_that_fail() {
        let group = || Node::new("n", NodeKind::Group);
        assert!(matches!(
            fails(AddNode { comp: STAGE, parent: STAGE_ROOT, index: 0, id: GROUND, node: group() }),
            E::NodeExists { .. }
        ));
        assert!(matches!(
            fails(AddNode {
                comp: STAGE,
                parent: STAGE_ROOT,
                index: 0,
                id: NEW,
                node: group().with_children(vec![GROUND])
            }),
            E::NodeHasChildren { .. }
        ));
        assert!(matches!(
            fails(AddNode { comp: STAGE, parent: STAGE_ROOT, index: 5, id: NEW, node: group() }),
            E::IndexOutOfRange { index: 5, len: 4 }
        ));
        assert!(matches!(fails(RemoveNode { comp: STAGE, node: STAGE_ROOT }), E::RootNode { .. }));
        assert!(matches!(fails(RemoveNode { comp: STAGE, node: NEW }), E::MissingNode { .. }));
        assert!(matches!(
            fails(MoveNode { comp: STAGE, node: STAGE_ROOT, parent: GROUND, index: 0 }),
            E::RootNode { .. }
        ));
        assert!(matches!(
            fails(MoveNode { comp: STAGE, node: GROUND, parent: GROUND, index: 0 }),
            E::MoveIntoOwnSubtree { .. }
        ));
        // Moving within the same parent: the node itself doesn't count.
        assert!(matches!(
            fails(MoveNode { comp: STAGE, node: GROUND, parent: STAGE_ROOT, index: 4 }),
            E::IndexOutOfRange { index: 4, len: 3 }
        ));
    }

    #[test]
    fn edits_that_would_make_the_project_invalid() {
        let invalid = |err: E, pred: fn(&ValidationError) -> bool| match err {
            E::Invalid(errors) => assert!(errors.iter().any(pred), "{errors:?}"),
            other => panic!("expected Invalid, got {other:?}"),
        };
        // Wrong value kind.
        let bad = Key::new(Time::ZERO, Value::Bool(true), Ease::Linear);
        invalid(
            fails(SetKey { comp: BALL, anim: BALL_BOUNCE, node: BALL_BODY, property: Property::Y, key: bad }),
            |e| matches!(e, ValidationError::WrongValueKind { .. }),
        );
        // Key past the end.
        invalid(
            fails(SetKey {
                comp: BALL,
                anim: BALL_BOUNCE,
                node: BALL_BODY,
                property: Property::Y,
                key: key((5, 1), 0.0),
            }),
            |e| matches!(e, ValidationError::KeyOutOfRange { .. }),
        );
        // Shortening an animation strands its keys.
        invalid(
            fails(SetAnimationTiming {
                comp: BALL,
                anim: BALL_BOUNCE,
                duration: Time::from_ratio(1, 10),
                looping: LoopMode::Loop,
                step: None,
            }),
            |e| matches!(e, ValidationError::KeyOutOfRange { .. }),
        );
        // A composition that contains itself.
        let instance = NodeKind::Instance(Instance { comp: STAGE, time: TimeMode::Free { animation: None } });
        invalid(
            fails(AddNode {
                comp: BALL,
                parent: BALL_ROOT,
                index: 0,
                id: NEW,
                node: Node::new("loop", instance),
            }),
            |e| matches!(e, ValidationError::RecursiveComposition(_)),
        );
        // A flipbook drawing that doesn't exist.
        invalid(
            fails(SetRest {
                comp: BLINKER,
                node: BLINKER_EYE,
                rest: Props { drawing: 99, ..Props::default() },
            }),
            |e| matches!(e, ValidationError::DrawingOutOfRange { .. }),
        );
    }

    #[test]
    fn errors_render_readably() {
        let text = fails(Batch(vec![RemoveNode { comp: STAGE, node: STAGE_ROOT }])).to_string();
        assert!(text.contains("batch") && text.contains("root node"), "{text}");
    }

    mod random_edits {
        use super::*;
        use proptest::prelude::*;

        /// Picks concrete commands for the current project from random
        /// numbers, so most of them refer to things that exist. Some are
        /// still invalid, which is fine: they must fail cleanly.
        fn pick(p: &Project, which: u8, n: [usize; 4], x: f32) -> Command {
            let nth = |len: usize, i: usize| i % len.max(1);
            let comps: Vec<_> = p.compositions.keys().copied().collect();
            let comp = comps[nth(comps.len(), n[0])];
            let c = &p.compositions[&comp];
            let nodes: Vec<_> = c.nodes.keys().copied().collect();
            let other = nodes[nth(nodes.len(), n[2])];
            let anims: Vec<_> = c.animations.keys().copied().collect();
            let anim = anims.get(nth(anims.len(), n[2])).copied().unwrap_or(AnimId::from_raw(1));
            // Half the time, aim at an existing track so key and track edits
            // find something to work on.
            let tracks = c.animations.get(&anim).map_or(&[][..], |a| &a.tracks[..]);
            let (node, property) = match tracks.get(nth(tracks.len(), n[1] / 2)) {
                Some(t) if n[1] % 2 == 1 => (t.node, t.property),
                _ => (nodes[nth(nodes.len(), n[1])], Property::ALL[nth(Property::ALL.len(), n[3])]),
            };
            let duration = c.animations.get(&anim).map_or(Time::from_secs(1), |a| a.duration);
            let at = Time::from_flicks(duration.flicks() / 8 * (n[3] % 9) as i64);
            let value = match property.value_kind() {
                crate::ValueKind::Number => Value::Number(x),
                crate::ValueKind::Bool => Value::Bool(x > 0.0),
                crate::ValueKind::Rgba => Value::Rgba([x, 0.5, 0.5, 1.0]),
                crate::ValueKind::Blend => Value::Blend(Default::default()),
                crate::ValueKind::Index => Value::Index(n[3] as u32 % 3),
                crate::ValueKind::Time => Value::Time(at),
            };
            let children = c.nodes[&other].children.len();
            match which % 9 {
                0 => SetRest { comp, node, rest: Props::at(x, -x) },
                1 => SetKey { comp, anim, node, property, key: Key::new(at, value, Ease::Linear) },
                2 => {
                    let at = c
                        .animations
                        .get(&anim)
                        .and_then(|a| a.track(node, property))
                        .map_or(at, |t| t.keys[nth(t.keys.len(), n[3])].at);
                    RemoveKey { comp, anim, node, property, at }
                }
                3 => RemoveTrack { comp, anim, node, property },
                4 => {
                    let id = NodeId::from_raw(0x00ee_0000 + n[0] as u64 % 64);
                    let kind = if n[3].is_multiple_of(4) {
                        NodeKind::Instance(Instance {
                            comp: comps[nth(comps.len(), n[1])],
                            time: TimeMode::Free { animation: None },
                        })
                    } else {
                        NodeKind::Group
                    };
                    AddNode {
                        comp,
                        parent: other,
                        index: nth(children + 1, n[3]),
                        id,
                        node: Node::new("n", kind),
                    }
                }
                5 => RemoveNode { comp, node },
                6 => MoveNode { comp, node, parent: other, index: nth(children + 1, n[3]) },
                7 => RenameNode { comp, node, name: format!("n{}", n[3]) },
                _ => SetAnimationTiming {
                    comp,
                    anim,
                    duration: Time::from_flicks(duration.flicks() * (1 + n[3] as i64 % 3) / 2),
                    looping: LoopMode::Loop,
                    step: None,
                },
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(128))]

            /// Any sequence of edits keeps the project valid, failed edits
            /// change nothing, and applying the inverses in reverse gets the
            /// original back exactly.
            #[test]
            fn edits_keep_the_project_valid_and_undo_exactly(
                seeds in proptest::collection::vec(
                    (any::<u8>(), any::<[usize; 4]>(), -100.0f32..100.0, any::<bool>()),
                    1..40,
                )
            ) {
                let original = sample::bounce();
                let mut p = original.clone();
                let mut inverses = Vec::new();
                let mut pending = Vec::new();
                for (which, n, x, flush) in seeds {
                    // Group some edits into batches.
                    pending.push(pick(&p, which, n, x));
                    if !flush {
                        continue;
                    }
                    let cmd = Command::batch(std::mem::take(&mut pending));
                    let before = p.clone();
                    match cmd.apply(&mut p) {
                        Ok(inverse) => inverses.push(inverse),
                        Err(_) => prop_assert_eq!(&p, &before),
                    }
                    prop_assert!(p.validate().is_ok(), "{:?}", p.validate());
                }
                for inverse in inverses.iter().rev() {
                    inverse.apply(&mut p).unwrap();
                }
                prop_assert_eq!(p, original);
            }
        }
    }
}
