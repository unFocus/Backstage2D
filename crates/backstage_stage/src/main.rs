//! Stage process: the editing engine. Renders offscreen with wgpu and streams
//! frames to the tools process through a shared-memory ring.
//!
//! Started by `backstage_tools` as `backstage_stage --socket <path>`. Exits
//! when the socket closes, so it never outlives its tools process. The
//! document arrives with `Load`; the stage is its only writer (see
//! [`host`]). See `docs/adr/0002-stage-process-isolation.md`.

mod host;
mod selection;
mod transport;
mod view;

use anyhow::{Context, Result, bail};
use backstage_core::{Vec2, evaluate_from};
use backstage_protocol::{
    FRAME_SLOTS, FrameRing, PROTOCOL_VERSION, ToStage, ToTools, read_message, write_message,
};
use backstage_render::{Frame, HeadlessGpu, OFFSCREEN_FORMAT, OffscreenTarget, Presentation, Renderer};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

fn main() -> Result<()> {
    match run() {
        // The tools process went away mid-write: that's a normal shutdown.
        Err(e) if is_disconnect(&e) => Ok(()),
        result => result,
    }
}

fn is_disconnect(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>().is_some_and(|e| {
        matches!(e.kind(), std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset)
    })
}

fn run() -> Result<()> {
    let Args { socket: socket_path } = parse_args(std::env::args().skip(1))?;
    let mut socket = UnixStream::connect(&socket_path)
        .with_context(|| format!("connecting to {}", socket_path.display()))?;
    let inbox = spawn_reader(socket.try_clone()?);

    let gpu = pollster::block_on(HeadlessGpu::new("stage"))
        .map_err(|e| anyhow::anyhow!("initializing GPU: {e}"))?;
    eprintln!("backstage_stage: using {}", gpu.adapter_name);
    write_message(
        &mut socket,
        &ToTools::Hello {
            version: PROTOCOL_VERSION,
            pid: std::process::id(),
            adapter: gpu.adapter_name.clone(),
        },
    )?;

    let mut stage = Stage::new(gpu, socket);
    let mut next_tick = Instant::now();
    let mut next_heartbeat = Instant::now();

    // Frames are drawn when something changed (input, an edit, the
    // transport) and on a 60 Hz tick while playing. Input is drawn as soon
    // as it arrives instead of waiting for the tick; everything queued is
    // handled first, so a burst of pointer moves makes one frame.
    loop {
        let wake = if stage.transport.playing() { next_tick.min(next_heartbeat) } else { next_heartbeat };
        match inbox.recv_timeout(wake.saturating_duration_since(Instant::now())) {
            Ok(msg) => {
                if stage.handle(msg)? == Flow::Exit {
                    return Ok(());
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        loop {
            match inbox.try_recv() {
                Ok(msg) => {
                    if stage.handle(msg)? == Flow::Exit {
                        return Ok(());
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }

        let now = Instant::now();
        if stage.transport.playing() && now >= next_tick {
            stage.dirty = true;
            next_tick += FRAME_INTERVAL;
            if next_tick < now {
                next_tick = now + FRAME_INTERVAL; // fell behind; don't try to catch up
            }
        }
        if stage.dirty {
            stage.draw()?;
        }
        if now >= next_heartbeat {
            stage.send(&ToTools::Heartbeat)?;
            next_heartbeat = now + HEARTBEAT_INTERVAL;
        }
    }
}

/// Whether the main loop carries on after a message.
#[derive(Debug, PartialEq)]
enum Flow {
    Continue,
    Exit,
}

/// Everything the stage process holds between messages.
struct Stage {
    gpu: HeadlessGpu,
    socket: UnixStream,
    host: host::DocumentHost,
    renderer: Renderer,
    target: Option<Target>,
    generation: u64,
    seq: u64,
    scale: f64,
    /// Where the pointer is over the stage section, in logical pixels.
    pointer: Option<(f32, f32)>,
    transport: transport::Transport,
    /// The editor's selection (ADR 0005). Nothing draws it yet: M4's
    /// selection handles and drags read it through `Selection::shown`.
    selection: selection::Selection,
    /// Something shown changed since the last frame.
    dirty: bool,
}

impl Stage {
    fn new(gpu: HeadlessGpu, socket: UnixStream) -> Self {
        let renderer = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
        Self {
            gpu,
            socket,
            host: host::DocumentHost::default(),
            renderer,
            target: None,
            generation: 0,
            seq: 0,
            scale: 1.0,
            pointer: None,
            transport: transport::Transport::new(Instant::now()),
            selection: selection::Selection::default(),
            dirty: true,
        }
    }

    fn send(&mut self, msg: &ToTools) -> Result<()> {
        Ok(write_message(&mut self.socket, msg)?)
    }

    fn handle(&mut self, msg: ToStage) -> Result<Flow> {
        match msg {
            ToStage::Hello { version } if version != PROTOCOL_VERSION => {
                bail!("protocol mismatch: tools {version}, stage {PROTOCOL_VERSION}")
            }
            ToStage::Hello { .. } => {}
            ToStage::Resize { width, height, scale } => {
                self.scale = scale;
                self.dirty = true;
                if width == 0 || height == 0 {
                    self.target = None;
                    return Ok(Flow::Continue);
                }
                if self.target.as_ref().is_some_and(|t| t.offscreen.size() == (width, height)) {
                    return Ok(Flow::Continue);
                }
                self.generation += 1;
                let t = Target::new(&self.gpu.device, (width, height), self.generation)?;
                let surface = ToTools::Surface {
                    generation: self.generation,
                    path: t.ring_path.display().to_string(),
                    width,
                    height,
                    stride: t.offscreen.stride(),
                };
                self.send(&surface)?;
                self.target = Some(t);
            }
            ToStage::Pointer(event) => {
                self.pointer = event.at().map(|at| (at.x, at.y));
                self.dirty = true;
            }
            ToStage::CancelGesture => {} // no gestures yet
            ToStage::Selection { comp, nodes } => {
                self.selection = selection::Selection::set(comp, nodes);
                self.dirty = true;
            }
            ToStage::Transport { composition, animation, time, playing } => {
                self.transport =
                    transport::Transport::set(composition, animation, time, playing, Instant::now());
                self.dirty = true;
            }
            ToStage::Load { base, log } => match self.host.load(&base, &log) {
                Ok(reply) => {
                    self.renderer.clear_cache();
                    self.dirty = true;
                    self.send(&reply)?;
                }
                Err(e) => {
                    // Tell the tools process why, then fail (its
                    // supervisor decides whether to retry).
                    let _ = self.send(&ToTools::Log(e.clone()));
                    bail!(e);
                }
            },
            ToStage::Submit { request, entry } => {
                let reply = self.host.submit(request, &entry);
                if matches!(reply, ToTools::Committed { .. }) {
                    // Meshes are cached by shape address, which adding or
                    // removing nodes can move or reuse.
                    self.renderer.clear_cache();
                    self.dirty = true;
                }
                self.send(&reply)?;
            }
            ToStage::Shutdown => return Ok(Flow::Exit),
        }
        Ok(Flow::Continue)
    }

    /// Draws a frame into the ring and announces it. Nothing to show until
    /// there's both a surface and a document; it stays dirty until then.
    fn draw(&mut self) -> Result<()> {
        let (Some(t), Some(project)) = (self.target.as_mut(), self.host.project()) else { return Ok(()) };
        // The edited composition (the root unless the editor entered
        // another one, which is shown on its own, origin at the centre).
        let comp = self.transport.composition().filter(|c| project.compositions.contains_key(c));
        let comp = comp.unwrap_or(project.root);
        let scene = evaluate_from(project, &self.transport.state(), comp, self.transport.now(Instant::now()));
        let origin = (comp != project.root).then(|| {
            let s = &project.settings;
            Vec2::new(s.stage_width as f32, s.stage_height as f32) / 2.0
        });
        // Hide and outline: editor-only, so applied here, never in evaluate.
        let (scene, outlines) = view::editor_view(project, comp, scene, origin.unwrap_or(Vec2::ZERO));
        let frame = Frame {
            project,
            scene: &scene,
            scale: self.scale as f32,
            pointer: self.pointer,
            presentation: Presentation::Editor,
            outlines: &outlines,
            origin_marker: origin,
        };
        self.seq += 1;
        let slot = (self.seq % FRAME_SLOTS as u64) as u32;
        t.render_into_ring(&self.gpu, &mut self.renderer, &frame, slot, self.seq)?;
        let ready = ToTools::FrameReady { generation: t.generation, slot, seq: self.seq };
        self.dirty = false;
        self.send(&ready)
    }
}

/// Command-line arguments: `--socket <path>`.
#[derive(Debug, PartialEq)]
struct Args {
    socket: PathBuf,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args> {
    let usage = "usage: backstage_stage --socket <path>";
    let mut socket = None;
    while let Some(flag) = args.next() {
        if flag != "--socket" {
            bail!("unknown argument {flag:?}; {usage}");
        }
        let value = args.next().with_context(|| format!("{flag} needs a value; {usage}"))?;
        if socket.replace(PathBuf::from(value)).is_some() {
            bail!("{flag} given twice; {usage}");
        }
    }
    Ok(Args { socket: socket.context(usage)? })
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

/// The offscreen target plus the shared-memory ring its frames are
/// published through. Recreated on every resize.
struct Target {
    generation: u64,
    offscreen: OffscreenTarget,
    ring: FrameRing,
    ring_path: PathBuf,
}

impl Target {
    fn new(device: &wgpu::Device, size: (u32, u32), generation: u64) -> Result<Self> {
        let offscreen = OffscreenTarget::new(device, size);
        let ring_path = ring_path(generation)?;
        let ring = FrameRing::create(&ring_path, offscreen.stride(), size.1)?;
        Ok(Self { generation, offscreen, ring, ring_path })
    }

    fn render_into_ring(
        &mut self,
        gpu: &HeadlessGpu,
        renderer: &mut Renderer,
        frame: &Frame,
        slot: u32,
        seq: u64,
    ) -> Result<()> {
        let ring = &mut self.ring;
        self.offscreen
            .render_and_read(gpu, renderer, frame, |pixels| ring.write(slot, seq, pixels))
            .map_err(|e| anyhow::anyhow!("rendering frame: {e}"))
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

#[cfg(test)]
mod tests {
    use super::{Args, parse_args};
    use std::path::PathBuf;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn parses_socket() {
        assert_eq!(
            parse_args(args(&["--socket", "/run/x.sock"])).unwrap(),
            Args { socket: PathBuf::from("/run/x.sock") }
        );
    }

    #[test]
    fn rejects_bad_arguments() {
        for bad in [
            &[][..],
            &["--socket"],
            &["--sock", "/x"],
            &["--socket", "/x", "extra"],
            &["--project", "p", "--socket", "/s"],
            &["--socket", "/a", "--socket", "/b"],
        ] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }
}
