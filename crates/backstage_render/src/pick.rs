//! Hit testing and bounds for scene items, on the CPU with the same lyon
//! paths the renderer tessellates, so what you see is what you hit. The
//! editing stage uses these for picking and selection boxes.

use crate::tessellate::{TOLERANCE, lyon_path};
use backstage_core::{DrawContent, DrawItem, Shape};
use glam::Vec2;
use lyon::algorithms::aabb::bounding_box;
use lyon::algorithms::hit_test::hit_test_path;
use lyon::math::point;
use lyon::path::iterator::PathIterator;
use lyon::path::{FillRule, PathEvent};

/// An axis-aligned rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    /// The smallest rectangle holding every point, or `None` for none.
    pub fn around(points: impl IntoIterator<Item = Vec2>) -> Option<Rect> {
        let mut points = points.into_iter();
        let first = points.next()?;
        Some(
            points
                .fold(Rect { min: first, max: first }, |r, p| Rect { min: r.min.min(p), max: r.max.max(p) }),
        )
    }

    pub fn union(self, other: Rect) -> Rect {
        Rect { min: self.min.min(other.min), max: self.max.max(other.max) }
    }

    fn corners(self) -> [Vec2; 4] {
        [self.min, Vec2::new(self.max.x, self.min.y), self.max, Vec2::new(self.min.x, self.max.y)]
    }
}

/// Whether stage point `p` hits `item`: inside a filled path, or within
/// half a stroke's width (plus `slop`, in stage units) of a stroked one.
/// Bitmaps aren't drawn yet, so they never hit.
pub fn hit_item(item: &DrawItem, p: Vec2, slop: f32) -> bool {
    let DrawContent::Shape(shape) = item.content else { return false };
    let det = item.transform.matrix2.determinant();
    if det.abs() < 1e-12 {
        return false; // Scaled to nothing.
    }
    let local = item.transform.inverse().transform_point2(p);
    // Slop is in stage units; scale it into local ones (an average for a
    // non-uniform scale).
    let local_slop = slop / det.abs().sqrt();
    hit_shape(shape, local, local_slop)
}

fn hit_shape(shape: &Shape, p: Vec2, slop: f32) -> bool {
    let at = point(p.x, p.y);
    shape.paths.iter().any(|styled| {
        let path = lyon_path(&styled.path);
        if styled.fill.is_some() && hit_test_path(&at, path.iter(), FillRule::NonZero, TOLERANCE) {
            return true;
        }
        styled.stroke.as_ref().is_some_and(|s| distance_to(&path, p) <= s.width / 2.0 + slop)
    })
}

