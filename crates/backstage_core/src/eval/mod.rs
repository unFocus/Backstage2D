//! `evaluate`: the document plus runtime state at a moment, as a flat list
//! of things to draw. Pure and deterministic. See ADR 0003.

pub mod clock;
pub mod ease;
pub mod runtime;
pub mod scene;
pub mod track;

pub use runtime::{InstancePath, Player, RuntimeState};
pub use scene::{DrawContent, DrawItem, Scene};

use crate::animation::{Animation, Property, Track, Value};
use crate::geom::{ColorTransform, Transform};
use crate::id::{CompId, NodeId};
use crate::node::{NodeKind, Props, Repeat, TimeMode};
use crate::project::{Composition, Project};
use crate::time::Time;
use glam::{Affine2, Mat2, Vec2};
use std::collections::BTreeMap;

/// Guards against runaway nesting in invalid data (validation already
/// rejects compositions that contain themselves).
const MAX_DEPTH: usize = 64;

/// The project's stage at global time `now`.
pub fn evaluate<'p>(project: &'p Project, state: &RuntimeState, now: Time) -> Scene<'p> {
    evaluate_from(project, state, project.root, now)
}

/// Any composition as the root (the editor edits one composition at a
/// time). The root runs as a free instance at the empty path, so its player
/// in `state` picks its animation and start time.
pub fn evaluate_from<'p>(project: &'p Project, state: &RuntimeState, root: CompId, now: Time) -> Scene<'p> {
    let mut eval = Evaluator { project, state, now, items: Vec::new() };
    if let Some(comp) = project.compositions.get(&root) {
        let path = InstancePath::new();
        let player = state.players.get(&path);
        let anim_id = player.and_then(|p| p.animation).or(comp.default_animation);
        let clock = now - player.map_or(Time::ZERO, |p| p.started);
        eval.composition(
            comp,
            anim_id.and_then(|a| comp.animations.get(&a)),
            clock,
            Repeat::Natural,
            &path,
            &Inherited::ROOT,
            0,
        );
    }
    Scene { items: eval.items }
}

/// What a node inherits from its ancestors.
#[derive(Clone, Copy)]
struct Inherited {
    transform: Affine2,
    opacity: f32,
    color: ColorTransform,
}

impl Inherited {
    const ROOT: Inherited = Inherited {
        transform: Affine2::IDENTITY,
        opacity: 1.0,
        color: ColorTransform { multiply: [1.0; 4], add: [0.0; 4] },
    };
}

/// One composition instance being walked: its animation, local time, and
/// tracks grouped by node.
struct Frame<'p> {
    comp: &'p Composition,
    local: Time,
    tracks: BTreeMap<NodeId, Vec<&'p Track>>,
    path: InstancePath,
    depth: usize,
}

struct Evaluator<'p, 's> {
    project: &'p Project,
    state: &'s RuntimeState,
    now: Time,
    items: Vec<DrawItem<'p>>,
}

