//! Backstage2D renderer: draws an evaluated [`Scene`] with wgpu.
//!
//! Shapes are tessellated with lyon once and cached; each item is drawn with
//! its world transform, opacity, and color transform. Gradients are evaluated
//! per fragment. Colors are authored sRGB and blended in sRGB space (like
//! Flash and CSS). Output is 4× MSAA, resolved into a caller-provided
//! `wgpu::TextureView`. See `docs/rendering.md`.

mod offscreen;
pub mod tessellate;

pub use offscreen::{Error, FALLBACK_ENV, HeadlessGpu, OFFSCREEN_FORMAT, OffscreenTarget, adapter_options};

use backstage_core::{Color, DrawContent, Project, Scene, Shape};
use bytemuck::{Pod, Zeroable};
use glam::{Affine2, Vec2};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// Background around the stage in the editor.
pub const PASTEBOARD: wgpu::Color = wgpu::Color { r: 0.23, g: 0.23, b: 0.25, a: 1.0 };
/// Bars around the stage in the player.
pub const LETTERBOX: wgpu::Color = wgpu::Color::BLACK;
/// Color of the pointer crosshair.
pub const CROSSHAIR: [f32; 4] = [0.95, 0.15, 0.45, 1.0];
/// Multisample count for anti-aliasing.
pub const SAMPLE_COUNT: u32 = 4;
/// Gap between the stage and the viewport edge, in logical pixels.
const STAGE_MARGIN: f32 = 24.0;

/// Everything needed to draw one frame.
pub struct Frame<'a> {
    pub project: &'a Project,
    pub scene: &'a Scene<'a>,
    /// Physical pixels per logical pixel.
    pub scale: f32,
    /// Pointer position in logical pixels, if over the stage section.
    pub pointer: Option<(f32, f32)>,
    pub presentation: Presentation,
    /// The editor's outline view: for each scene item (by index), draw it
    /// as thin outlines in this colour instead. Empty for the player.
    pub outlines: &'a [Option<Color>],
}

/// How the stage is framed in the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presentation {
    /// The editor's stage section: pasteboard, margin, shadow, and the
    /// pointer crosshair; never enlarged past 1 stage unit per logical pixel.
    Editor,
    /// The player: the stage fills the window (keeping its aspect ratio)
    /// between black bars, with no shadow or crosshair.
    Player,
}

/// Where the stage sits in the viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Framing {
    /// Stage coordinates → physical pixels.
    pub view: Affine2,
    /// Stage size in physical pixels.
    pub stage_size: Vec2,
}

/// Fits a `stage` of stage units into a `viewport` of physical pixels,
/// centered.
/// - `Editor`: with a margin, never enlarging beyond 1 stage unit per
///   logical pixel.
/// - `Player`: as large as fits. In pixel-art mode the scale is rounded down
///   to a whole number once the viewport is at least as big as the stage,
///   so stage pixels stay square.
pub fn frame_stage(
    viewport: (u32, u32),
    stage: (f32, f32),
    scale: f32,
    presentation: Presentation,
    pixel_art: bool,
) -> Framing {
    let (vw, vh) = (viewport.0 as f32, viewport.1 as f32);
    let fit = match presentation {
        Presentation::Editor => {
            let margin = STAGE_MARGIN * scale;
            ((vw - 2.0 * margin) / stage.0).min((vh - 2.0 * margin) / stage.1).min(scale)
        }
        Presentation::Player => {
            let fit = (vw / stage.0).min(vh / stage.1);
            if pixel_art && fit >= 1.0 { fit.floor() } else { fit }
        }
    }
    .max(0.01);
    let size = Vec2::new(stage.0, stage.1) * fit;
    let origin = (Vec2::new(vw, vh) - size) / 2.0;
    Framing {
        view: Affine2::from_translation(origin) * Affine2::from_scale(Vec2::splat(fit)),
        stage_size: size,
    }
}

/// Pixel-art mode: moves an item to whole physical pixels. Only the
/// translation changes, so rotation and scale stay exact.
pub fn snap_to_pixels(mut m: Affine2) -> Affine2 {
    m.translation = m.translation.round();
    m
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

/// A rotated solid rectangle in physical pixels (`quads.wgsl`), used for
/// the stage background, its shadow, and the pointer crosshair.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Quad {
    center: [f32; 2],
    half_size: [f32; 2],
    angle: f32,
    color: [f32; 4],
}

