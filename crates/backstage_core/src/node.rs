//! Nodes: the fixed content tree of a composition.

use crate::animation::{Property, Value};
use crate::geom::{ColorTransform, Transform};
use crate::id::{AnimId, AssetId, CompId, DrawingId, NodeId};
use crate::shape::Shape;
use crate::time::Time;
use serde::{Deserialize, Serialize};

/// An element of a composition's tree. Its ID is the key in
/// [`Composition::nodes`](crate::Composition::nodes). Sibling order in
/// `children` is z-order (later draws on top).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub kind: NodeKind,
    /// Property values when no animation or script overrides them.
    #[serde(default, skip_serializing_if = "is_default")]
    pub rest: Props,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NodeId>,
    /// Editor-only state (hide, lock, outline). Saved with the project,
    /// but playback never looks at it: `evaluate` ignores it.
    #[serde(default, skip_serializing_if = "is_default")]
    pub editor: NodeFlags,
}

/// How the editor treats a node and its subtree. None of this affects
/// playback. See [`crate::editor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeFlags {
    /// Not drawn on the editor's stage.
    #[serde(skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Can't be edited on stage or in Properties.
    #[serde(skip_serializing_if = "is_false")]
    pub locked: bool,
    /// Drawn as outlines on the editor's stage.
    #[serde(skip_serializing_if = "is_false")]
    pub outline: bool,
}

impl Node {
    pub fn new(name: impl Into<String>, kind: NodeKind) -> Self {
        Self {
            name: name.into(),
            kind,
            rest: Props::default(),
            children: Vec::new(),
            editor: NodeFlags::default(),
        }
    }

    pub fn with_rest(mut self, rest: Props) -> Self {
        self.rest = rest;
        self
    }

    pub fn with_children(mut self, children: Vec<NodeId>) -> Self {
        self.children = children;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    /// A container. Top-level groups are shown as layers in the editor.
    Group,
    /// Vector artwork.
    Shape(Shape),
    /// Frame-by-frame drawings; the `drawing` property picks one.
    Flipbook(Vec<Drawing>),
    /// A bitmap asset.
    Bitmap(AssetId),
    /// A nested composition.
    Instance(Instance),
    /// Clips the siblings drawn after it to its children's coverage.
    Mask,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Drawing {
    pub id: DrawingId,
    pub shape: Shape,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub comp: CompId,
    pub time: TimeMode,
}

/// How a nested composition's clock relates to its parent's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TimeMode {
    /// Plays one of the child's animations on the parent's clock, shifted by
    /// `offset` (Flash's Graphic). `offset` is animatable (`TimeOffset`).
    Synced {
        animation: AnimId,
        #[serde(default, skip_serializing_if = "is_default")]
        offset: Time,
        #[serde(default, skip_serializing_if = "is_default")]
        repeat: Repeat,
    },
    /// Runs on its own clock with its own mixer (Flash's MovieClip). Starts
    /// with `animation`, or the composition's default animation.
    Free {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        animation: Option<AnimId>,
    },
}

/// What a synced instance does past the end of its animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Repeat {
    /// Use the animation's own loop mode.
    #[default]
    Natural,
    /// Play once and hold the last pose.
    Once,
    /// Always show the pose at this child time (Flash's "single frame").
    Hold(Time),
}

/// Animatable properties of a node, with their rest values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Props {
    #[serde(skip_serializing_if = "is_default")]
    pub transform: Transform,
    #[serde(skip_serializing_if = "is_one")]
    pub opacity: f32,
    #[serde(skip_serializing_if = "is_default")]
    pub color: ColorTransform,
    #[serde(skip_serializing_if = "is_true")]
    pub visible: bool,
    #[serde(skip_serializing_if = "is_default")]
    pub blend: BlendMode,
    /// Index into a flipbook's drawings. Ignored for other kinds.
    #[serde(skip_serializing_if = "is_default")]
    pub drawing: u32,
}

impl Default for Props {
    fn default() -> Self {
        Self {
            transform: Transform::default(),
            opacity: 1.0,
            color: ColorTransform::default(),
            visible: true,
            blend: BlendMode::default(),
            drawing: 0,
        }
    }
}

impl Props {
    pub fn at(x: f32, y: f32) -> Self {
        Self { transform: Transform::at(x, y), ..Self::default() }
    }

    /// The value of `property` in these properties.
    pub fn get(&self, property: Property) -> Value {
        let props = self;
        let t = &props.transform;
        match property {
            Property::X => Value::Number(t.position.x),
            Property::Y => Value::Number(t.position.y),
            Property::Rotation => Value::Number(t.rotation),
            Property::ScaleX => Value::Number(t.scale.x),
            Property::ScaleY => Value::Number(t.scale.y),
            Property::SkewX => Value::Number(t.skew.x),
            Property::SkewY => Value::Number(t.skew.y),
            Property::PivotX => Value::Number(t.pivot.x),
            Property::PivotY => Value::Number(t.pivot.y),
            Property::Opacity => Value::Number(props.opacity),
            Property::ColorMultiply => Value::Rgba(props.color.multiply),
            Property::ColorAdd => Value::Rgba(props.color.add),
            Property::Visible => Value::Bool(props.visible),
            Property::Blend => Value::Blend(props.blend),
            Property::Drawing => Value::Index(props.drawing),
            Property::TimeOffset => Value::Time(Time::ZERO),
        }
    }

    /// Sets `property` to `value`. A value of the wrong kind for the
    /// property changes nothing.
    pub fn set(&mut self, property: Property, value: Value) {
        let props = self;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Add,
    Subtract,
    Overlay,
    Darken,
    Lighten,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}
fn is_one(v: &f32) -> bool {
    *v == 1.0
}
fn is_true(v: &bool) -> bool {
    *v
}
fn is_false(v: &bool) -> bool {
    !*v
}

#[cfg(test)]
mod tests {
    use super::Props;
    use crate::animation::{Property, Value};
    use crate::time::Time;

    /// A value of the right kind for `p` that differs from the default.
    fn other(p: Property) -> Value {
        match Props::default().get(p) {
            Value::Number(v) => Value::Number(v + 1.5),
            Value::Bool(v) => Value::Bool(!v),
            Value::Rgba(_) => Value::Rgba([0.25, 0.5, 0.75, 1.0]),
            Value::Blend(_) => Value::Blend(super::BlendMode::Screen),
            Value::Index(v) => Value::Index(v + 3),
            Value::Time(_) => Value::Time(Time::from_secs(1)),
        }
    }

    #[test]
    fn get_reads_what_set_wrote_and_nothing_else_changes() {
        for p in Property::ALL {
            if p == Property::TimeOffset {
                continue; // lives on the instance, not in Props
            }
            let mut props = Props::default();
            props.set(p, other(p));
            assert_eq!(props.get(p), other(p), "{p:?}");
            for q in Property::ALL.into_iter().filter(|&q| q != p) {
                assert_eq!(props.get(q), Props::default().get(q), "setting {p:?} changed {q:?}");
            }
        }
    }

    #[test]
    fn a_value_of_the_wrong_kind_changes_nothing() {
        let mut props = Props::default();
        props.set(Property::X, Value::Bool(true));
        props.set(Property::Visible, Value::Number(0.0));
        assert_eq!(props, Props::default());
    }
}