impl<'p> Evaluator<'p, '_> {
    #[allow(clippy::too_many_arguments)]
    fn composition(
        &mut self,
        comp: &'p Composition,
        anim: Option<&'p Animation>,
        clock: Time,
        repeat: Repeat,
        path: &InstancePath,
        inherited: &Inherited,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        let local = anim.map_or(Time::ZERO, |a| clock::local_time(a, clock, repeat));
        let mut tracks: BTreeMap<NodeId, Vec<&'p Track>> = BTreeMap::new();
        for track in anim.map_or(&[][..], |a| &a.tracks) {
            tracks.entry(track.node).or_default().push(track);
        }
        let frame = Frame { comp, local, tracks, path: path.clone(), depth };
        self.node(&frame, comp.root, inherited);
    }

    fn node(&mut self, frame: &Frame<'p>, id: NodeId, parent: &Inherited) {
        let Some(node) = frame.comp.nodes.get(&id) else { return };
        let tracks = frame.tracks.get(&id).map_or(&[][..], Vec::as_slice);
        let mut props = node.rest;
        let mut time_offset = None;
        for track in tracks {
            if let Some(value) = track::sample_track(track, frame.local) {
                if track.property == Property::TimeOffset {
                    if let Value::Time(t) = value {
                        time_offset = Some(t);
                    }
                } else {
                    apply(&mut props, track.property, value);
                }
            }
        }
        if !props.visible {
            return;
        }
        let here = Inherited {
            transform: parent.transform * local_matrix(&props.transform),
            opacity: parent.opacity * props.opacity,
            color: compose(&props.color, &parent.color),
        };
        let emit = |this: &mut Self, content| {
            this.items.push(DrawItem {
                instance: frame.path.clone(),
                node: id,
                transform: here.transform,
                opacity: here.opacity,
                color: here.color,
                blend: props.blend,
                content,
            });
        };
        match &node.kind {
            NodeKind::Group => {}
            NodeKind::Shape(shape) => emit(self, DrawContent::Shape(shape)),
            NodeKind::Flipbook(drawings) => {
                if let Some(d) = drawings.get((props.drawing as usize).min(drawings.len().saturating_sub(1)))
                {
                    emit(self, DrawContent::Shape(&d.shape));
                }
            }
            NodeKind::Bitmap(asset) => emit(self, DrawContent::Bitmap(*asset)),
            NodeKind::Mask => return, // Not yet supported: mask and its children are skipped.
            NodeKind::Instance(instance) => {
                if let Some(child) = self.project.compositions.get(&instance.comp) {
                    let mut path = frame.path.clone();
                    path.push(id);
                    let (anim_id, clock, repeat) = match instance.time {
                        TimeMode::Free { animation } => {
                            let player = self.state.players.get(&path);
                            let anim =
                                player.and_then(|p| p.animation).or(animation).or(child.default_animation);
                            (anim, self.now - player.map_or(Time::ZERO, |p| p.started), Repeat::Natural)
                        }
                        TimeMode::Synced { animation, offset, repeat } => {
                            (Some(animation), frame.local + time_offset.unwrap_or(offset), repeat)
                        }
                    };
                    let anim = anim_id.and_then(|a| child.animations.get(&a));
                    self.composition(child, anim, clock, repeat, &path, &here, frame.depth + 1);
                }
            }
        }
        for &child in &node.children {
            self.node(frame, child, &here);
        }
    }
}

/// Writes a sampled track value into the node's properties.
fn apply(props: &mut Props, property: Property, value: Value) {
    let t = &mut props.transform;
    match (property, value) {
        (Property::X, Value::Number(v)) => t.position.x = v,
        (Property::Y, Value::Number(v)) => t.position.y = v,
        (Property::Rotation, Value::Number(v)) => t.rotation = v,
        (Property::ScaleX, Value::Number(v)) => t.scale.x = v,
        (Property::ScaleY, Value::Number(v)) => t.scale.y = v,
        (Property::SkewX, Value::Number(v)) => t.skew.x = v,
        (Property::SkewY, Value::Number(v)) => t.skew.y = v,
        (Property::PivotX, Value::Number(v)) => t.pivot.x = v,
        (Property::PivotY, Value::Number(v)) => t.pivot.y = v,
        (Property::Opacity, Value::Number(v)) => props.opacity = v,
        (Property::ColorMultiply, Value::Rgba(v)) => props.color.multiply = v,
        (Property::ColorAdd, Value::Rgba(v)) => props.color.add = v,
        (Property::Visible, Value::Bool(v)) => props.visible = v,
        (Property::Blend, Value::Blend(v)) => props.blend = v,
        (Property::Drawing, Value::Index(v)) => props.drawing = v,
        _ => {} // Mismatched kinds are rejected by validation.
    }
}

/// `T(position) · R(rotation) · Skew(x, y) · S(scale) · T(-pivot)`, where
/// the skew matrix is `[[1, tan x], [tan y, 1]]`. Angles are degrees;
/// positive rotation is clockwise on screen (y is down).
pub fn local_matrix(t: &Transform) -> Affine2 {
    let (tx, ty) = (t.skew.x.to_radians().tan(), t.skew.y.to_radians().tan());
    let skew = Mat2::from_cols(Vec2::new(1.0, ty), Vec2::new(tx, 1.0));
    Affine2::from_translation(t.position)
        * Affine2::from_angle(t.rotation.to_radians())
        * Affine2::from_mat2(skew)
        * Affine2::from_scale(t.scale)
        * Affine2::from_translation(-t.pivot)
}

