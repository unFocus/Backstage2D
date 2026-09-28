//! Structural checks on a project. `load` runs them; edit commands (M2) will
//! keep them true.

use crate::animation::{Property, Value, ValueKind};
use crate::id::{AnimId, AssetId, CompId, NodeId};
use crate::node::{NodeKind, Repeat, TimeMode};
use crate::project::{Composition, Project};
use crate::time::Time;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ValidationError {
    #[error("root composition {0} does not exist")]
    MissingRootComposition(CompId),
    #[error("composition stored under {key} has id {id}")]
    CompositionIdMismatch { key: CompId, id: CompId },
    #[error("{comp}: root node {node} does not exist")]
    MissingRootNode { comp: CompId, node: NodeId },
    #[error("{comp}: node {parent} lists missing child {child}")]
    MissingChild { comp: CompId, parent: NodeId, child: NodeId },
    #[error("{comp}: node {node} has more than one parent")]
    MultipleParents { comp: CompId, node: NodeId },
    #[error("{comp}: root node {node} is listed as a child")]
    RootHasParent { comp: CompId, node: NodeId },
    #[error("{comp}: node {node} is not reachable from the root")]
    Unreachable { comp: CompId, node: NodeId },
    #[error("{comp}: node {node} instances missing composition {target}")]
    MissingComposition { comp: CompId, node: NodeId, target: CompId },
    #[error("compositions contain themselves: {}", fmt_cycle(.0))]
    RecursiveComposition(Vec<CompId>),
    #[error("{comp}: {context} refers to missing animation {anim} of {target}")]
    MissingAnimation { comp: CompId, context: String, target: CompId, anim: AnimId },
    #[error("{comp}: node {node} refers to missing asset {asset}")]
    MissingAsset { comp: CompId, node: NodeId, asset: AssetId },
    #[error("{comp}: flipbook {node} has no drawings")]
    EmptyFlipbook { comp: CompId, node: NodeId },
    #[error("{comp}: node {node} uses drawing {index}, but has {len}")]
    DrawingOutOfRange { comp: CompId, node: NodeId, index: u32, len: usize },
    #[error("{comp}: node {node} holds at negative time {at}")]
    NegativeHold { comp: CompId, node: NodeId, at: Time },
    #[error("{comp}/{anim}: duration {duration} must be positive")]
    NonPositiveDuration { comp: CompId, anim: AnimId, duration: Time },
    #[error("{comp}/{anim}: step {step} must be positive")]
    NonPositiveStep { comp: CompId, anim: AnimId, step: Time },
    #[error("{comp}/{anim}: more than one {property:?} track for {node}")]
    DuplicateTrack { comp: CompId, anim: AnimId, node: NodeId, property: Property },
    #[error("{comp}/{anim}: track targets missing node {node}")]
    TrackMissingNode { comp: CompId, anim: AnimId, node: NodeId },
    #[error("{comp}/{anim}: {property:?} does not apply to node {node}")]
    PropertyNotApplicable { comp: CompId, anim: AnimId, node: NodeId, property: Property },
    #[error("{comp}/{anim}: {property:?} track for {node} has no keys")]
    EmptyTrack { comp: CompId, anim: AnimId, node: NodeId, property: Property },
    #[error("{comp}/{anim}: {property:?} key for {node} holds {found:?}, expected {expected:?}")]
    WrongValueKind {
        comp: CompId,
        anim: AnimId,
        node: NodeId,
        property: Property,
        expected: ValueKind,
        found: ValueKind,
    },
    #[error("{comp}/{anim}: {property:?} keys for {node} are not in strictly increasing time order")]
    KeysOutOfOrder { comp: CompId, anim: AnimId, node: NodeId, property: Property },
    #[error("{comp}/{anim}: key at {at} is outside 0..={duration}")]
    KeyOutOfRange { comp: CompId, anim: AnimId, at: Time, duration: Time },
    #[error("{comp}/{anim}: marker {name:?} at {at} is outside 0..={duration}")]
    MarkerOutOfRange { comp: CompId, anim: AnimId, name: String, at: Time, duration: Time },
}

fn fmt_cycle(cycle: &[CompId]) -> String {
    cycle.iter().map(ToString::to_string).collect::<Vec<_>>().join(" → ")
}

impl Project {
    /// Checks every structural invariant. Returns all problems found.
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        if !self.compositions.contains_key(&self.root) {
            errors.push(ValidationError::MissingRootComposition(self.root));
        }
        for (&key, comp) in &self.compositions {
            if key != comp.id {
                errors.push(ValidationError::CompositionIdMismatch { key, id: comp.id });
            }
            check_tree(comp, &mut errors);
            check_nodes(self, comp, &mut errors);
            check_animations(comp, &mut errors);
        }
        check_recursion(self, &mut errors);
        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }
}

