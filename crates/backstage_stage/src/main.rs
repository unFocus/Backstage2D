//! Stage process: the editing engine. Renders offscreen with wgpu and streams
//! frames to the tools process through a shared-memory ring.
//!
//! Started by `backstage_tools` as `backstage_stage --socket <path>`. Exits
//! when the socket closes, so it never outlives its tools process. See
//! `docs/adr/0002-stage-process-isolation.md`.

use anyhow::{Context, Result, bail};
use backstage_protocol::{
    FRAME_SLOTS, FrameRing, PROTOCOL_VERSION, ToStage, ToTools, read_message, write_message,
};
use backstage_render::{Renderer, TestScene};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() -> Result<()> {
    let socket_path = parse_args()?;
    let mut socket = UnixStream::connect(&socket_path)
        .with_context(|| format!("connecting to {}", socket_path.display()))?;
    let inbox = spawn_reader(socket.try_clone()?);

    let gpu = pollster::block_on(Gpu::new())?;
    write_message(
        &mut socket,
        &ToTools::Hello {
            version: PROTOCOL_VERSION,
            pid: std::process::id(),
            adapter: gpu.adapter_name.clone(),
        },
    )?;

    let mut renderer = Renderer::new(&gpu.device, FORMAT);
    let mut target: Option<Target> = None;
    let mut generation = 0u64;
    let mut seq = 0u64;
    let mut scale = 1.0f64;
    let mut pointer = None;
    let started = Instant::now();
    let mut next_frame = Instant::now();
    let mut next_heartbeat = Instant::now();

    loop {
        // Drain control messages.
        loop {
            match inbox.try_recv() {
                Ok(ToStage::Hello { version }) if version != PROTOCOL_VERSION => {
                    bail!("protocol mismatch: tools {version}, stage {PROTOCOL_VERSION}")
                }
                Ok(ToStage::Hello { .. }) => {}
                Ok(ToStage::Resize { width, height, scale: s }) => {
                    scale = s;
                    if width == 0 || height == 0 {
                        target = None;
                        continue;
                    }
                    if target.as_ref().is_some_and(|t| t.size == (width, height)) {
                        continue;
                    }
                    generation += 1;
                    let t = Target::new(&gpu.device, (width, height), generation)?;
                    write_message(
                        &mut socket,
                        &ToTools::Surface {
                            generation,
                            path: t.ring_path.display().to_string(),
                            width,
                            height,
                            stride: t.stride,
                        },
                    )?;
                    target = Some(t);
                }
                Ok(ToStage::Pointer(p)) => pointer = p,
                Ok(ToStage::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }

        if let Some(t) = target.as_mut() {
            let scene = TestScene {
                time: started.elapsed().as_secs_f32(),
                scale: scale as f32,
                pointer,
            };
            seq += 1;
            let slot = (seq % FRAME_SLOTS as u64) as u32;
            t.render_into_ring(&gpu, &mut renderer, &scene, slot, seq)?;
            write_message(&mut socket, &ToTools::FrameReady { generation: t.generation, slot, seq })?;
        }

        let now = Instant::now();
        if now >= next_heartbeat {
            write_message(&mut socket, &ToTools::Heartbeat)?;
            next_heartbeat = now + HEARTBEAT_INTERVAL;
        }
        next_frame += FRAME_INTERVAL;
        match next_frame.checked_duration_since(Instant::now()) {
            Some(wait) => std::thread::sleep(wait),
            None => next_frame = Instant::now(), // fell behind; don't try to catch up
        }
    }
}

fn parse_args() -> Result<PathBuf> {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (Some("--socket"), Some(path)) => Ok(path.into()),
        _ => bail!("usage: backstage_stage --socket <path>"),
    }
}

/// Forwards incoming messages to the main loop. The channel disconnects when
/// the socket closes, which the main loop treats as shutdown.
fn spawn_reader(mut socket: UnixStream) -> mpsc::Receiver<ToStage> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(msg) = read_message::<ToStage>(&mut socket) {
            if tx.send(msg).is_err() {
                break;
            }
        }
    });
    rx
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
}

impl Gpu {
    async fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
            .context("no suitable GPU adapter")?;
        let info = adapter.get_info();
        let adapter_name = format!("{} ({:?})", info.name, info.backend);
        eprintln!("backstage_stage: using {adapter_name}");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("stage"),
                ..Default::default()
            })
            .await
            .context("creating GPU device")?;
        Ok(Self { device, queue, adapter_name })
    }
}

/// Offscreen render target, its readback buffer, and the shared-memory ring
/// frames are published through. Recreated on every resize.
struct Target {
    size: (u32, u32),
    generation: u64,
    stride: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    ring: FrameRing,
    ring_path: PathBuf,
}

impl Target {
    fn new(device: &wgpu::Device, size: (u32, u32), generation: u64) -> Result<Self> {
        let stride = (size.0 * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("stage target"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stage readback"),
            size: stride as u64 * size.1 as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let ring_path = ring_path(generation)?;
        let ring = FrameRing::create(&ring_path, stride, size.1)?;
        Ok(Self { size, generation, stride, texture, view, readback, ring, ring_path })
    }

    fn render_into_ring(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        scene: &TestScene,
        slot: u32,
        seq: u64,
    ) -> Result<()> {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        renderer.render(&gpu.device, &gpu.queue, &mut encoder, &self.view, self.size, scene);
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
            wgpu::Extent3d { width: self.size.0, height: self.size.1, depth_or_array_layers: 1 },
        );
        gpu.queue.submit([encoder.finish()]);

        let (tx, rx) = mpsc::channel();
        self.readback.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()?.context("mapping readback buffer")?;
        {
            let pixels = self.readback.get_mapped_range(..)?;
            self.ring.write(slot, seq, &pixels);
        }
        self.readback.unmap();
        Ok(())
    }
}

impl Drop for Target {
    /// Unlinks the ring file. Tools keeps any mapping it already has, so an
    /// in-flight frame from the previous generation still reads correctly.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.ring_path);
    }
}

fn ring_path(generation: u64) -> Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    let dir = PathBuf::from(runtime).join("backstage2d");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("stage-{}-{generation}.frames", std::process::id())))
}
