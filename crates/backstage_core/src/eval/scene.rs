//! Output of `evaluate`: a flat list of things to draw, in painter's order.

use super::runtime::InstancePath;
use crate::geom::ColorTransform;
use crate::id::{AssetId, NodeId};
use crate::node::BlendMode;
use crate::shape::Shape;
use glam::Affine2;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene<'p> {
    pub items: Vec<DrawItem<'p>>,
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