/// Per-item instance data (`scene.wgsl` locations 2..=7). Field order must
/// match the attribute list: `vertex_attr_array!` packs attributes back to
/// back, so any padding must come last.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ItemInstance {
    m0: [f32; 2],
    m1: [f32; 2],
    m2: [f32; 2],
    multiply: [f32; 4],
    add: [f32; 4],
    opacity: f32,
    _pad: f32,
}

const ITEM_ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    2 => Float32x2, 3 => Float32x2, 4 => Float32x2, 5 => Float32x4, 6 => Float32x4, 7 => Float32
];

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    paints: wgpu::BindGroup,
}

type MeshKey = (usize, Option<[u8; 4]>);

pub struct Renderer {
    format: wgpu::TextureFormat,
    quad_pipeline: wgpu::RenderPipeline,
    scene_pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    paints_layout: wgpu::BindGroupLayout,
    quads: GrowBuffer,
    items: GrowBuffer,
    /// Tessellated shapes, keyed by the shape's address in `cache_owner`
    /// and its outline colour, if drawn as an outline.
    meshes: HashMap<MeshKey, GpuMesh>,
    cache_owner: usize,
    msaa: Option<((u32, u32), wgpu::TextureView)>,
}

impl Renderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        let paints_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("paints"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let target = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let multisample = wgpu::MultisampleState { count: SAMPLE_COUNT, ..Default::default() };

        let quad_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quads"),
            source: wgpu::ShaderSource::Wgsl(include_str!("quads.wgsl").into()),
        });
        let quad_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quads"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });
        let quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quads"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &quad_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Quad>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32, 3 => Float32x4
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &quad_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            multiview_mask: None,
            cache: None,
        });

        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&globals_layout), Some(&paints_layout)],
            immediate_size: 0,
        });
        let scene_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &scene_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<tessellate::Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Uint32],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<ItemInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &ITEM_ATTRIBUTES,
                    }),
                ],
            },
            // Tessellated winding isn't guaranteed; never cull.
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &scene_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            format,
            quad_pipeline,
            scene_pipeline,
            globals,
            globals_bind_group,
            paints_layout,
            quads: GrowBuffer::new("quad instances"),
            items: GrowBuffer::new("item instances"),
            meshes: HashMap::new(),
            cache_owner: 0,
            msaa: None,
        }
    }

    /// Forgets all cached meshes. Call when the project is replaced or its
    /// shapes are edited.
    pub fn clear_cache(&mut self) {
        self.meshes.clear();
    }

    fn mesh_for(&mut self, device: &wgpu::Device, key: MeshKey, shape: &Shape) -> Option<&GpuMesh> {
        if !self.meshes.contains_key(&key) {
            let mesh = match key.1 {
                Some([r, g, b, a]) => tessellate::tessellate_outline(shape, Color::rgba8(r, g, b, a)),
                None => tessellate::tessellate(shape),
            };
            if mesh.indices.is_empty() || mesh.paints.is_empty() {
                return None;
            }
            let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh vertices"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh indices"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
            let paint_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh paints"),
                contents: bytemuck::cast_slice(&mesh.paints),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let paints = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mesh paints"),
                layout: &self.paints_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: paint_buffer.as_entire_binding() }],
            });
            let gpu = GpuMesh { vertices, indices, index_count: mesh.indices.len() as u32, paints };
            self.meshes.insert(key, gpu);
        }
        self.meshes.get(&key)
    }

    fn msaa_view(&mut self, device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
        if self.msaa.as_ref().is_none_or(|(s, _)| *s != size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("msaa target"),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: SAMPLE_COUNT,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.msaa = Some((size, texture.create_view(&Default::default())));
        }
        self.msaa.as_ref().unwrap().1.clone()
    }

    /// Records `frame` into `encoder`, resolving into `view` (`size` physical
    /// pixels, single-sampled, in the renderer's format).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: (u32, u32),
        frame: &Frame,
    ) {
        // Meshes are keyed by shape address, which is only meaningful for
        // the project that owns them.
        let owner = frame.project as *const Project as usize;
        if owner != self.cache_owner {
            self.clear_cache();
            self.cache_owner = owner;
        }
        let settings = &frame.project.settings;
        let framing = frame_stage(
            size,
            (settings.stage_width as f32, settings.stage_height as f32),
            frame.scale,
            frame.presentation,
            settings.pixel_art,
        );

        // Background quads (shadow, stage), then items, then the crosshair.
        let (quads, under) = overlay_quads(&framing, frame, settings.background);
        let mut instances = Vec::with_capacity(frame.scene.items.len());
        let mut draws = Vec::with_capacity(frame.scene.items.len());
        for (i, item) in frame.scene.items.iter().enumerate() {
            let DrawContent::Shape(shape) = item.content else { continue }; // Bitmaps: not yet.
            let outline = frame.outlines.get(i).copied().flatten().map(Color::to_rgba8);
            let key = (shape as *const Shape as usize, outline);
            if self.mesh_for(device, key, shape).is_none() {
                continue;
            }
            let mut m = framing.view * item.transform;
            if settings.pixel_art {
                m = snap_to_pixels(m);
            }
            draws.push((key, instances.len() as u32));
            instances.push(ItemInstance {
                m0: m.matrix2.x_axis.to_array(),
                m1: m.matrix2.y_axis.to_array(),
                m2: m.translation.to_array(),
                multiply: item.color.multiply,
                add: item.color.add,
                opacity: item.opacity,
                _pad: 0.0,
            });
        }

        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals { viewport: [size.0 as f32, size.1 as f32], _pad: [0.0; 2] }),
        );
        self.quads.write(device, queue, bytemuck::cast_slice(&quads), wgpu::BufferUsages::VERTEX);
        self.items.write(device, queue, bytemuck::cast_slice(&instances), wgpu::BufferUsages::VERTEX);
        let msaa = self.msaa_view(device, size);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("stage"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &msaa,
                depth_slice: None,
                resolve_target: Some(view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(match frame.presentation {
                        Presentation::Editor => PASTEBOARD,
                        Presentation::Player => LETTERBOX,
                    }),
                    store: wgpu::StoreOp::Discard,
                },
            })],
            ..Default::default()
        });
        pass.set_bind_group(0, &self.globals_bind_group, &[]);

        let quad_buffer = self.quads.buffer();
        pass.set_pipeline(&self.quad_pipeline);
        if let Some(buffer) = quad_buffer {
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..6, 0..under);
        }

        if let Some(item_buffer) = self.items.buffer() {
            pass.set_pipeline(&self.scene_pipeline);
            pass.set_vertex_buffer(1, item_buffer.slice(..));
            for (key, instance) in &draws {
                let mesh = &self.meshes[key];
                pass.set_bind_group(1, &mesh.paints, &[]);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, *instance..*instance + 1);
            }
        }

        if let Some(buffer) = quad_buffer
            && quads.len() as u32 > under
        {
            pass.set_pipeline(&self.quad_pipeline);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..6, under..quads.len() as u32);
        }
    }
}