fn check_tree(comp: &Composition, errors: &mut Vec<ValidationError>) {
    let c = comp.id;
    if !comp.nodes.contains_key(&comp.root) {
        errors.push(ValidationError::MissingRootNode { comp: c, node: comp.root });
        return;
    }
    let mut parents: BTreeMap<NodeId, usize> = BTreeMap::new();
    for (&parent, node) in &comp.nodes {
        for &child in &node.children {
            if !comp.nodes.contains_key(&child) {
                errors.push(ValidationError::MissingChild { comp: c, parent, child });
            }
            *parents.entry(child).or_default() += 1;
        }
    }
    for (&node, &count) in &parents {
        if node == comp.root {
            errors.push(ValidationError::RootHasParent { comp: c, node });
        } else if count > 1 {
            errors.push(ValidationError::MultipleParents { comp: c, node });
        }
    }
    // With one parent per node and a parentless root, everything reachable
    // forms a tree; any cycle is unreachable and reported here.
    let mut reached = BTreeSet::new();
    let mut stack = vec![comp.root];
    while let Some(id) = stack.pop() {
        if reached.insert(id)
            && let Some(node) = comp.nodes.get(&id)
        {
            stack.extend(node.children.iter().copied());
        }
    }
    for &node in comp.nodes.keys() {
        if !reached.contains(&node) {
            errors.push(ValidationError::Unreachable { comp: c, node });
        }
    }
}

fn check_nodes(project: &Project, comp: &Composition, errors: &mut Vec<ValidationError>) {
    let c = comp.id;
    for (&id, node) in &comp.nodes {
        match &node.kind {
            NodeKind::Instance(instance) => {
                let Some(target) = project.compositions.get(&instance.comp) else {
                    errors.push(ValidationError::MissingComposition {
                        comp: c,
                        node: id,
                        target: instance.comp,
                    });
                    continue;
                };
                let (anim, hold) = match instance.time {
                    TimeMode::Synced { animation, repeat, .. } => {
                        (Some(animation), if let Repeat::Hold(at) = repeat { Some(at) } else { None })
                    }
                    TimeMode::Free { animation } => (animation, None),
                };
                if let Some(anim) = anim
                    && !target.animations.contains_key(&anim)
                {
                    errors.push(ValidationError::MissingAnimation {
                        comp: c,
                        context: format!("node {id}"),
                        target: target.id,
                        anim,
                    });
                }
                if let Some(at) = hold
                    && at.is_negative()
                {
                    errors.push(ValidationError::NegativeHold { comp: c, node: id, at });
                }
            }
            NodeKind::Bitmap(asset) if !project.assets.contains_key(asset) => {
                errors.push(ValidationError::MissingAsset { comp: c, node: id, asset: *asset });
            }
            NodeKind::Flipbook(drawings) if drawings.is_empty() => {
                errors.push(ValidationError::EmptyFlipbook { comp: c, node: id });
            }
            NodeKind::Flipbook(drawings) if node.rest.drawing as usize >= drawings.len() => {
                errors.push(ValidationError::DrawingOutOfRange {
                    comp: c,
                    node: id,
                    index: node.rest.drawing,
                    len: drawings.len(),
                });
            }
            _ => {}
        }
    }
    if let Some(anim) = comp.default_animation
        && !comp.animations.contains_key(&anim)
    {
        errors.push(ValidationError::MissingAnimation {
            comp: c,
            context: "default_animation".into(),
            target: c,
            anim,
        });
    }
}

fn applies_to(property: Property, kind: &NodeKind) -> bool {
    match property {
        Property::Drawing => matches!(kind, NodeKind::Flipbook(_)),
        Property::TimeOffset => {
            matches!(kind, NodeKind::Instance(i) if matches!(i.time, TimeMode::Synced { .. }))
        }
        _ => true,
    }
}

