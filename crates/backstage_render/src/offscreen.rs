//! Headless GPU setup and offscreen rendering with CPU readback. Used by the
//! stage process and by tests (golden images).

use crate::{Frame, Renderer};
use std::sync::mpsc;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Texture format of offscreen targets. Rows read back as tightly packed
/// RGBA8 pixels (plus row padding up to [`OffscreenTarget::stride`]).
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Set to `1` to force wgpu's fallback (software) adapter, e.g. lavapipe.
/// Tests always set it so they run without a GPU and render deterministically.
pub const FALLBACK_ENV: &str = "BACKSTAGE_WGPU_FALLBACK";

pub struct HeadlessGpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// Human-readable adapter description, e.g. "llvmpipe (...) (Vulkan)".
    pub adapter_name: String,
}

impl HeadlessGpu {
    pub async fn new(label: &str) -> Result<Self, Error> {
        let fallback = std::env::var(FALLBACK_ENV).is_ok_and(|v| v == "1");
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: fallback,
                ..Default::default()
            })
            .await?;
        let info = adapter.get_info();
        let adapter_name = format!("{} ({:?})", info.name, info.backend);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor { label: Some(label), ..Default::default() })
            .await?;
        Ok(Self { device, queue, adapter_name })
    }
}

/// An offscreen render target plus a buffer to read it back into.
pub struct OffscreenTarget {
    size: (u32, u32),
    stride: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

impl OffscreenTarget {
    pub fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let stride = (size.0 * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen target"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offscreen readback"),
            size: stride as u64 * size.1 as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self { size, stride, texture, view, readback }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Bytes per row of the read-back pixels (≥ width × 4, padded for wgpu).
    pub fn stride(&self) -> u32 {
        self.stride
    }

    /// Renders `frame`, waits for the GPU, and passes the pixels
    /// (`stride × height` bytes) to `read`.
    pub fn render_and_read(
        &mut self,
        gpu: &HeadlessGpu,
        renderer: &mut Renderer,
        frame: &Frame,
        read: impl FnOnce(&[u8]),
    ) -> Result<(), Error> {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        renderer.render(&gpu.device, &gpu.queue, &mut encoder, &self.view, self.size, frame);
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.stride),
                    rows_per_image: None,
                },
            },
            extent(self.size),
        );
        gpu.queue.submit([encoder.finish()]);

        let (tx, rx) = mpsc::channel();
        self.readback.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        {
            let pixels = self.readback.get_mapped_range(..)?;
            read(&pixels);
        }
        self.readback.unmap();
        Ok(())
    }
}

fn extent(size: (u32, u32)) -> wgpu::Extent3d {
    wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 }
}