/// The stage's background (and, in the editor, its shadow), drawn under the
/// items, followed by the editor's pointer crosshair, drawn over them.
/// Returns the quads and how many go under.
fn overlay_quads(framing: &Framing, frame: &Frame, background: backstage_core::Color) -> (Vec<Quad>, u32) {
    let scale = frame.scale;
    let editor = frame.presentation == Presentation::Editor;
    let center = framing.view.translation + framing.stage_size / 2.0;
    let half = (framing.stage_size / 2.0).to_array();
    let mut quads = Vec::new();
    if editor {
        quads.push(Quad {
            center: (center + Vec2::splat(4.0 * scale)).to_array(),
            half_size: half,
            angle: 0.0,
            color: [0.0, 0.0, 0.0, 0.35],
        });
    }
    quads.push(Quad {
        center: center.to_array(),
        half_size: half,
        angle: 0.0,
        color: [background.r, background.g, background.b, background.a],
    });
    let under = quads.len() as u32;
    if let Some((px, py)) = frame.pointer.filter(|_| editor) {
        // Snap to pixel centers so 1px lines cover whole pixels (crisp).
        let (px, py) = ((px * scale).floor() + 0.5, (py * scale).floor() + 0.5);
        let arm = 14.0 * scale;
        let thick = scale.max(1.0);
        quads.push(Quad { center: [px, py], half_size: [arm, thick / 2.0], angle: 0.0, color: CROSSHAIR });
        quads.push(Quad { center: [px, py], half_size: [thick / 2.0, arm], angle: 0.0, color: CROSSHAIR });
    }
    (quads, under)
}

