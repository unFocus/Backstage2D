//! Free-transform handles around the selection (ADR 0006): where they are,
//! which one the pointer is on, and the cursor for it. Pure geometry: the
//! stage draws them and turns grips into drags.
//!
//! One selected node gets a box in its own space, so it turns and skews
//! with the node: 8 scale handles (corners and edge middles), rotate zones
//! just outside the corners, and its pivot. Several selected nodes share
//! one stage-aligned box with corner handles only (they scale evenly) and
//! its centre as the pivot.

use backstage_core::Vec2;
use backstage_protocol::CursorKind;
use backstage_render::pick::Rect;
use backstage_render::{HANDLE_HALF, HandleKind, Overlay, SELECTION};
use glam::Affine2;

/// How far past a handle's edge a press still grabs it, in logical pixels.
const GRAB_SLOP: f32 = 2.0;
/// How far out from a corner a press rotates, in logical pixels.
const ROTATE_REACH: f32 = 16.0;

/// The handles of one selected node.
#[derive(Debug, Clone, PartialEq)]
pub struct Handles {
    /// The node's box in its own (shape) space.
    pub local: Rect,
    /// The node's own space to the stage.
    pub world: Affine2,
    /// The node's pivot (registration point), in its own space.
    pub pivot: Vec2,
    /// Several nodes' shared box: corner handles only.
    pub corners_only: bool,
}

/// What a press on the handles grabs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Grip {
    /// Scale handle `index` (0..8, clockwise from the top-left corner).
    /// `handle` and `anchor` (the opposite handle) are in the node's space.
    Scale { index: usize, handle: Vec2, anchor: Vec2, corner: bool },
    /// Just outside a corner.
    Rotate,
}

impl Handles {
    /// The 8 handle points in the node's space, clockwise from the top-left
    /// corner: corners at even indices, edge middles at odd ones. Handle
    /// `i`'s opposite is `(i + 4) % 8`.
    pub fn points(&self) -> [Vec2; 8] {
        let c = self.local.corners();
        std::array::from_fn(|i| {
            if i.is_multiple_of(2) { c[i / 2] } else { (c[i / 2] + c[(i / 2 + 1) % 4]) / 2.0 }
        })
    }

    /// What's under screen point `p`, if anything. `view` maps stage
    /// coordinates to the screen; `px` is screen pixels per logical pixel.
    pub fn grip_at(&self, p: Vec2, view: Affine2, px: f32) -> Option<Grip> {
        let to_screen = view * self.world;
        let points = self.points();
        let reach = (HANDLE_HALF + GRAB_SLOP) * px;
        for (index, &handle) in points.iter().enumerate() {
            if self.corners_only && !index.is_multiple_of(2) {
                continue;
            }
            let d = (to_screen.transform_point2(handle) - p).abs();
            if d.x <= reach && d.y <= reach {
                let anchor = points[(index + 4) % 8];
                return Some(Grip::Scale { index, handle, anchor, corner: index.is_multiple_of(2) });
            }
        }
        // Outside the box, near a corner.
        if to_screen.matrix2.determinant().abs() < 1e-9 {
            return None;
        }
        let local = to_screen.inverse().transform_point2(p);
        let inside = local.cmpge(self.local.min).all() && local.cmple(self.local.max).all();
        let near_corner =
            points.iter().step_by(2).any(|&c| to_screen.transform_point2(c).distance(p) <= ROTATE_REACH * px);
        (!inside && near_corner).then_some(Grip::Rotate)
    }

    /// The cursor for `grip`: a resize arrow along the handle's direction
    /// on screen (so it turns with the node), or the rotate cursor.
    pub fn cursor(&self, grip: Grip, view: Affine2) -> CursorKind {
        let Grip::Scale { handle, .. } = grip else { return CursorKind::Rotate };
        let to_screen = view * self.world;
        let centre = (self.local.min + self.local.max) / 2.0;
        let d = to_screen.transform_point2(handle) - to_screen.transform_point2(centre);
        // The direction's angle, folded to 0..180 and rounded to 45°.
        let degrees = d.y.atan2(d.x).to_degrees().rem_euclid(180.0);
        match ((degrees / 45.0).round() as i32) % 4 {
            0 => CursorKind::ResizeEw,
            1 => CursorKind::ResizeNwse,
            2 => CursorKind::ResizeNs,
            _ => CursorKind::ResizeNesw,
        }
    }