fn check_animations(comp: &Composition, errors: &mut Vec<ValidationError>) {
    let c = comp.id;
    for (&a, anim) in &comp.animations {
        let duration = anim.duration;
        if duration <= Time::ZERO {
            errors.push(ValidationError::NonPositiveDuration { comp: c, anim: a, duration });
        }
        if let Some(step) = anim.step
            && step <= Time::ZERO
        {
            errors.push(ValidationError::NonPositiveStep { comp: c, anim: a, step });
        }
        let in_range = |at: Time| at >= Time::ZERO && at <= duration;
        let mut seen = BTreeSet::new();
        for track in &anim.tracks {
            let (node, property) = (track.node, track.property);
            if !seen.insert((node, property)) {
                errors.push(ValidationError::DuplicateTrack { comp: c, anim: a, node, property });
            }
            let Some(target) = comp.nodes.get(&node) else {
                errors.push(ValidationError::TrackMissingNode { comp: c, anim: a, node });
                continue;
            };
            if !applies_to(property, &target.kind) {
                errors.push(ValidationError::PropertyNotApplicable { comp: c, anim: a, node, property });
            }
            if track.keys.is_empty() {
                errors.push(ValidationError::EmptyTrack { comp: c, anim: a, node, property });
            }
            if track.keys.windows(2).any(|w| w[0].at >= w[1].at) {
                errors.push(ValidationError::KeysOutOfOrder { comp: c, anim: a, node, property });
            }
            for key in &track.keys {
                if key.value.kind() != property.value_kind() {
                    errors.push(ValidationError::WrongValueKind {
                        comp: c,
                        anim: a,
                        node,
                        property,
                        expected: property.value_kind(),
                        found: key.value.kind(),
                    });
                }
                if !in_range(key.at) {
                    errors.push(ValidationError::KeyOutOfRange { comp: c, anim: a, at: key.at, duration });
                }
                if let (Value::Index(index), NodeKind::Flipbook(drawings)) = (key.value, &target.kind)
                    && index as usize >= drawings.len()
                {
                    errors.push(ValidationError::DrawingOutOfRange {
                        comp: c,
                        node,
                        index,
                        len: drawings.len(),
                    });
                }
            }
        }
        for marker in &anim.markers {
            if !in_range(marker.at) {
                errors.push(ValidationError::MarkerOutOfRange {
                    comp: c,
                    anim: a,
                    name: marker.name.clone(),
                    at: marker.at,
                    duration,
                });
            }
        }
    }
}

/// Finds compositions that (transitively) instance themselves.
fn check_recursion(project: &Project, errors: &mut Vec<ValidationError>) {
    let children = |comp: &Composition| -> BTreeSet<CompId> {
        comp.nodes
            .values()
            .filter_map(|n| match &n.kind {
                NodeKind::Instance(i) if project.compositions.contains_key(&i.comp) => Some(i.comp),
                _ => None,
            })
            .collect()
    };
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }
    fn visit(
        id: CompId,
        project: &Project,
        children: &dyn Fn(&Composition) -> BTreeSet<CompId>,
        marks: &mut BTreeMap<CompId, Mark>,
        path: &mut Vec<CompId>,
        errors: &mut Vec<ValidationError>,
    ) {
        match marks.get(&id) {
            Some(Mark::Done) => return,
            Some(Mark::Visiting) => {
                let start = path.iter().position(|&p| p == id).unwrap();
                let mut cycle = path[start..].to_vec();
                cycle.push(id);
                errors.push(ValidationError::RecursiveComposition(cycle));
                return;
            }
            None => {}
        }
        marks.insert(id, Mark::Visiting);
        path.push(id);
        for child in children(&project.compositions[&id]) {
            visit(child, project, children, marks, path, errors);
        }
        path.pop();
        marks.insert(id, Mark::Done);
    }
    let mut marks = BTreeMap::new();
    for &id in project.compositions.keys() {
        visit(id, project, &children, &mut marks, &mut Vec::new(), errors);
    }
}

#[cfg(test)]
mod tests {
    use super::ValidationError as E;
    use crate::animation::{Ease, Key, Marker, Property, Track, Value};
    use crate::id::{AnimId, AssetId, CompId, NodeId};
    use crate::node::{Node, NodeKind, Repeat, TimeMode};
    use crate::project::Project;
    use crate::sample::{self, ids::*};
    use crate::time::Time;

    /// Breaks the sample with `f` and returns the validation errors.
    fn errors(f: impl FnOnce(&mut Project)) -> Vec<E> {
        let mut p = sample::bounce();
        f(&mut p);
        p.validate().expect_err("expected validation errors")
    }

    fn has(errors: &[E], pred: impl Fn(&E) -> bool) -> bool {
        errors.iter().any(pred)
    }

    #[test]
    fn sample_is_valid() {
        sample::bounce().validate().unwrap();
    }

    #[test]
    fn missing_root_composition() {
        let e = errors(|p| p.root = CompId::from_raw(1));
        assert!(has(&e, |e| matches!(e, E::MissingRootComposition(_))));
    }

