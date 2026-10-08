//! Shapes → triangle meshes (lyon), plus the paints their vertices refer to.

use backstage_core::{Color, LineCap, LineJoin, Paint, PathCmd, Shape};
use bytemuck::{Pod, Zeroable};
use lyon::math::point;
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator, StrokeVertex,
    VertexBuffers,
};

/// Flattening tolerance in local units: curves stay within this distance
/// of the true outline.
pub const TOLERANCE: f32 = 0.05;
/// Gradient stops beyond this are dropped.
pub const MAX_STOPS: usize = 8;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    /// Local (shape) coordinates.
    pub pos: [f32; 2],
    /// Index into the shape's paints.
    pub paint: u32,
}

/// A paint as the shader sees it (`scene.wgsl`'s `Paint`). 192 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GpuPaint {
    /// 0 solid, 1 linear gradient, 2 radial gradient.
    pub kind: u32,
    pub stop_count: u32,
    pub radius: f32,
    pub _pad: f32,
    /// Linear: start. Radial: center.
    pub p0: [f32; 2],
    /// Linear: end.
    pub p1: [f32; 2],
    /// Straight-alpha sRGB colors.
    pub stops: [[f32; 4]; MAX_STOPS],
    /// Stop offsets, four per vec4.
    pub offsets: [[f32; 4]; MAX_STOPS / 4],
}

#[derive(Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub paints: Vec<GpuPaint>,
}

pub fn gpu_paint(paint: &Paint) -> GpuPaint {
    let mut gpu = GpuPaint::zeroed();
    let rgba = |c: backstage_core::Color| [c.r, c.g, c.b, c.a];
    let stops = match paint {
        Paint::Solid(color) => {
            gpu.stops[0] = rgba(*color);
            gpu.stop_count = 1;
            return gpu;
        }
        Paint::LinearGradient { start, end, stops } => {
            gpu.kind = 1;
            gpu.p0 = start.to_array();
            gpu.p1 = end.to_array();
            stops
        }
        Paint::RadialGradient { center, radius, stops } => {
            gpu.kind = 2;
            gpu.p0 = center.to_array();
            gpu.radius = *radius;
            stops
        }
    };
    if stops.len() > MAX_STOPS {
        eprintln!("backstage_render: gradient has {} stops; using the first {MAX_STOPS}", stops.len());
    }
    for (i, stop) in stops.iter().take(MAX_STOPS).enumerate() {
        gpu.stops[i] = rgba(stop.color);
        gpu.offsets[i / 4][i % 4] = stop.offset;
    }
    gpu.stop_count = stops.len().min(MAX_STOPS) as u32;
    gpu
}

pub(crate) fn lyon_path(cmds: &[PathCmd]) -> Path {
    let mut builder = Path::builder();
    let mut open = false;
    let p = |v: backstage_core::Vec2| point(v.x, v.y);
    for cmd in cmds {
        match *cmd {
            PathCmd::MoveTo(to) => {
                if open {
                    builder.end(false);
                }
                builder.begin(p(to));
                open = true;
            }
            PathCmd::LineTo(to) if open => {
                builder.line_to(p(to));
            }
            PathCmd::QuadTo(ctrl, to) if open => {
                builder.quadratic_bezier_to(p(ctrl), p(to));
            }
            PathCmd::CubicTo(c1, c2, to) if open => {
                builder.cubic_bezier_to(p(c1), p(c2), p(to));
            }
            PathCmd::Close if open => {
                builder.end(true);
                open = false;
            }
            _ => {} // Drawing commands before any MoveTo are ignored.
        }
    }
    if open {
        builder.end(false);
    }
    builder.build()
}