/// Distance from `p` to the nearest point on `path`'s outline.
fn distance_to(path: &lyon::path::Path, p: Vec2) -> f32 {
    let v = |q: lyon::math::Point| Vec2::new(q.x, q.y);
    let mut best = f32::INFINITY;
    for event in path.iter().flattened(TOLERANCE) {
        let (from, to) = match event {
            PathEvent::Line { from, to } => (from, to),
            PathEvent::End { last, first, close: true } => (last, first),
            _ => continue,
        };
        best = best.min(segment_distance(p, v(from), v(to)));
    }
    best
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// `item`'s bounds on the stage: its shape's tight local bounds, grown by
/// half the widest stroke, then transformed. `None` for an empty shape or
/// a bitmap.
pub fn item_bounds(item: &DrawItem) -> Option<Rect> {
    let DrawContent::Shape(shape) = item.content else { return None };
    let local = shape_bounds(shape)?;
    Rect::around(local.corners().map(|c| item.transform.transform_point2(c)))
}

fn shape_bounds(shape: &Shape) -> Option<Rect> {
    shape
        .paths
        .iter()
        .filter(|s| s.path.len() > 1)
        .map(|styled| {
            let b = bounding_box(lyon_path(&styled.path).iter());
            let grow = styled.stroke.as_ref().map_or(0.0, |s| s.width / 2.0);
            Rect {
                min: Vec2::new(b.min.x, b.min.y) - Vec2::splat(grow),
                max: Vec2::new(b.max.x, b.max.y) + Vec2::splat(grow),
            }
        })
        .reduce(Rect::union)
}

#[cfg(test)]
mod tests {
    use super::*;
    use backstage_core::{BlendMode, Color, ColorTransform, NodeId, Paint, Stroke};
    use glam::Affine2;

    fn item(shape: &Shape, transform: Affine2) -> DrawItem<'_> {
        DrawItem {
            instance: Vec::new(),
            node: NodeId::from_raw(1),
            transform,
            opacity: 1.0,
            color: ColorTransform::default(),
            blend: BlendMode::default(),
            content: DrawContent::Shape(shape),
        }
    }

    fn red() -> Option<Paint> {
        Some(Paint::Solid(Color::rgb8(255, 0, 0)))
    }

    #[test]
    fn fills_hit_inside_their_outline_only() {
        let disc = Shape::ellipse(Vec2::ZERO, Vec2::splat(10.0), red(), None);
        let disc = item(&disc, Affine2::from_translation(Vec2::new(100.0, 50.0)));
        assert!(hit_item(&disc, Vec2::new(100.0, 50.0), 0.0), "centre");
        assert!(hit_item(&disc, Vec2::new(109.0, 50.0), 0.0), "just inside the edge");
        assert!(!hit_item(&disc, Vec2::new(111.0, 50.0), 0.0), "just outside");
        assert!(!hit_item(&disc, Vec2::new(108.0, 58.0), 0.0), "inside the bounding box's corner");
    }

    #[test]
    fn strokes_hit_within_half_their_width_plus_slop() {
        let ring =
            Shape::ellipse(Vec2::ZERO, Vec2::splat(10.0), None, Some(Stroke::solid(Color::BLACK, 4.0)));
        let ring = item(&ring, Affine2::IDENTITY);
        assert!(!hit_item(&ring, Vec2::ZERO, 0.0), "an unfilled middle");
        assert!(hit_item(&ring, Vec2::new(11.9, 0.0), 0.0), "on the stroke");
        assert!(!hit_item(&ring, Vec2::new(13.0, 0.0), 0.0), "past it");
        assert!(hit_item(&ring, Vec2::new(13.0, 0.0), 1.5), "within the slop");
    }

    #[test]
    fn transforms_map_the_point_into_the_shape() {
        // 20 × 10 rect at the origin, scaled 2×, turned 90° clockwise,
        // moved to (100, 100): it covers x 80..100, y 100..140.
        let rect = Shape::rect(Vec2::ZERO, Vec2::new(20.0, 10.0), red(), None);
        let m = Affine2::from_translation(Vec2::splat(100.0))
            * Affine2::from_angle(90f32.to_radians())
            * Affine2::from_scale(Vec2::splat(2.0));
        let rect = item(&rect, m);
        assert!(hit_item(&rect, Vec2::new(90.0, 120.0), 0.0));
        assert!(!hit_item(&rect, Vec2::new(110.0, 120.0), 0.0));
        assert!(!hit_item(&rect, Vec2::new(90.0, 145.0), 0.0));

        let b = item_bounds(&rect).unwrap();
        assert!(b.min.abs_diff_eq(Vec2::new(80.0, 100.0), 1e-3), "{b:?}");
        assert!(b.max.abs_diff_eq(Vec2::new(100.0, 140.0), 1e-3), "{b:?}");
    }

    #[test]
    fn bounds_are_tight_and_include_strokes() {
        let disc =
            Shape::ellipse(Vec2::ZERO, Vec2::splat(10.0), red(), Some(Stroke::solid(Color::BLACK, 2.0)));
        let b = item_bounds(&item(&disc, Affine2::IDENTITY)).unwrap();
        // Tight: the curve, not its control points (which reach 10 too
        // here, but would overshoot on a diagonal arc).
        assert!(b.min.abs_diff_eq(Vec2::splat(-11.0), 1e-3), "{b:?}");
        assert!(b.max.abs_diff_eq(Vec2::splat(11.0), 1e-3), "{b:?}");
        assert_eq!(item_bounds(&item(&Shape::default(), Affine2::IDENTITY)), None, "empty");
    }

    #[test]
    fn squashed_flat_items_never_hit() {
        let rect = Shape::rect(Vec2::ZERO, Vec2::splat(10.0), red(), None);
        let flat = item(&rect, Affine2::from_scale(Vec2::new(1.0, 0.0)));
        assert!(!hit_item(&flat, Vec2::new(5.0, 0.0), 1.0));
    }
}