/// A GPU buffer that grows (never shrinks) to fit what's written.
struct GrowBuffer {
    label: &'static str,
    buffer: Option<wgpu::Buffer>,
    used: u64,
}

impl GrowBuffer {
    fn new(label: &'static str) -> Self {
        Self { label, buffer: None, used: 0 }
    }

    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8], usage: wgpu::BufferUsages) {
        self.used = bytes.len() as u64;
        if bytes.is_empty() {
            return;
        }
        if self.buffer.as_ref().is_none_or(|b| b.size() < self.used) {
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: self.used.next_power_of_two().max(256),
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        queue.write_buffer(self.buffer.as_ref().unwrap(), 0, bytes);
    }

    fn buffer(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref().filter(|_| self.used > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_fits_inside_viewport_with_margin() {
        for (size, scale) in [((800, 600), 1.0), ((300, 900), 2.0), ((1920, 400), 1.5)] {
            let f = frame_stage(size, (550.0, 400.0), scale, Presentation::Editor, false);
            let (lo, hi) = (f.view.translation, f.view.translation + f.stage_size);
            let margin = STAGE_MARGIN * scale - 0.01;
            assert!(lo.x >= margin && hi.x <= size.0 as f32 - margin, "{size:?}");
            assert!(lo.y >= margin && hi.y <= size.1 as f32 - margin, "{size:?}");
        }
    }

    #[test]
    fn stage_never_upscales_past_scale() {
        let f = frame_stage((8000, 8000), (550.0, 400.0), 1.25, Presentation::Editor, false);
        assert!((f.stage_size - Vec2::new(550.0, 400.0) * 1.25).length() < 0.01);
        assert_eq!(f.view.transform_point2(Vec2::ZERO), f.view.translation);
    }

    #[test]
    fn degenerate_viewports_stay_finite() {
        for size in [(0, 0), (1, 1), (10, 5000)] {
            let f = frame_stage(size, (550.0, 400.0), 1.0, Presentation::Editor, false);
            assert!(f.view.translation.is_finite() && f.stage_size.is_finite(), "{size:?}");
        }
    }

    #[test]
    fn item_attributes_match_the_struct_layout() {
        let offsets: Vec<u64> = ITEM_ATTRIBUTES.iter().map(|a| a.offset).collect();
        let s = ItemInstance::zeroed();
        let base = &s as *const _ as usize;
        let field = |p: *const f32| (p as usize - base) as u64;
        let expected = [
            field(s.m0.as_ptr()),
            field(s.m1.as_ptr()),
            field(s.m2.as_ptr()),
            field(s.multiply.as_ptr()),
            field(s.add.as_ptr()),
            field(&s.opacity),
        ];
        assert_eq!(offsets, expected);
    }

    #[test]
    fn player_fills_the_window_keeping_aspect() {
        // Wider window: bars left and right.
        let f = frame_stage((1100, 400), (550.0, 400.0), 1.0, Presentation::Player, false);
        assert_eq!(f.stage_size, Vec2::new(550.0, 400.0));
        assert_eq!(f.view.translation, Vec2::new(275.0, 0.0));
        // Taller window, enlarged: bars top and bottom.
        let f = frame_stage((1100, 1600), (550.0, 400.0), 1.0, Presentation::Player, false);
        assert_eq!(f.stage_size, Vec2::new(1100.0, 800.0));
        assert_eq!(f.view.translation, Vec2::new(0.0, 400.0));
    }

    #[test]
    fn pixel_art_player_uses_whole_number_scales() {
        let f = frame_stage((1000, 700), (320.0, 180.0), 1.0, Presentation::Player, true);
        assert_eq!(f.stage_size, Vec2::new(960.0, 540.0), "3×, not 3.125×");
        // Smaller than the stage: scale down smoothly rather than to 0×.
        let f = frame_stage((160, 90), (320.0, 180.0), 1.0, Presentation::Player, true);
        assert_eq!(f.stage_size, Vec2::new(160.0, 90.0));
    }

    #[test]
    fn pixel_snapping_rounds_translation_only() {
        let m = Affine2::from_scale_angle_translation(Vec2::splat(1.5), 0.3, Vec2::new(10.4, 7.6));
        let s = snap_to_pixels(m);
        assert_eq!(s.translation, Vec2::new(10.0, 8.0));
        assert_eq!(s.matrix2, m.matrix2);
    }
}