/// Tessellates every styled path of `shape` (fill, then stroke) into one mesh.
pub fn tessellate(shape: &Shape) -> Mesh {
    let mut buffers: VertexBuffers<Vertex, u32> = VertexBuffers::new();
    let mut paints = Vec::new();
    let mut fill = FillTessellator::new();
    let mut stroke = StrokeTessellator::new();
    for styled in &shape.paths {
        let path = lyon_path(&styled.path);
        if let Some(paint) = &styled.fill {
            let index = paints.len() as u32;
            paints.push(gpu_paint(paint));
            let options =
                FillOptions::tolerance(TOLERANCE).with_fill_rule(lyon::tessellation::FillRule::NonZero);
            let result = fill.tessellate_path(
                &path,
                &options,
                &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| Vertex {
                    pos: v.position().to_array(),
                    paint: index,
                }),
            );
            if let Err(e) = result {
                eprintln!("backstage_render: fill tessellation failed: {e:?}");
            }
        }
        if let Some(s) = &styled.stroke {
            let index = paints.len() as u32;
            paints.push(gpu_paint(&s.paint));
            let cap = match s.cap {
                LineCap::Round => lyon::tessellation::LineCap::Round,
                LineCap::Butt => lyon::tessellation::LineCap::Butt,
                LineCap::Square => lyon::tessellation::LineCap::Square,
            };
            let join = match s.join {
                LineJoin::Round => lyon::tessellation::LineJoin::Round,
                LineJoin::Miter => lyon::tessellation::LineJoin::Miter,
                LineJoin::Bevel => lyon::tessellation::LineJoin::Bevel,
            };
            let options = StrokeOptions::tolerance(TOLERANCE)
                .with_line_width(s.width)
                .with_line_cap(cap)
                .with_line_join(join)
                .with_miter_limit(s.miter_limit.max(StrokeOptions::MINIMUM_MITER_LIMIT));
            let result = stroke.tessellate_path(
                &path,
                &options,
                &mut BuffersBuilder::new(&mut buffers, |v: StrokeVertex| Vertex {
                    pos: v.position().to_array(),
                    paint: index,
                }),
            );
            if let Err(e) = result {
                eprintln!("backstage_render: stroke tessellation failed: {e:?}");
            }
        }
    }
    Mesh { vertices: buffers.vertices, indices: buffers.indices, paints }
}

/// Width of editor outlines, in the shape's own units.
pub const OUTLINE_WIDTH: f32 = 1.0;

