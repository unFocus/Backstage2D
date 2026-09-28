//! Nodes: the fixed content tree of a composition.

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
}

impl Node {
    pub fn new(name: impl Into<String>, kind: NodeKind) -> Self {
        Self { name: name.into(), kind, rest: Props::default(), children: Vec::new() }
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
