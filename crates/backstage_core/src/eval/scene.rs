//! Output of `evaluate`: a flat list of things to draw, in painter's order.

use super::runtime::InstancePath;
use crate::geom::ColorTransform;
use crate::id::{AssetId, NodeId};
use crate::node::{BlendMode, Props};
use crate::shape::Shape;
use glam::Affine2;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene<'p> {
    pub items: Vec<DrawItem<'p>>,
    /// Each node of the evaluated composition itself (not of instances
    /// inside it), groups included: what editing needs to turn a stage
    /// position into a node's own values. Nodes under an invisible one are
    /// left out.
    pub nodes: BTreeMap<NodeId, NodeState>,
}

/// A node as evaluated: its properties at that moment (rest values with
/// animations applied) and its transform to the stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeState {
    pub props: Props,
    /// Local (node) coordinates to stage pixels.
    pub world: Affine2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawItem<'p> {
    /// The composition instance this item belongs to.
    pub instance: InstancePath,
    /// The node within that composition.
    pub node: NodeId,
    /// Local (node) coordinates to stage pixels.
    pub transform: Affine2,
    /// Product of the node's and its ancestors' opacities.
    pub opacity: f32,
    /// Node's color transform composed with its ancestors'.
    pub color: ColorTransform,
    pub blend: BlendMode,
    pub content: DrawContent<'p>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawContent<'p> {
    Shape(&'p Shape),
    Bitmap(AssetId),
}
