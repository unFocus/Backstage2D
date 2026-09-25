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
use backstage_render::{HeadlessGpu, OFFSCREEN_FORMAT, OffscreenTarget, Renderer, TestScene};
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
    let socket_path = parse_args(std::env::args().skip(1))?;
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

    let mut renderer = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
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
                Ok(ToStage::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }

        if let Some(t) = target.as_mut() {
            let scene = TestScene { time: started.elapsed().as_secs_f32(), scale: scale as f32, pointer };
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

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<PathBuf> {
    match (args.next().as_deref(), args.next(), args.next()) {
        (Some("--socket"), Some(path), None) => Ok(path.into()),
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
        scene: &TestScene,
        slot: u32,
        seq: u64,
    ) -> Result<()> {
        let ring = &mut self.ring;
        self.offscreen
            .render_and_read(gpu, renderer, scene, |pixels| ring.write(slot, seq, pixels))
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
    use super::parse_args;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn parses_socket_path() {
        assert_eq!(parse_args(args(&["--socket", "/run/x.sock"])).unwrap().to_str(), Some("/run/x.sock"));
    }

    #[test]
    fn rejects_bad_arguments() {
        for bad in [&[][..], &["--socket"], &["--sock", "/x"], &["--socket", "/x", "extra"]] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }
}