/// Applies `child` first, then `parent`: `(c·mc + ac)·mp + ap`.
pub fn compose(child: &ColorTransform, parent: &ColorTransform) -> ColorTransform {
    ColorTransform {
        multiply: std::array::from_fn(|i| child.multiply[i] * parent.multiply[i]),
        add: std::array::from_fn(|i| child.add[i] * parent.multiply[i] + parent.add[i]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::EasePreset;
    use crate::node::Node;
    use crate::sample::{self, ids::*};

    const EPS: f32 = 1e-3;

    fn s(n: i64, d: i64) -> Time {
        Time::from_ratio(n, d)
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < EPS
    }

    /// World position of an item's local origin.
    fn origin(item: &DrawItem) -> Vec2 {
        item.transform.transform_point2(Vec2::ZERO)
    }

    fn item<'a, 'p>(scene: &'a Scene<'p>, instance: &[NodeId], node: NodeId) -> &'a DrawItem<'p> {
        scene.items.iter().find(|i| i.instance == instance && i.node == node).expect("item not in scene")
    }

    #[test]
    fn rotation_is_around_the_pivot() {
        let t = Transform {
            position: Vec2::new(100.0, 100.0),
            rotation: 90.0,
            pivot: Vec2::new(10.0, 0.0),
            ..Transform::default()
        };
        let m = local_matrix(&t);
        assert!(
            close(m.transform_point2(Vec2::new(10.0, 0.0)), Vec2::new(100.0, 100.0)),
            "pivot lands on position"
        );
        // 90° clockwise with y down: +x maps to +y.
        assert!(close(m.transform_point2(Vec2::new(20.0, 0.0)), Vec2::new(100.0, 110.0)));
    }

    #[test]
    fn scale_and_skew_follow_the_documented_matrix() {
        let t = Transform { scale: Vec2::new(2.0, 3.0), skew: Vec2::new(45.0, 0.0), ..Transform::default() };
        let m = local_matrix(&t);
        // Scale first: (1, 1) → (2, 3); then skew x by tan 45° · y: x = 2 + 3.
        assert!(close(m.transform_point2(Vec2::new(1.0, 1.0)), Vec2::new(5.0, 3.0)));
    }

    #[test]
    fn color_transforms_compose_child_first() {
        let child = ColorTransform { multiply: [0.5; 4], add: [0.1, 0.0, 0.0, 0.0] };
        let parent = ColorTransform { multiply: [2.0; 4], add: [0.0, 0.2, 0.0, 0.0] };
        let c = compose(&child, &parent);
        // (x·0.5 + 0.1)·2 + (0, 0.2) for x = 1 → (1.2, 1.2, …)
        assert_eq!(c.multiply[0], 1.0);
        assert!((1.0 * c.multiply[0] + c.add[0] - 1.2).abs() < EPS);
        assert!((1.0 * c.multiply[1] + c.add[1] - 1.2).abs() < EPS);
    }

    #[test]
    fn parents_compose_and_invisible_subtrees_are_skipped() {
        let mut p = sample::bounce();
        let ball = p.compositions.get_mut(&BALL).unwrap();
        let root = ball.nodes.get_mut(&BALL_ROOT).unwrap();
        root.rest.opacity = 0.5;
        root.rest.transform.position = Vec2::new(10.0, 0.0);
        let scene = evaluate(&p, &RuntimeState::default(), s(1, 2));
        let body = item(&scene, &[FREE_BALL], BALL_BODY);
        assert!(close(origin(body), Vec2::new(410.0, 300.0)), "{:?}", origin(body));
        assert_eq!(body.opacity, 0.5);

        p.compositions.get_mut(&BALL).unwrap().nodes.get_mut(&BALL_ROOT).unwrap().rest.visible = false;
        let scene = evaluate(&p, &RuntimeState::default(), s(1, 2));
        assert!(scene.items.iter().all(|i| i.node != BALL_BODY), "invisible ball subtree must be skipped");
    }

    #[test]
    fn sample_bounces_over_time() {
        let p = sample::bounce();
        let state = RuntimeState::default();
        let at = |t: Time| evaluate(&p, &state, t);

        // Painter's order follows the stage's children: ground, eyes, free ball, synced ball.
        let scene = at(Time::ZERO);
        let order: Vec<_> = scene.items.iter().map(|i| (i.instance.clone(), i.node)).collect();
        assert_eq!(
            order,
            vec![
                (vec![], GROUND),
                (vec![EYES], BLINKER_EYE),
                (vec![FREE_BALL], BALL_BODY),
                (vec![SYNCED_BALL], BALL_BODY)
            ]
        );

        // Free ball: bounce keys Y from -180 (t = 0) to 0 (t = 0.5 s), on a rest y of 300.
        assert!(close(origin(item(&scene, &[FREE_BALL], BALL_BODY)), Vec2::new(400.0, 120.0)));
        let half = at(s(1, 2));
        assert!(close(origin(item(&half, &[FREE_BALL], BALL_BODY)), Vec2::new(400.0, 300.0)));

        // Synced ball at 0.5 s: stage-local 0.5 s plus the 0.25 s offset = 0.75 s into
        // bounce, halfway up (ease-out) from 0 to -180. X follows the stage's main track.
        let y = 300.0 - 180.0 * ease::ease(crate::animation::Ease::Preset(EasePreset::EaseOut), 0.5);
        let x = 120.0 + 140.0 * ease::ease(crate::animation::Ease::Preset(EasePreset::EaseInOut), 0.125);
        assert!(close(origin(item(&half, &[SYNCED_BALL], BALL_BODY)), Vec2::new(x, y)));
    }

    #[test]
    fn flipbook_follows_hold_keys() {
        let p = sample::bounce();
        let drawing_at = |t: Time| {
            let scene = evaluate(&p, &RuntimeState::default(), t);
            let DrawContent::Shape(shape) = item(&scene, &[EYES], BLINKER_EYE).content else { panic!() };
            let NodeKind::Flipbook(drawings) = &p.compositions[&BLINKER].nodes[&BLINKER_EYE].kind else {
                panic!()
            };
            drawings.iter().position(|d| std::ptr::eq(&d.shape, shape)).unwrap()
        };
        assert_eq!(drawing_at(Time::ZERO), 0);
        assert_eq!(drawing_at(s(155, 100)), 1);
        assert_eq!(drawing_at(s(16, 10)), 2);
        assert_eq!(drawing_at(s(17, 10)), 1);
        assert_eq!(drawing_at(s(19, 10)), 0);
    }

    #[test]
    fn players_shift_free_instances() {
        let p = sample::bounce();
        let mut state = RuntimeState::default();
        state.players.insert(vec![FREE_BALL], Player { animation: None, started: s(1, 4) });
        let scene = evaluate(&p, &state, s(3, 4));
        assert!(close(origin(item(&scene, &[FREE_BALL], BALL_BODY)), Vec2::new(400.0, 300.0)));
        // The synced ball ignores players: it follows its parent.
        let synced = evaluate(&p, &RuntimeState::default(), s(3, 4));
        assert_eq!(item(&scene, &[SYNCED_BALL], BALL_BODY), item(&synced, &[SYNCED_BALL], BALL_BODY));
    }

    #[test]
    fn deterministic() {
        let p = sample::bounce();
        let state = RuntimeState::default();
        for t in [Time::ZERO, s(7, 24), s(3, 1), s(-1, 2)] {
            assert_eq!(evaluate(&p, &state, t), evaluate(&p, &state, t));
        }
    }

    #[test]
    fn evaluate_from_any_composition() {
        let p = sample::bounce();
        let scene = evaluate_from(&p, &RuntimeState::default(), BALL, Time::ZERO);
        assert_eq!(scene.items.len(), 1);
        assert!(close(origin(&scene.items[0]), Vec2::new(0.0, -180.0)));
        assert!(
            evaluate_from(&p, &RuntimeState::default(), CompId::from_raw(1), Time::ZERO).items.is_empty()
        );
    }

    #[test]
    fn masks_are_skipped_for_now() {
        let mut p = sample::bounce();
        let stage = p.compositions.get_mut(&STAGE).unwrap();
        stage
            .nodes
            .insert(NodeId::from_raw(1), Node::new("mask", NodeKind::Mask).with_children(vec![GROUND]));
        stage.nodes.get_mut(&STAGE_ROOT).unwrap().children.retain(|&n| n != GROUND);
        stage.nodes.get_mut(&STAGE_ROOT).unwrap().children.insert(0, NodeId::from_raw(1));
        let scene = evaluate(&p, &RuntimeState::default(), Time::ZERO);
        assert!(scene.items.iter().all(|i| i.node != GROUND));
    }
}
