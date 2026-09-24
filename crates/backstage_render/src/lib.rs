//! Backstage2D renderer: turns a display list into wgpu draw calls.
//!
//! For now it draws a hard-coded test scene (a Flash-style white stage on a
//! grey pasteboard, some rotating quads, and a pointer crosshair). It always
//! renders into a caller-provided `wgpu::TextureView`; see `docs/rendering.md`.

use bytemuck::{Pod, Zeroable};
use std::f32::consts::TAU;

/// Default Flash stage size, in stage units.
pub const STAGE_SIZE: (f32, f32) = (550.0, 400.0);

const PASTEBOARD: wgpu::Color = wgpu::Color { r: 0.23, g: 0.23, b: 0.25, a: 1.0 };

/// Inputs for one frame of the test scene.
pub struct TestScene {
    /// Seconds since the stage started.
    pub time: f32,
    /// Physical pixels per logical pixel.
    pub scale: f32,
    /// Pointer position in logical pixels, if over the stage section.
    pub pointer: Option<(f32, f32)>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

/// One rotated, solid-colored rectangle, in physical pixels.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Quad {
    center: [f32; 2],
    half_size: [f32; 2],
    angle: f32,
    color: [f32; 4],
}

pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    instance_capacity: usize,
}

impl Renderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quads"),
            source: wgpu::ShaderSource::Wgsl(include_str!("quads.wgsl").into()),
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quads"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quads"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
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
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let instance_capacity = 64;
        let instances = Self::instance_buffer(device, instance_capacity);
        Self { pipeline, globals, bind_group, instances, instance_capacity }
    }

    fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad instances"),
            size: (capacity * size_of::<Quad>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Records the test scene into `encoder`, targeting `view` of `size`
    /// physical pixels.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: (u32, u32),
        scene: &TestScene,
    ) {
        let quads = build_test_scene(size, scene);
        if quads.len() > self.instance_capacity {
            self.instance_capacity = quads.len().next_power_of_two();
            self.instances = Self::instance_buffer(device, self.instance_capacity);
        }
        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals { viewport: [size.0 as f32, size.1 as f32], _pad: [0.0; 2] }),
        );
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&quads));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("test scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(PASTEBOARD),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..quads.len() as u32);
    }
}

fn build_test_scene(size: (u32, u32), scene: &TestScene) -> Vec<Quad> {
    let (vw, vh) = (size.0 as f32, size.1 as f32);
    let (sw, sh) = STAGE_SIZE;
    // Fit the stage into the viewport with a margin, never upscaling past 1:1
    // logical pixels.
    let margin = 24.0 * scene.scale;
    let fit = ((vw - 2.0 * margin) / sw).min((vh - 2.0 * margin) / sh).min(scene.scale).max(0.01);
    let origin = [(vw - sw * fit) / 2.0, (vh - sh * fit) / 2.0];
    let to_px = |x: f32, y: f32| [origin[0] + x * fit, origin[1] + y * fit];

    let mut quads = vec![
        // Drop shadow, then the white stage.
        Quad {
            center: [vw / 2.0 + 4.0 * scene.scale, vh / 2.0 + 4.0 * scene.scale],
            half_size: [sw * fit / 2.0, sh * fit / 2.0],
            angle: 0.0,
            color: [0.0, 0.0, 0.0, 0.35],
        },
        Quad {
            center: [vw / 2.0, vh / 2.0],
            half_size: [sw * fit / 2.0, sh * fit / 2.0],
            angle: 0.0,
            color: [1.0, 1.0, 1.0, 1.0],
        },
    ];

    let palette = [
        [0.91, 0.30, 0.24, 1.0],
        [0.95, 0.61, 0.07, 1.0],
        [0.18, 0.80, 0.44, 1.0],
        [0.20, 0.60, 0.86, 1.0],
        [0.61, 0.35, 0.71, 1.0],
    ];
    for (i, color) in palette.iter().enumerate() {
        let phase = i as f32 / palette.len() as f32 * TAU;
        let t = scene.time * 0.6 + phase;
        let (x, y) = (sw / 2.0 + t.cos() * 150.0, sh / 2.0 + (t * 2.0).sin() * 110.0);
        let half = 22.0 + 8.0 * (scene.time * 2.0 + phase).sin();
        quads.push(Quad {
            center: to_px(x, y),
            half_size: [half * fit, half * fit],
            angle: scene.time * (1.0 + i as f32 * 0.3),
            color: *color,
        });
    }

    if let Some((px, py)) = scene.pointer {
        let (px, py) = (px * scene.scale, py * scene.scale);
        let arm = 14.0 * scene.scale;
        let thick = scene.scale.max(1.0);
        let color = [0.95, 0.15, 0.45, 1.0];
        quads.push(Quad { center: [px, py], half_size: [arm, thick / 2.0], angle: 0.0, color });
        quads.push(Quad { center: [px, py], half_size: [thick / 2.0, arm], angle: 0.0, color });
    }
    quads
}
