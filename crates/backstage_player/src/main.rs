//! Standalone player: plays a project in its own window with the same core
//! and renderer as the editor's stage, without the editor.
//!
//! `backstage_player [project.bs2d] [--max-fps N] [--frames N]`
//!
//! Keys: Space pauses, F / F11 toggles fullscreen, Escape or Ctrl+Q quits.

use anyhow::{Context, Result, bail};
use backstage_core::{Project, RuntimeState, Time, evaluate};
use backstage_render::{Frame, Presentation, Renderer, adapter_options};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Fullscreen, Window, WindowId};

#[derive(Debug, Default, PartialEq)]
struct Args {
    project: Option<PathBuf>,
    max_fps: Option<u32>,
    /// Quit after presenting this many frames (for automated tests).
    frames: Option<u64>,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args> {
    let usage = "usage: backstage_player [project.bs2d] [--max-fps N] [--frames N]";
    let mut parsed = Args::default();
    while let Some(arg) = args.next() {
        let mut number = |flag: &str| -> Result<u64> {
            let value = args.next().with_context(|| format!("{flag} needs a value; {usage}"))?;
            match value.parse::<u64>() {
                Ok(n) if n > 0 => Ok(n),
                _ => bail!("{flag} needs a positive number, got {value:?}; {usage}"),
            }
        };
        match arg.as_str() {
            "--max-fps" => parsed.max_fps = Some(number("--max-fps")?.min(u32::MAX as u64) as u32),
            "--frames" => parsed.frames = Some(number("--frames")?),
            flag if flag.starts_with("--") => bail!("unknown option {flag}; {usage}"),
            _ if parsed.project.is_some() => bail!("more than one project given; {usage}"),
            path => parsed.project = Some(PathBuf::from(path)),
        }
    }
    Ok(parsed)
}

fn main() -> Result<()> {
    let args = parse_args(std::env::args().skip(1))?;
    let project = match &args.project {
        Some(dir) => backstage_core::load(dir).with_context(|| format!("loading {}", dir.display()))?,
        None => backstage_core::sample::bounce(),
    };
    let event_loop = EventLoop::new().context("opening a Wayland connection")?;
    let mut app = App::new(project, &args, &event_loop);
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// The GPU side, created once the window exists.
struct Gpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    view_format: wgpu::TextureFormat,
    renderer: Renderer,
}

/// Play time that can be paused: elapsed time minus time spent paused.
struct Clock {
    started: Instant,
    paused_at: Option<Instant>,
    paused_total: Duration,
}

impl Clock {
    fn new() -> Self {
        Self { started: Instant::now(), paused_at: None, paused_total: Duration::ZERO }
    }

    fn toggle_pause(&mut self) {
        match self.paused_at.take() {
            Some(at) => self.paused_total += at.elapsed(),
            None => self.paused_at = Some(Instant::now()),
        }
    }

