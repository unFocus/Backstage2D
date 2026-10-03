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

    let mut host = host::DocumentHost::default();

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

    let mut renderer = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
    let mut target: Option<Target> = None;
    let mut generation = 0u64;
    let mut seq = 0u64;
    let mut scale = 1.0f64;
    let mut pointer = None;
    let mut transport = transport::Transport::new(Instant::now());
    // The editor's selection (ADR 0005). Nothing draws it yet: M4's
    // selection handles and drags read it through `Selection::shown`.
    let mut _selection = selection::Selection::default();
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
                    if target.as_ref().is_some_and(|t| t.offscreen.size() == (width, height)) {
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
                            stride: t.offscreen.stride(),
                        },
                    )?;
                    target = Some(t);
                }
                Ok(ToStage::Pointer(p)) => pointer = p,
                Ok(ToStage::Selection { comp, nodes }) => _selection = selection::Selection::set(comp, nodes),
                Ok(ToStage::Transport { composition, animation, time, playing }) => {
                    transport =
                        transport::Transport::set(composition, animation, time, playing, Instant::now());
                }
                Ok(ToStage::Load { base, log }) => match host.load(&base, &log) {
                    Ok(reply) => {
                        renderer.clear_cache();
                        write_message(&mut socket, &reply)?;
                    }
                    Err(e) => {
                        // Tell the tools process why, then fail (its
                        // supervisor decides whether to retry).
                        let _ = write_message(&mut socket, &ToTools::Log(e.clone()));
                        bail!(e);
                    }
                },
                Ok(ToStage::Submit { request, entry }) => {
                    let reply = host.submit(request, &entry);
                    if matches!(reply, ToTools::Committed { .. }) {
                        // Meshes are cached by shape address, which adding or
                        // removing nodes can move or reuse.
                        renderer.clear_cache();
                    }
                    write_message(&mut socket, &reply)?;
                }
                Ok(ToStage::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }

        // Nothing to show until the document arrives.
        if let (Some(t), Some(project)) = (target.as_mut(), host.project()) {
            // The edited composition (the root unless the editor entered
            // another one, which is shown on its own, origin at the centre).
            let comp = transport.composition().filter(|c| project.compositions.contains_key(c));
            let comp = comp.unwrap_or(project.root);
            let scene = evaluate_from(project, &transport.state(), comp, transport.now(Instant::now()));
            let origin = (comp != project.root).then(|| {
                let s = &project.settings;
                Vec2::new(s.stage_width as f32, s.stage_height as f32) / 2.0
            });
            // Hide and outline: editor-only, so applied here, never in evaluate.
            let (scene, outlines) = view::editor_view(project, comp, scene, origin.unwrap_or(Vec2::ZERO));
            let frame = Frame {
                project,
                scene: &scene,
                scale: scale as f32,
                pointer,
                presentation: Presentation::Editor,
                outlines: &outlines,
                origin_marker: origin,
            };
            seq += 1;
            let slot = (seq % FRAME_SLOTS as u64) as u32;
            t.render_into_ring(&gpu, &mut renderer, &frame, slot, seq)?;
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