    #[test]
    fn composition_id_mismatch() {
        let e = errors(|p| p.compositions.get_mut(&BALL).unwrap().id = CompId::from_raw(1));
        assert!(has(&e, |e| matches!(e, E::CompositionIdMismatch { .. })));
    }

    #[test]
    fn missing_root_node() {
        let e = errors(|p| p.compositions.get_mut(&BALL).unwrap().root = NodeId::from_raw(1));
        assert!(has(&e, |e| matches!(e, E::MissingRootNode { .. })));
    }

    #[test]
    fn missing_child() {
        let e = errors(|p| {
            let c = p.compositions.get_mut(&BALL).unwrap();
            c.nodes.get_mut(&BALL_ROOT).unwrap().children.push(NodeId::from_raw(1));
        });
        assert!(has(&e, |e| matches!(e, E::MissingChild { .. })));
    }

    #[test]
    fn multiple_parents() {
        let e = errors(|p| {
            let c = p.compositions.get_mut(&STAGE).unwrap();
            c.nodes.get_mut(&FREE_BALL).unwrap().children.push(GROUND);
        });
        assert!(has(&e, |e| matches!(e, E::MultipleParents { node, .. } if *node == GROUND)));
    }

    #[test]
    fn root_as_child_and_unreachable_cycle() {
        let e = errors(|p| {
            let c = p.compositions.get_mut(&BALL).unwrap();
            c.nodes.get_mut(&BALL_BODY).unwrap().children.push(BALL_ROOT);
        });
        assert!(has(&e, |e| matches!(e, E::RootHasParent { .. })));

        let e = errors(|p| {
            let c = p.compositions.get_mut(&BALL).unwrap();
            let (a, b) = (NodeId::from_raw(1), NodeId::from_raw(2));
            c.nodes.insert(a, Node::new("a", NodeKind::Group).with_children(vec![b]));
            c.nodes.insert(b, Node::new("b", NodeKind::Group).with_children(vec![a]));
        });
        assert!(has(&e, |e| matches!(e, E::Unreachable { .. })));
    }

    #[test]
    fn missing_composition_and_animation() {
        let e = errors(|p| {
            let c = p.compositions.get_mut(&STAGE).unwrap();
            let NodeKind::Instance(i) = &mut c.nodes.get_mut(&FREE_BALL).unwrap().kind else { panic!() };
            i.comp = CompId::from_raw(1);
        });
        assert!(has(&e, |e| matches!(e, E::MissingComposition { .. })));

        let e = errors(|p| {
            let c = p.compositions.get_mut(&STAGE).unwrap();
            let NodeKind::Instance(i) = &mut c.nodes.get_mut(&SYNCED_BALL).unwrap().kind else { panic!() };
            i.time = TimeMode::Synced {
                animation: AnimId::from_raw(1),
                offset: Time::ZERO,
                repeat: Repeat::Natural,
            };
        });
        assert!(has(&e, |e| matches!(e, E::MissingAnimation { .. })));

        let e =
            errors(|p| p.compositions.get_mut(&BALL).unwrap().default_animation = Some(AnimId::from_raw(1)));
        assert!(has(
            &e,
            |e| matches!(e, E::MissingAnimation { context, .. } if context == "default_animation")
        ));
    }

    #[test]
    fn negative_hold() {
        let e = errors(|p| {
            let c = p.compositions.get_mut(&STAGE).unwrap();
            let NodeKind::Instance(i) = &mut c.nodes.get_mut(&SYNCED_BALL).unwrap().kind else { panic!() };
            i.time = TimeMode::Synced {
                animation: BALL_BOUNCE,
                offset: Time::ZERO,
                repeat: Repeat::Hold(-Time::from_secs(1)),
            };
        });
        assert!(has(&e, |e| matches!(e, E::NegativeHold { .. })));
    }

    #[test]
    fn recursive_composition() {
        let e = errors(|p| {
            // Ball now contains the stage, which contains balls.
            let c = p.compositions.get_mut(&BALL).unwrap();
            let n = NodeId::from_raw(1);
            let instance = crate::node::Instance { comp: STAGE, time: TimeMode::Free { animation: None } };
            c.nodes.insert(n, Node::new("loop", NodeKind::Instance(instance)));
            c.nodes.get_mut(&BALL_ROOT).unwrap().children.push(n);
        });
        assert!(has(
            &e,
            |e| matches!(e, E::RecursiveComposition(cycle) if cycle.contains(&STAGE) && cycle.contains(&BALL))
        ));
    }

