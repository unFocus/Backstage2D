//! Vector artwork: paths with fills and strokes.

use crate::geom::{Color, Vec2};
use serde::{Deserialize, Serialize};

/// Bezier circle approximation constant: control-point distance per radius.
const KAPPA: f32 = 0.552_284_8;

/// A vector drawing: styled paths, painted in order.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Shape {
    pub paths: Vec<StyledPath>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyledPath {
    pub path: Vec<PathCmd>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PathCmd {
    MoveTo(Vec2),
    LineTo(Vec2),
    QuadTo(Vec2, Vec2),
    CubicTo(Vec2, Vec2, Vec2),
    Close,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Paint {
    Solid(Color),
    LinearGradient { start: Vec2, end: Vec2, stops: Vec<GradientStop> },
    RadialGradient { center: Vec2, radius: f32, stops: Vec<GradientStop> },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    /// Position along the gradient, 0..=1.
    pub offset: f32,
    pub color: Color,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub paint: Paint,
    pub width: f32,
    #[serde(default, skip_serializing_if = "is_default")]
    pub cap: LineCap,
    #[serde(default, skip_serializing_if = "is_default")]
    pub join: LineJoin,
    #[serde(default = "default_miter_limit", skip_serializing_if = "is_default_miter_limit")]
    pub miter_limit: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LineCap {
    #[default]
    Round,
    Butt,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LineJoin {
    #[default]
    Round,
    Miter,
    Bevel,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}
fn default_miter_limit() -> f32 {
    4.0
}
fn is_default_miter_limit(v: &f32) -> bool {
    *v == default_miter_limit()
}

impl Stroke {
    pub fn solid(color: Color, width: f32) -> Self {
        Self {
            paint: Paint::Solid(color),
            width,
            cap: LineCap::default(),
            join: LineJoin::default(),
            miter_limit: default_miter_limit(),
        }
    }
}

impl Shape {
    pub fn single(path: Vec<PathCmd>, fill: Option<Paint>, stroke: Option<Stroke>) -> Self {
        Self { paths: vec![StyledPath { path, fill, stroke }] }
    }

    /// An axis-aligned rectangle with its top-left corner at `origin`.
    pub fn rect(origin: Vec2, size: Vec2, fill: Option<Paint>, stroke: Option<Stroke>) -> Self {
        Self::single(rect_path(origin, size), fill, stroke)
    }

    /// An ellipse centered on `center`.
    pub fn ellipse(center: Vec2, radii: Vec2, fill: Option<Paint>, stroke: Option<Stroke>) -> Self {
        Self::single(ellipse_path(center, radii), fill, stroke)
    }
}

pub fn rect_path(origin: Vec2, size: Vec2) -> Vec<PathCmd> {
    let (x0, y0, x1, y1) = (origin.x, origin.y, origin.x + size.x, origin.y + size.y);
    vec![
        PathCmd::MoveTo(Vec2::new(x0, y0)),
        PathCmd::LineTo(Vec2::new(x1, y0)),
        PathCmd::LineTo(Vec2::new(x1, y1)),
        PathCmd::LineTo(Vec2::new(x0, y1)),
        PathCmd::Close,
    ]
}

/// Four cubic arcs, starting at the rightmost point and going clockwise.
pub fn ellipse_path(c: Vec2, r: Vec2) -> Vec<PathCmd> {
    let k = r * KAPPA;
    let p = |x: f32, y: f32| c + Vec2::new(x, y);
    vec![
        PathCmd::MoveTo(p(r.x, 0.0)),
        PathCmd::CubicTo(p(r.x, k.y), p(k.x, r.y), p(0.0, r.y)),
        PathCmd::CubicTo(p(-k.x, r.y), p(-r.x, k.y), p(-r.x, 0.0)),
        PathCmd::CubicTo(p(-r.x, -k.y), p(-k.x, -r.y), p(0.0, -r.y)),
        PathCmd::CubicTo(p(k.x, -r.y), p(r.x, -k.y), p(r.x, 0.0)),
        PathCmd::Close,
    ]
}