/// The editor's outline view of `shape`: every path stroked thinly in
/// `color`, with no fills or strokes of its own.
pub fn tessellate_outline(shape: &Shape, color: Color) -> Mesh {
    let mut buffers: VertexBuffers<Vertex, u32> = VertexBuffers::new();
    let mut stroke = StrokeTessellator::new();
    let options = StrokeOptions::tolerance(TOLERANCE).with_line_width(OUTLINE_WIDTH);
    for styled in &shape.paths {
        let result = stroke.tessellate_path(
            &lyon_path(&styled.path),
            &options,
            &mut BuffersBuilder::new(&mut buffers, |v: StrokeVertex| Vertex {
                pos: v.position().to_array(),
                paint: 0,
            }),
        );
        if let Err(e) = result {
            eprintln!("backstage_render: outline tessellation failed: {e:?}");
        }
    }
    Mesh {
        vertices: buffers.vertices,
        indices: buffers.indices,
        paints: vec![gpu_paint(&Paint::Solid(color))],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backstage_core::{Color, GradientStop, Stroke, Vec2};

    fn bounds(mesh: &Mesh) -> (Vec2, Vec2) {
        mesh.vertices.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), v| {
            let p = Vec2::from(v.pos);
            (lo.min(p), hi.max(p))
        })
    }

    #[test]
    fn outlines_use_one_paint_and_only_thin_strokes() {
        let shape = Shape::rect(
            Vec2::ZERO,
            Vec2::new(10.0, 10.0),
            Some(Paint::Solid(Color::BLACK)),
            Some(Stroke::solid(Color::BLACK, 6.0)),
        );
        let mesh = tessellate_outline(&shape, Color::rgb8(255, 0, 0));
        assert_eq!(mesh.paints.len(), 1);
        assert!(!mesh.indices.is_empty());
        let (min, max) = mesh
            .vertices
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.pos[0]), hi.max(v.pos[0])));
        assert!(
            min >= -OUTLINE_WIDTH && max <= 10.0 + OUTLINE_WIDTH,
            "not the shape's own wide stroke: {min}..{max}"
        );
    }

    #[test]
    fn rect_is_two_triangles_covering_its_bounds() {
        let shape =
            Shape::rect(Vec2::new(10.0, 20.0), Vec2::new(30.0, 40.0), Some(Paint::Solid(Color::WHITE)), None);
        let mesh = tessellate(&shape);
        assert_eq!(mesh.indices.len(), 6);
        assert_eq!(bounds(&mesh), (Vec2::new(10.0, 20.0), Vec2::new(40.0, 60.0)));
        assert_eq!(mesh.paints.len(), 1);
        assert!(mesh.vertices.iter().all(|v| v.paint == 0));
    }

    #[test]
    fn ellipse_bounds_match_radii() {
        let shape = Shape::ellipse(
            Vec2::new(5.0, 5.0),
            Vec2::new(20.0, 10.0),
            Some(Paint::Solid(Color::BLACK)),
            None,
        );
        let (lo, hi) = bounds(&tessellate(&shape));
        assert!((lo - Vec2::new(-15.0, -5.0)).abs().max_element() < 0.1, "{lo}");
        assert!((hi - Vec2::new(25.0, 15.0)).abs().max_element() < 0.1, "{hi}");
    }

    #[test]
    fn stroke_is_wider_than_the_path_and_uses_its_own_paint() {
        let shape = Shape::rect(
            Vec2::ZERO,
            Vec2::splat(10.0),
            Some(Paint::Solid(Color::WHITE)),
            Some(Stroke::solid(Color::BLACK, 4.0)),
        );
        let mesh = tessellate(&shape);
        assert_eq!(mesh.paints.len(), 2);
        let stroke: Vec<_> = mesh.vertices.iter().filter(|v| v.paint == 1).collect();
        assert!(!stroke.is_empty());
        let min = stroke.iter().map(|v| v.pos[0]).fold(f32::MAX, f32::min);
        assert!((min + 2.0).abs() < 0.01, "stroke extends half its width outside: {min}");
        assert!(mesh.indices.iter().all(|&i| (i as usize) < mesh.vertices.len()));
    }

    #[test]
    fn paints_convert() {
        let solid = gpu_paint(&Paint::Solid(Color::rgb8(255, 0, 0)));
        assert_eq!((solid.kind, solid.stop_count, solid.stops[0]), (0, 1, [1.0, 0.0, 0.0, 1.0]));

        let stops: Vec<_> =
            (0..10).map(|i| GradientStop { offset: i as f32 / 9.0, color: Color::WHITE }).collect();
        let radial = gpu_paint(&Paint::RadialGradient { center: Vec2::new(1.0, 2.0), radius: 3.0, stops });
        assert_eq!((radial.kind, radial.stop_count, radial.p0, radial.radius), (2, 8, [1.0, 2.0], 3.0));
        assert_eq!(radial.offsets[1][3], 7.0 / 9.0);

        let linear = gpu_paint(&Paint::LinearGradient {
            start: Vec2::ZERO,
            end: Vec2::X,
            stops: vec![
                GradientStop { offset: 0.0, color: Color::BLACK },
                GradientStop { offset: 1.0, color: Color::WHITE },
            ],
        });
        assert_eq!((linear.kind, linear.stop_count, linear.p1), (1, 2, [1.0, 0.0]));
        assert_eq!(size_of::<GpuPaint>(), 192, "must match scene.wgsl's Paint layout");
    }

    #[test]
    fn commands_before_move_to_are_ignored() {
        let shape = Shape::single(
            vec![
                PathCmd::LineTo(Vec2::ONE),
                PathCmd::Close,
                PathCmd::MoveTo(Vec2::ZERO),
                PathCmd::LineTo(Vec2::X),
                PathCmd::LineTo(Vec2::ONE),
            ],
            Some(Paint::Solid(Color::WHITE)),
            None,
        );
        assert_eq!(tessellate(&shape).indices.len(), 3);
    }
}
