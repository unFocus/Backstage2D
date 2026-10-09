//! Stage process: the editing engine. Renders offscreen with wgpu and streams
//! frames to the tools process through a shared-memory ring.
//!
//! Started by `backstage_tools` as `backstage_stage --socket <path>`. Exits
//! when the socket closes, so it never outlives its tools process. The
//! document arrives with `Load`; the stage is its only writer (see
//! [`host`]). See `docs/adr/0002-stage-process-isolation.md`.

mod host;
mod pick;
mod selection;
mod transport;
mod view;

use anyhow::{Context, Result, bail};
use backstage_core::{Color, CompId, NodeId, Project, Scene, Vec2, evaluate_from};
use backstage_protocol::{
    FRAME_SLOTS, FrameRing, PROTOCOL_VERSION, PickMode, PointerAt, PointerEvent, ToStage, ToTools,
    read_message, write_message,
};
use backstage_render::pick::Rect;
use backstage_render::{
    Frame, Framing, HOVER, HeadlessGpu, MARQUEE, OFFSCREEN_FORMAT, OffscreenTarget, Overlay, Presentation,
    Renderer, SELECTION, frame_stage,
};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
/// How far off a stroke a click still hits it, in logical pixels.
const PICK_SLOP: f32 = 3.0;
/// How far the pointer moves, in logical pixels, before a press on empty
/// space becomes a marquee rather than a click.
const DRAG_THRESHOLD: f32 = 3.0;
/// Selection and hover boxes sit this far outside the bounds, in logical
/// pixels, so they stay visible on edges shared with the stage.
const BOX_OUTSET: f32 = 2.0;

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
    /// The primary button is down (between `Down` and `Up`).
    held: bool,
    /// The node a click would pick right now (stage state, ADR 0005).
    hover: Option<NodeId>,
    /// The press-drag-release in progress, if any (stage state).
    gesture: Option<Gesture>,
    transport: transport::Transport,
    /// The editor's selection (ADR 0005), changed here first by picks.
    selection: selection::Selection,
    /// Something shown changed since the last frame.
    dirty: bool,
    /// The last `ToTools::Framing` sent, to send it again only on a change.
    framing_sent: Option<ToTools>,
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
            held: false,
            hover: None,
            gesture: None,
            transport: transport::Transport::new(Instant::now()),
            selection: selection::Selection::default(),
            dirty: true,
            framing_sent: None,
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
            ToStage::Pointer(event) => self.on_pointer(event)?,
            ToStage::CancelGesture => {
                self.gesture = None;
                self.dirty = true;
            }
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

    fn on_pointer(&mut self, event: PointerEvent) -> Result<()> {
        self.pointer = event.at().map(|at| (at.x, at.y));
        self.dirty = true;
        match event {
            PointerEvent::Down(at) => {
                self.held = true;
                self.hover = None;
                let Some(hit) = self.hit(at) else { return Ok(()) };
                if let Some(picked) = self.pick_on_press(&hit, at.modifiers.shift) {
                    self.send(&picked)?;
                }
                if hit.node.is_none() {
                    self.gesture = Some(Gesture::Marquee {
                        comp: hit.comp,
                        from: hit.point,
                        to: hit.point,
                        shift: at.modifiers.shift,
                        deep: at.modifiers.ctrl,
                        live: false,
                    });
                }
            }
            PointerEvent::Move(at) if !self.held => {
                self.hover = self.hit(at).and_then(|hit| hit.node);
            }
            PointerEvent::Move(at) => {
                let Some(hit) = self.hit(at) else { return Ok(()) };
                if let Some(Gesture::Marquee { from, to, deep, live, .. }) = &mut self.gesture {
                    *to = hit.point;
                    *deep = at.modifiers.ctrl;
                    *live |= from.distance(*to) >= DRAG_THRESHOLD * hit.per_pixel;
                }
            }
            PointerEvent::Up(at) => {
                self.held = false;
                if let Some(Gesture::Marquee { comp, from, to, shift, deep, live: true }) =
                    self.gesture.take()
                    && let Some(picked) =
                        self.pick_marquee(comp, Rect::around([from, to]).unwrap(), shift, deep)
                {
                    self.send(&picked)?;
                }
                self.gesture = None;
                self.hover = self.hit(at).and_then(|hit| hit.node);
            }
            PointerEvent::Leave => self.hover = None,
        }
        Ok(())
    }

    /// What's under the pointer, or `None` with nothing to pick from yet
    /// (no document or surface).
    fn hit(&self, at: PointerAt) -> Option<Hit> {
        let project = self.host.project()?;
        let framing = self.framing(project)?;
        let shown = Shown::now(project, &self.transport);
        let picker = pick::Picker::new(&project.compositions[&shown.comp]);
        let point = framing.view.inverse().transform_point2(Vec2::new(at.x, at.y) * self.scale as f32);
        // The view maps stage units to physical pixels.
        let per_pixel = self.scale as f32 / framing.view.matrix2.x_axis.x;
        let node = picker.pick(&shown.scene, point, at.modifiers.ctrl, PICK_SLOP * per_pixel);
        Some(Hit { comp: shown.comp, node, point, per_pixel })
    }

    /// A press picks (ADR 0005): the stage changes its mirror at once and
    /// returns the `Picked` to report, or `None` if the selection stays.
    /// - On a node: it becomes the selection, unless already selected (so
    ///   a drag can move a whole multi-selection). Shift toggles it.
    /// - On nothing: clears the selection. Shift leaves it alone.
    fn pick_on_press(&mut self, hit: &Hit, shift: bool) -> Option<ToTools> {
        let comp = hit.comp;
        let current = self.selection.shown(self.host.project()?, comp);
        let (nodes, mode) = match hit.node {
            Some(n) if shift => (vec![n], PickMode::Toggle),
            Some(n) if current.contains(&n) => return None,
            Some(n) => (vec![n], PickMode::Replace),
            None if shift || current.is_empty() => return None,
            None => (Vec::new(), PickMode::Replace),
        };
        self.selection = selection::Selection::set(comp, mode.apply(&current, &nodes));
        Some(ToTools::Picked { comp, nodes, mode })
    }

    /// A marquee over stage rect `rect` picks what it touches: replacing
    /// the selection (already cleared by the press), or adding with Shift.
    /// `None` if that changes nothing, or `comp` is no longer shown.
    fn pick_marquee(&mut self, comp: CompId, rect: Rect, shift: bool, deep: bool) -> Option<ToTools> {
        let project = self.host.project()?;
        let shown = Shown::now(project, &self.transport);
        if shown.comp != comp {
            return None;
        }
        let nodes = pick::Picker::new(&project.compositions[&comp]).in_rect(&shown.scene, rect, deep);
        let current = self.selection.shown(project, comp);
        let mode = if shift { PickMode::Add } else { PickMode::Replace };
        let after = mode.apply(&current, &nodes);
        if after == current {
            return None;
        }
        self.selection = selection::Selection::set(comp, after);
        Some(ToTools::Picked { comp, nodes, mode })
    }

    /// Where the stage sits in the current surface.
    fn framing(&self, project: &Project) -> Option<Framing> {
        let size = self.target.as_ref()?.offscreen.size();
        let s = &project.settings;
        let stage = (s.stage_width as f32, s.stage_height as f32);
        Some(frame_stage(size, stage, self.scale as f32, Presentation::Editor, s.pixel_art))
    }

    /// Draws a frame into the ring and announces it. Nothing to show until
    /// there's both a surface and a document; it stays dirty until then.
    fn draw(&mut self) -> Result<()> {
        // Tell the editor where the stage is first, if that changed, so it
        // can map what this frame shows.
        if let Some(framing) = self.host.project().and_then(|p| self.framing(p)) {
            let s = self.scale as f32;
            let origin = framing.view.translation / s;
            let msg =
                ToTools::Framing { origin: (origin.x, origin.y), scale: framing.view.matrix2.x_axis.x / s };
            if self.framing_sent.as_ref() != Some(&msg) {
                self.send(&msg)?;
                self.framing_sent = Some(msg);
            }
        }
        let (Some(t), Some(project)) = (self.target.as_mut(), self.host.project()) else { return Ok(()) };
        let shown = Shown::now(project, &self.transport);
        let picker = pick::Picker::new(&project.compositions[&shown.comp]);
        let selected = self.selection.shown(project, shown.comp);
        // A live marquee previews what it would pick; otherwise the hover.
        let marquee = match &self.gesture {
            Some(Gesture::Marquee { comp, from, to, deep, live: true, .. }) if *comp == shown.comp => {
                Rect::around([*from, *to]).map(|rect| (rect, *deep))
            }
            _ => None,
        };
        let preview = match marquee {
            Some((rect, deep)) => picker.in_rect(&shown.scene, rect, deep),
            None => self.hover.into_iter().collect(),
        };
        let hovered = preview.into_iter().filter(|n| !selected.contains(n));
        let marks = selected.iter().map(|&n| (n, SELECTION)).chain(hovered.map(|n| (n, HOVER)));
        let mut overlay: Vec<Overlay> = marks
            .filter_map(|(n, color)| {
                Some(Overlay::Box { rect: picker.bounds(&shown.scene, n)?, color, outset: BOX_OUTSET })
            })
            .collect();
        if let Some((rect, _)) = marquee {
            overlay.push(Overlay::Fill { rect, color: MARQUEE });
            overlay.push(Overlay::Box { rect, color: SELECTION, outset: 0.0 });
        }
        let frame = Frame {
            project,
            scene: &shown.scene,
            scale: self.scale as f32,
            pointer: self.pointer,
            presentation: Presentation::Editor,
            outlines: &shown.outlines,
            origin_marker: shown.origin,
            overlay: &overlay,
        };
        self.seq += 1;
        let slot = (self.seq % FRAME_SLOTS as u64) as u32;
        t.render_into_ring(&self.gpu, &mut self.renderer, &frame, slot, self.seq)?;
        let ready = ToTools::FrameReady { generation: t.generation, slot, seq: self.seq };
        self.dirty = false;
        self.send(&ready)
    }
}