    fn now(&self) -> Time {
        let end = self.paused_at.unwrap_or_else(Instant::now);
        let played = end.duration_since(self.started).saturating_sub(self.paused_total);
        Time::from_ratio(played.as_nanos() as i64, 1_000_000_000)
    }
}

struct App {
    project: Project,
    state: RuntimeState,
    instance: wgpu::Instance,
    gpu: Option<Gpu>,
    clock: Clock,
    modifiers: ModifiersState,
    frame_interval: Option<Duration>,
    next_frame: Instant,
    frames_left: Option<u64>,
    error: Option<anyhow::Error>,
}

impl App {
    fn new(project: Project, args: &Args, event_loop: &EventLoop<()>) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
            event_loop.owned_display_handle(),
        )));
        Self {
            project,
            state: RuntimeState::default(),
            instance,
            gpu: None,
            clock: Clock::new(),
            modifiers: ModifiersState::empty(),
            frame_interval: args.max_fps.map(|fps| Duration::from_secs_f64(1.0 / fps as f64)),
            next_frame: Instant::now(),
            frames_left: args.frames,
            error: None,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }

    fn create_gpu(&self, event_loop: &ActiveEventLoop) -> Result<Gpu> {
        let settings = &self.project.settings;
        let title = self.project.root_composition().map_or("Backstage2D", |c| c.name.as_str());
        let attributes = Window::default_attributes()
            .with_title(format!("{title} — Backstage2D"))
            .with_inner_size(LogicalSize::new(settings.stage_width, settings.stage_height));
        let window = Arc::new(event_loop.create_window(attributes)?);
        let surface = self.instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(self.instance.request_adapter(&adapter_options(Some(&surface))))
            .context("no GPU adapter can draw to this window")?;
        let info = adapter.get_info();
        eprintln!("backstage_player: using {} ({:?})", info.name, info.backend);
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor { label: Some("player"), ..Default::default() }),
        )?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("the window surface isn't supported by this adapter")?;
        // The renderer writes sRGB-encoded values directly, so it needs a
        // non-sRGB view of the swapchain.
        let caps = surface.get_capabilities(&adapter);
        config.format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(config.format);
        let view_format = config.format.remove_srgb_suffix();
        config.view_formats = if view_format != config.format { vec![view_format] } else { vec![] };
        config.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&device, &config);
        let renderer = Renderer::new(&device, view_format);
        Ok(Gpu { window, surface, device, queue, config, view_format, renderer })
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        let texture = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                gpu.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                gpu.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                let error = anyhow::anyhow!("the window surface failed validation");
                self.fail(event_loop, error);
                return;
            }
        };
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(gpu.view_format),
            ..Default::default()
        });
        let scene = evaluate(&self.project, &self.state, self.clock.now());
        let frame = Frame {
            project: &self.project,
            scene: &scene,
            scale: gpu.window.scale_factor() as f32,
            pointer: None,
            presentation: Presentation::Player,
        };
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let size = (gpu.config.width, gpu.config.height);
        gpu.renderer.render(&gpu.device, &gpu.queue, &mut encoder, &view, size, &frame);
        gpu.queue.submit([encoder.finish()]);
        gpu.window.pre_present_notify();
        gpu.queue.present(texture);

        if let Some(left) = self.frames_left.as_mut() {
            *left -= 1;
            if *left == 0 {
                event_loop.exit();
                return;
            }
        }
        match self.frame_interval {
            Some(interval) => {
                self.next_frame = (self.next_frame + interval).max(Instant::now());
                event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
            }
            None => gpu.window.request_redraw(),
        }
    }

    fn key(&mut self, event_loop: &ActiveEventLoop, event: KeyEvent) {
        if event.state != ElementState::Pressed || event.repeat {
            return;
        }
        match event.logical_key.as_ref() {
            Key::Named(NamedKey::Escape) => event_loop.exit(),
            Key::Character("q") if self.modifiers.control_key() => event_loop.exit(),
            Key::Named(NamedKey::Space) => self.clock.toggle_pause(),
            Key::Named(NamedKey::F11) | Key::Character("f") => {
                if let Some(gpu) = &self.gpu {
                    let full = gpu.window.fullscreen().is_some();
                    gpu.window.set_fullscreen(if full { None } else { Some(Fullscreen::Borderless(None)) });
                }
            }
            _ => {}
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        match self.create_gpu(event_loop) {
            Ok(gpu) => {
                gpu.window.request_redraw();
                self.gpu = Some(gpu);
            }
            Err(e) => self.fail(event_loop, e),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut()
                    && size.width > 0
                    && size.height > 0
                {
                    gpu.config.width = size.width;
                    gpu.config.height = size.height;
                    gpu.surface.configure(&gpu.device, &gpu.config);
                    gpu.window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } => self.key(event_loop, event),
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        // With a frame cap, wake up on schedule and draw the next frame.
        if self.frame_interval.is_some()
            && Instant::now() >= self.next_frame
            && let Some(gpu) = &self.gpu
        {
            gpu.window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args> {
        parse_args(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_project_and_options() {
        assert_eq!(args(&[]).unwrap(), Args::default());
        assert_eq!(
            args(&["p.bs2d", "--max-fps", "30", "--frames", "5"]).unwrap(),
            Args { project: Some(PathBuf::from("p.bs2d")), max_fps: Some(30), frames: Some(5) }
        );
    }

    #[test]
    fn rejects_bad_arguments() {
        for bad in
            [&["--max-fps"][..], &["--max-fps", "0"], &["--frames", "x"], &["--bogus"], &["a.bs2d", "b.bs2d"]]
        {
            assert!(args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn paused_clock_stands_still() {
        let mut clock = Clock::new();
        clock.toggle_pause();
        let t = clock.now();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(clock.now(), t);
        clock.toggle_pause();
        std::thread::sleep(Duration::from_millis(20));
        assert!(clock.now() > t);
    }
}
