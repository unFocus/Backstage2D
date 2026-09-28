//! Animations: named, time-based keyframe tracks over a composition's nodes.

use crate::id::NodeId;
use crate::node::BlendMode;
use crate::time::Time;
use serde::{Deserialize, Serialize};

/// A named animation. Its ID is the key in
/// [`Composition::animations`](crate::Composition::animations).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    pub name: String,
    pub duration: Time,
    #[serde(default, skip_serializing_if = "is_default")]
    pub looping: LoopMode,
    /// Hold values for this long ("on twos" / posterize). Keys stay
    /// continuous; stepping happens when evaluating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<Time>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<Track>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<Marker>,
}

impl Animation {
    pub fn new(name: impl Into<String>, duration: Time, looping: LoopMode) -> Self {
        Self { name: name.into(), duration, looping, step: None, tracks: Vec::new(), markers: Vec::new() }
    }

    pub fn track(&self, node: NodeId, property: Property) -> Option<&Track> {
        self.tracks.iter().find(|t| t.node == node && t.property == property)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LoopMode {
    #[default]
    Once,
    Loop,
    PingPong,
}

/// A named point in time, for events and script cues.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub name: String,
    pub at: Time,
}

/// Keys for one property of one node. At most one track per
/// `(node, property)` in an animation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub node: NodeId,
    pub property: Property,
    pub keys: Vec<Key>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Key {
    /// Time from the start of the animation.
    pub at: Time,
    pub value: Value,
    /// Easing from this key to the next.
    #[serde(default, skip_serializing_if = "is_default")]
    pub ease: Ease,
}

impl Key {
    pub fn new(at: Time, value: Value, ease: Ease) -> Self {
        Self { at, value, ease }
    }
}

/// An animatable property. Vector properties are split per axis so each axis
/// has its own keys and easing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Property {
    X,
    Y,
    Rotation,
    ScaleX,
    ScaleY,
    SkewX,
    SkewY,
    PivotX,
    PivotY,
    Opacity,
    ColorMultiply,
    ColorAdd,
    Visible,
    Blend,
    /// Flipbook drawing index.
    Drawing,
    /// Offset of a synced instance's clock.
    TimeOffset,
}

impl Property {
    pub const ALL: [Property; 16] = [
        Property::X,
        Property::Y,
        Property::Rotation,
        Property::ScaleX,
        Property::ScaleY,
        Property::SkewX,
        Property::SkewY,
        Property::PivotX,
        Property::PivotY,
        Property::Opacity,
        Property::ColorMultiply,
        Property::ColorAdd,
        Property::Visible,
        Property::Blend,
        Property::Drawing,
        Property::TimeOffset,
    ];

    /// The kind of [`Value`] this property's keys hold.
    pub fn value_kind(self) -> ValueKind {
        match self {
            Property::X
            | Property::Y
            | Property::Rotation
            | Property::ScaleX
            | Property::ScaleY
            | Property::SkewX
            | Property::SkewY
            | Property::PivotX
            | Property::PivotY
            | Property::Opacity => ValueKind::Number,
            Property::ColorMultiply | Property::ColorAdd => ValueKind::Rgba,
            Property::Visible => ValueKind::Bool,
            Property::Blend => ValueKind::Blend,
            Property::Drawing => ValueKind::Index,
            Property::TimeOffset => ValueKind::Time,
        }
    }

    /// Discrete properties can't be interpolated or blended: they hold until
    /// the next key, and blending takes the strongest animation's value.
    pub fn is_discrete(self) -> bool {
        matches!(self.value_kind(), ValueKind::Bool | ValueKind::Blend | ValueKind::Index)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Number(f32),
    Bool(bool),
    /// Four channels, e.g. a color transform's multiply or add.
    Rgba([f32; 4]),
    Blend(BlendMode),
    Index(u32),
    Time(Time),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Number,
    Bool,
    Rgba,
    Blend,
    Index,
    Time,
}

impl Value {
    pub fn kind(&self) -> ValueKind {
        match self {
            Value::Number(_) => ValueKind::Number,
            Value::Bool(_) => ValueKind::Bool,
            Value::Rgba(_) => ValueKind::Rgba,
            Value::Blend(_) => ValueKind::Blend,
            Value::Index(_) => ValueKind::Index,
            Value::Time(_) => ValueKind::Time,
        }
    }
}

/// How a value moves from one key to the next.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Ease {
    /// Jump at the next key.
    Hold,
    #[default]
    Linear,
    /// CSS-style cubic bezier: control points (x1, y1) and (x2, y2).
    CubicBezier(f32, f32, f32, f32),
    Preset(EasePreset),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EasePreset {
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl EasePreset {
    /// The equivalent CSS cubic bezier control points.
    pub fn control_points(self) -> (f32, f32, f32, f32) {
        match self {
            EasePreset::EaseIn => (0.42, 0.0, 1.0, 1.0),
            EasePreset::EaseOut => (0.0, 0.0, 0.58, 1.0),
            EasePreset::EaseInOut => (0.42, 0.0, 0.58, 1.0),
        }
    }
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}