/// What's under the pointer.
struct Hit {
    /// The composition shown.
    comp: CompId,
    /// The node a click there picks (Ctrl: the deepest one).
    node: Option<NodeId>,
    /// The pointer in stage coordinates.
    point: Vec2,
    /// Stage units per logical pixel.
    per_pixel: f32,
}

/// A press-drag-release in progress on the stage. Stage state (ADR 0005):
/// lost if the stage restarts, and dropped by `CancelGesture`.
enum Gesture {
    /// Pressed on empty space at `from` (stage coordinates): a marquee to
    /// `to`, once the pointer has moved far enough to be `live`. `shift`
    /// was held at the press (add, don't replace); `deep` is Ctrl now.
    Marquee { comp: CompId, from: Vec2, to: Vec2, shift: bool, deep: bool, live: bool },
}

/// The edited composition as the editor shows it right now: the root
/// unless the editor entered another one, which is shown on its own with
/// its origin at the stage centre. Hidden nodes are left out.
struct Shown<'p> {
    comp: CompId,
    scene: Scene<'p>,
    outlines: Vec<Option<Color>>,
    /// Where the origin was moved to, if it was.
    origin: Option<Vec2>,
}

impl<'p> Shown<'p> {
    fn now(project: &'p Project, transport: &transport::Transport) -> Self {
        let comp = transport.composition().filter(|c| project.compositions.contains_key(c));
        let comp = comp.unwrap_or(project.root);
        let scene = evaluate_from(project, &transport.state(), comp, transport.now(Instant::now()));
        let origin = (comp != project.root).then(|| {
            let s = &project.settings;
            Vec2::new(s.stage_width as f32, s.stage_height as f32) / 2.0
        });
        // Hide and outline: editor-only, so applied here, never in evaluate.
        let (scene, outlines) = view::editor_view(project, comp, scene, origin.unwrap_or(Vec2::ZERO));
        Self { comp, scene, outlines, origin }
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