    #[test]
    fn missing_asset_and_flipbook_ranges() {
        let e = errors(|p| {
            let c = p.compositions.get_mut(&BALL).unwrap();
            c.nodes.get_mut(&BALL_BODY).unwrap().kind = NodeKind::Bitmap(AssetId::from_raw(1));
        });
        assert!(has(&e, |e| matches!(e, E::MissingAsset { .. })));

        let e = errors(|p| {
            let c = p.compositions.get_mut(&BLINKER).unwrap();
            c.nodes.get_mut(&BLINKER_EYE).unwrap().rest.drawing = 3;
        });
        assert!(has(&e, |e| matches!(e, E::DrawingOutOfRange { index: 3, len: 3, .. })));

        let e = errors(|p| {
            let c = p.compositions.get_mut(&BLINKER).unwrap();
            c.nodes.get_mut(&BLINKER_EYE).unwrap().kind = NodeKind::Flipbook(vec![]);
        });
        assert!(has(&e, |e| matches!(e, E::EmptyFlipbook { .. })));

        let e = errors(|p| {
            let a = p.compositions.get_mut(&BLINKER).unwrap().animations.get_mut(&BLINKER_BLINK).unwrap();
            a.tracks[0].keys[1].value = Value::Index(7);
        });
        assert!(has(&e, |e| matches!(e, E::DrawingOutOfRange { index: 7, .. })));
    }

    #[test]
    fn durations_and_steps_must_be_positive() {
        let e = errors(|p| {
            let a = p.compositions.get_mut(&BALL).unwrap().animations.get_mut(&BALL_SQUASH).unwrap();
            a.duration = Time::ZERO;
            a.step = Some(Time::ZERO);
        });
        assert!(has(&e, |e| matches!(e, E::NonPositiveDuration { .. })));
        assert!(has(&e, |e| matches!(e, E::NonPositiveStep { .. })));
    }

    fn bounce_anim(p: &mut Project) -> &mut crate::animation::Animation {
        p.compositions.get_mut(&BALL).unwrap().animations.get_mut(&BALL_BOUNCE).unwrap()
    }

    #[test]
    fn track_problems() {
        let e = errors(|p| {
            let t = bounce_anim(p).tracks[0].clone();
            bounce_anim(p).tracks.push(t);
        });
        assert!(has(&e, |e| matches!(e, E::DuplicateTrack { .. })));

        let e = errors(|p| bounce_anim(p).tracks[0].node = NodeId::from_raw(1));
        assert!(has(&e, |e| matches!(e, E::TrackMissingNode { .. })));

        let e = errors(|p| {
            bounce_anim(p).tracks.push(Track { node: BALL_BODY, property: Property::Drawing, keys: vec![] });
        });
        assert!(has(&e, |e| matches!(e, E::PropertyNotApplicable { property: Property::Drawing, .. })));
        assert!(has(&e, |e| matches!(e, E::EmptyTrack { .. })));

        let e = errors(|p| bounce_anim(p).tracks[0].keys[1].value = Value::Bool(true));
        assert!(has(&e, |e| matches!(e, E::WrongValueKind { .. })));

        let e = errors(|p| bounce_anim(p).tracks[0].keys.swap(0, 1));
        assert!(has(&e, |e| matches!(e, E::KeysOutOfOrder { .. })));

        let e = errors(|p| {
            bounce_anim(p).tracks[0].keys.push(Key::new(
                Time::from_secs(2),
                Value::Number(0.0),
                Ease::Linear,
            ));
        });
        assert!(has(&e, |e| matches!(e, E::KeyOutOfRange { .. })));

        let e =
            errors(|p| bounce_anim(p).markers.push(Marker { name: "late".into(), at: Time::from_secs(5) }));
        assert!(has(&e, |e| matches!(e, E::MarkerOutOfRange { .. })));
    }

    #[test]
    fn time_offset_only_on_synced_instances() {
        let e = errors(|p| {
            let a = p.compositions.get_mut(&STAGE).unwrap().animations.get_mut(&STAGE_MAIN).unwrap();
            let keys = vec![Key::new(Time::ZERO, Value::Time(Time::ZERO), Ease::Linear)];
            a.tracks.push(Track { node: FREE_BALL, property: Property::TimeOffset, keys });
        });
        assert!(has(&e, |e| matches!(e, E::PropertyNotApplicable { property: Property::TimeOffset, .. })));
    }

    #[test]
    fn errors_render_readably() {
        let e = errors(|p| bounce_anim(p).tracks[0].keys.swap(0, 1));
        let text = e[0].to_string();
        assert!(text.contains("comp_") && text.contains("anim_") && text.contains("Y"), "{text}");
    }
}