    /// The box, its handles, and the pivot, to draw.
    pub fn overlay(&self) -> Vec<Overlay> {
        let corners = self.local.corners().map(|c| self.world.transform_point2(c));
        let mut marks = vec![Overlay::Outline { corners, color: SELECTION }];
        let step = if self.corners_only { 2 } else { 1 };
        marks.extend(
            self.points()
                .iter()
                .step_by(step)
                .map(|&p| Overlay::Handle { at: self.world.transform_point2(p), kind: HandleKind::Square }),
        );
        marks.push(Overlay::Handle { at: self.world.transform_point2(self.pivot), kind: HandleKind::Pivot });
        marks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 100 × 50 box at (200, 100) on the stage, seen at 1:1.
    fn plain() -> Handles {
        Handles {
            local: Rect { min: Vec2::ZERO, max: Vec2::new(100.0, 50.0) },
            world: Affine2::from_translation(Vec2::new(200.0, 100.0)),
            pivot: Vec2::ZERO,
            corners_only: false,
        }
    }

    fn grip(h: &Handles, x: f32, y: f32) -> Option<Grip> {
        h.grip_at(Vec2::new(x, y), Affine2::IDENTITY, 1.0)
    }

    #[test]
    fn handles_sit_on_the_corners_and_edge_middles_with_opposites() {
        let points = plain().points();
        assert_eq!(points[0], Vec2::ZERO);
        assert_eq!(points[1], Vec2::new(50.0, 0.0));
        assert_eq!(points[4], Vec2::new(100.0, 50.0));
        assert_eq!(points[7], Vec2::new(0.0, 25.0));
        let Some(Grip::Scale { index: 4, anchor, corner: true, .. }) = grip(&plain(), 301.0, 152.0) else {
            panic!("the bottom-right corner")
        };
        assert_eq!(anchor, Vec2::ZERO, "anchored at the top-left");
        assert!(matches!(grip(&plain(), 250.0, 103.0), Some(Grip::Scale { index: 1, corner: false, .. })));
    }

    #[test]
    fn just_outside_a_corner_rotates_and_elsewhere_grabs_nothing() {
        let h = plain();
        assert_eq!(grip(&h, 310.0, 160.0), Some(Grip::Rotate), "past the bottom-right corner");
        assert_eq!(grip(&h, 190.0, 92.0), Some(Grip::Rotate), "past the top-left corner");
        assert_eq!(grip(&h, 250.0, 125.0), None, "inside: the node's own (a move)");
        assert_eq!(grip(&h, 250.0, 80.0), None, "far from any corner");
        assert_eq!(grip(&h, 330.0, 175.0), None, "too far out");
    }

    #[test]
    fn resize_cursors_follow_the_handle_on_screen() {
        let h = plain();
        let cursor = |i: usize| {
            let p = h.points()[i];
            h.cursor(
                Grip::Scale { index: i, handle: p, anchor: p, corner: i.is_multiple_of(2) },
                Affine2::IDENTITY,
            )
        };
        // A 2:1 box: its corners are about 27° off horizontal, which rounds
        // to the diagonals.
        assert_eq!(cursor(0), CursorKind::ResizeNwse);
        assert_eq!(cursor(1), CursorKind::ResizeNs);
        assert_eq!(cursor(2), CursorKind::ResizeNesw);
        assert_eq!(cursor(3), CursorKind::ResizeEw);
        assert_eq!(h.cursor(Grip::Rotate, Affine2::IDENTITY), CursorKind::Rotate);

        // Turned 90°: the top edge's handle now points sideways.
        let turned = Handles { world: h.world * Affine2::from_angle(90f32.to_radians()), ..h };
        let top = turned.points()[1];
        let grip = Grip::Scale { index: 1, handle: top, anchor: top, corner: false };
        assert_eq!(turned.cursor(grip, Affine2::IDENTITY), CursorKind::ResizeEw);
    }

    #[test]
    fn a_shared_box_has_corner_handles_only() {
        let shared = Handles { corners_only: true, ..plain() };
        assert_eq!(grip(&shared, 250.0, 100.0), None, "no edge handles");
        assert!(matches!(grip(&shared, 300.0, 150.0), Some(Grip::Scale { index: 4, corner: true, .. })));
        assert_eq!(shared.overlay().len(), 6, "the box, 4 corners, and the pivot");
    }

    #[test]
    fn the_overlay_is_a_box_eight_handles_and_the_pivot() {
        let marks = plain().overlay();
        assert_eq!(marks.len(), 10);
        assert!(matches!(marks[0], Overlay::Outline { .. }));
        assert!(
            matches!(marks[9], Overlay::Handle { kind: HandleKind::Pivot, at } if at == Vec2::new(200.0, 100.0))
        );
    }
}
