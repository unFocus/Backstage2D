//! Integration tests: the real stage binary on the software adapter, driven
//! over the real protocol.

mod common;

use backstage_core::sample::{self, ids};
use backstage_core::{Color, Command, Document, Entry, ProjectSettings, Time};
use backstage_protocol::{PointerAt, PointerEvent, Snapshot, ToStage, ToTools};
use backstage_render::{CROSSHAIR, PASTEBOARD};
use common::{StageHarness, color_close};
use std::time::{Duration, Instant};

fn pasteboard() -> [f32; 4] {
    [PASTEBOARD.r as f32, PASTEBOARD.g as f32, PASTEBOARD.b as f32, 1.0]
}

fn resize(stage: &mut StageHarness, width: u32, height: u32) -> u64 {
    stage.send(&ToStage::Resize { width, height, scale: 1.0 });
    stage.recv_until(|m| match m {
        ToTools::Surface { generation, width: w, height: h, stride, .. } => {
            assert_eq!((*w, *h), (width, height));
            assert!(*stride >= width * 4 && stride % 256 == 0, "stride {stride}");
            Some(*generation)
        }
        _ => None,
    })
}

fn stage_interior(stage: &mut StageHarness) -> [u8; 4] {
    // Near the stage's top-left corner, where the sample draws nothing.
    stage.next_frame().pixel(60, 45)
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

fn set_background(color: Color) -> Entry {
    let settings = ProjectSettings { background: color, ..sample::bounce().settings };
    Entry::Do(Command::SetSettings(settings))
}

#[test]
fn shows_nothing_until_a_document_is_loaded() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    resize(&mut stage, 64, 64);
    // Well over a second of 60 Hz ticks: heartbeats, but no frames.
    let mut heartbeats = 0;
    while heartbeats < 2 {
        match stage.recv() {
            ToTools::FrameReady { .. } => panic!("frame before Load"),
            ToTools::Heartbeat => heartbeats += 1,
            _ => {}
        }
    }
    stage.load_sample();
    stage.next_frame();
}

#[test]
fn loads_a_project_directory() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/bounce.bs2d");
    let project = backstage_core::load(std::path::Path::new(dir)).unwrap();
    let mut stage = StageHarness::spawn();
    stage.handshake();
    let (seq, hash) = stage.load(Snapshot::of(&project).unwrap(), vec![]);
    assert_eq!((seq, hash), (0, Document::new(project).hash()));
    resize(&mut stage, 550, 400);
    // The sample's green ground strip spans the stage's bottom (stage y 340..400).
    let frame = stage.next_frame();
    assert!(
        color_close(
            frame.pixel(275, 360),
            [0x6a as f32 / 255.0, 0xbf as f32 / 255.0, 0x4b as f32 / 255.0, 1.0]
        ),
        "{:?}",
        frame.pixel(275, 360)
    );
}

#[test]
fn load_replays_the_log() {
    let log = vec![set_background(Color::rgb8(255, 0, 0)), Entry::Undo, Entry::Redo];
    let expected = Document::replay(sample::bounce(), &log).unwrap();
    let mut stage = StageHarness::spawn();
    stage.handshake();
    let loaded = stage.load(Snapshot::of(&sample::bounce()).unwrap(), log);
    assert_eq!(loaded, (3, expected.hash()));
    resize(&mut stage, 320, 200);
    assert!(color_close(stage_interior(&mut stage), RED));
}

#[test]
fn committed_edits_undo_and_redo_show_on_stage() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    stage.load_sample();
    resize(&mut stage, 320, 200);
    assert!(color_close(stage_interior(&mut stage), [1.0; 4]));

    let red = set_background(Color::rgb8(255, 0, 0));
    assert_eq!(stage.submit(7, red.clone()), ToTools::Committed { seq: 1, request: Some(7), entry: red });
    assert!(color_close(stage_interior(&mut stage), RED));

    assert_eq!(
        stage.submit(8, Entry::Undo),
        ToTools::Committed { seq: 2, request: Some(8), entry: Entry::Undo }
    );
    assert!(color_close(stage_interior(&mut stage), [1.0; 4]));

    assert_eq!(
        stage.submit(9, Entry::Redo),
        ToTools::Committed { seq: 3, request: Some(9), entry: Entry::Redo }
    );
    assert!(color_close(stage_interior(&mut stage), RED));
}

/// Sets the playhead, and returns once every later frame reflects it: the
/// stage answers requests in order, so a rejected no-op makes a barrier.
/// A paused stage draws only when something changes, and its frame for the
/// new playhead may come before the barrier's answer, so a pointer `Leave`
/// after the barrier makes sure one more frame follows it.
fn transport(stage: &mut StageHarness, animation: Option<backstage_core::AnimId>, secs: f64, playing: bool) {
    show(stage, None, animation, secs, playing);
}

/// Like [`transport`], also choosing the composition shown.
fn show(
    stage: &mut StageHarness,
    composition: Option<backstage_core::CompId>,
    animation: Option<backstage_core::AnimId>,
    secs: f64,
    playing: bool,
) {
    stage.send(&ToStage::Transport { composition, animation, time: Time::from_secs_f64(secs), playing });
    stage.send(&ToStage::Submit { request: u64::MAX, entry: Entry::Redo });
    stage.send(&ToStage::Pointer(PointerEvent::Leave));
    stage.recv_until(|m| matches!(m, ToTools::Rejected { request: u64::MAX, .. }).then_some(()));
}

#[test]
fn another_composition_is_shown_on_its_own_around_the_stage_centre() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    stage.load_sample();
    resize(&mut stage, 550, 400);
    let orange = |p: [u8; 4]| p[0] > 200 && (90..200).contains(&p[1]) && p[2] < 100;
    // The editor frames a 550 × 400 stage in a 550 × 400 viewport with a
    // margin; the stage centre is the viewport centre either way. Sample
    // just off it, clear of the origin mark's arms but inside the ball.
    let centre = |stage: &mut StageHarness| stage.next_frame().pixel(283, 208);

    show(&mut stage, None, None, 0.0, false);
    assert!(!orange(centre(&mut stage)), "the root has nothing at the centre: {:?}", centre(&mut stage));

    // At 0.5 s the bounce lands: the body sits on its origin.
    show(&mut stage, Some(ids::BALL), None, 0.5, false);
    assert!(orange(centre(&mut stage)), "the ball, centred: {:?}", centre(&mut stage));

    show(&mut stage, Some(backstage_core::CompId::from_raw(1)), None, 0.5, false);
    assert!(!orange(centre(&mut stage)), "an unknown composition falls back to the root");
}

#[test]
fn the_transport_pauses_seeks_and_picks_the_animation() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    stage.load_sample();
    resize(&mut stage, 275, 200);

    transport(&mut stage, None, 0.5, false);
    let paused = stage.next_frame().pixels;

    transport(&mut stage, None, 1.5, false);
    assert!(stage.next_frame().pixels != paused, "seeking moves the scene");

    transport(&mut stage, Some(ids::STAGE_MAIN), 0.5, false);
    assert!(stage.next_frame().pixels == paused, "`main` is the root's default animation");
    transport(&mut stage, Some(backstage_core::AnimId::from_raw(1)), 0.5, false);
    assert!(stage.next_frame().pixels != paused, "an animation the root doesn't have keys nothing");

    transport(&mut stage, None, 0.5, true);
    let first = stage.next_frame().pixels;
    std::thread::sleep(Duration::from_millis(100));
    assert!(stage.next_frame().pixels != first, "a playing stage moves");
}

#[test]
fn hidden_nodes_leave_the_editor_stage_and_undo_brings_them_back() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    stage.load_sample();
    resize(&mut stage, 550, 400);
    let green = [0x6a as f32 / 255.0, 0xbf as f32 / 255.0, 0x4b as f32 / 255.0, 1.0];
    // The ground strip, inside the editor's framing of the stage.
    let ground = |stage: &mut StageHarness| stage.next_frame().pixel(275, 360);
    assert!(color_close(ground(&mut stage), green), "{:?}", ground(&mut stage));

    let hide = Entry::Do(Command::SetNodeFlags {
        comp: ids::STAGE,
        node: ids::GROUND,
        flags: backstage_core::NodeFlags { hidden: true, ..Default::default() },
    });
    assert!(matches!(stage.submit(1, hide), ToTools::Committed { seq: 1, .. }));
    assert!(color_close(ground(&mut stage), [1.0; 4]), "hidden: the white stage shows");

    assert!(matches!(stage.submit(2, Entry::Undo), ToTools::Committed { seq: 2, .. }));
    assert!(color_close(ground(&mut stage), green), "shown again");
}

#[test]
fn rejected_entries_use_no_sequence_number() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    assert!(matches!(stage.submit(1, Entry::Undo), ToTools::Rejected { request: 1, .. }), "before Load");
    stage.load_sample();
    let bad = Entry::Do(Command::RemoveNode { comp: ids::STAGE, node: ids::STAGE_ROOT });
    let ToTools::Rejected { request: 2, reason } = stage.submit(2, bad) else { panic!("expected Rejected") };
    assert!(reason.contains("root node"), "{reason}");
    assert!(matches!(stage.submit(3, set_background(Color::BLACK)), ToTools::Committed { seq: 1, .. }));
}

#[test]
fn bad_document_is_reported_and_fails() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    stage.send(&ToStage::Load { base: Snapshot { files: vec![] }, log: vec![] });
    let log = stage.recv_until(|m| match m {
        ToTools::Log(line) => Some(line.clone()),
        _ => None,
    });
    assert!(log.contains("project.ron"), "{log}");
    assert!(!stage.wait_exit().success());
}

#[test]
fn handshake_reports_adapter() {
    let mut stage = StageHarness::spawn();
    assert!(!stage.handshake().is_empty());
}

#[test]
fn streams_frames_after_resize() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    stage.load_sample();
    resize(&mut stage, 320, 200);
    let frame = stage.next_frame();
    assert_eq!((frame.width, frame.height), (320, 200));
    assert_eq!(frame.pixel(160, 100)[3], 255, "opaque");
    assert!(color_close(frame.pixel(2, 2), pasteboard()), "corner {:?}", frame.pixel(2, 2));
    // Stage interior near its top-left corner is white (no quad goes there).
    assert!(color_close(frame.pixel(60, 45), [1.0; 4]), "stage {:?}", frame.pixel(60, 45));
}

#[test]
fn resize_starts_a_new_generation_and_drops_the_old_ring() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    stage.load_sample();
    let first = resize(&mut stage, 320, 200);
    stage.next_frame();
    let second = resize(&mut stage, 400, 300);
    assert!(second > first);
    let frame = stage.next_frame();
    assert_eq!((frame.width, frame.height), (400, 300));
    let files = stage.wait_for_ring_files(|f| f.len() == 1);
    assert_eq!(files.len(), 1, "old ring not removed: {files:?}");
    assert!(files[0].ends_with(&format!("-{second}.frames")), "{files:?}");
}

#[test]
fn same_size_resize_is_ignored() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    resize(&mut stage, 320, 200);
    stage.send(&ToStage::Resize { width: 320, height: 200, scale: 1.0 });
    for _ in 0..10 {
        assert!(!matches!(stage.recv(), ToTools::Surface { .. }));
    }
}

#[test]
fn shutdown_exits_cleanly_and_removes_files() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    stage.load_sample();
    resize(&mut stage, 64, 64);
    stage.next_frame();
    stage.send(&ToStage::Shutdown);
    assert!(stage.wait_exit().success());
    assert_eq!(stage.ring_files(), Vec::<String>::new());
}

#[test]
fn closing_the_socket_stops_the_stage() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    resize(&mut stage, 64, 64);
    stage.close_socket();
    assert!(stage.wait_exit().success(), "tools going away is a clean shutdown");
    assert_eq!(stage.ring_files(), Vec::<String>::new());
}

#[test]
fn protocol_mismatch_is_an_error() {
    let mut stage = StageHarness::spawn();
    stage.send(&ToStage::Hello { version: 9999 });
    assert!(!stage.wait_exit().success());
}

#[test]
fn pointer_draws_crosshair() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    stage.load_sample();
    resize(&mut stage, 320, 200);
    stage.send(&move_to(40.0, 30.0));
    let deadline = Instant::now() + common::TIMEOUT;
    while Instant::now() < deadline {
        if color_close(stage.next_frame().pixel(40, 30), CROSSHAIR) {
            return;
        }
    }
    panic!("crosshair never appeared");
}

fn move_to(x: f32, y: f32) -> ToStage {
    ToStage::Pointer(PointerEvent::Move(PointerAt::new(x, y)))
}

/// Paused and untouched, the stage draws nothing; input is drawn right
/// away, in one frame, without waiting for a tick.
#[test]
fn a_paused_stage_idles_until_input_arrives() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    stage.load_sample();
    resize(&mut stage, 320, 200);
    transport(&mut stage, None, 0.0, false);
    // The barrier may leave one more frame in flight; let it land.
    stage.next_frame();
    stage.frames_for(Duration::from_millis(200));
    stage.assert_no_frame_for(Duration::from_millis(300));

    stage.send(&move_to(40.0, 30.0));
    assert!(color_close(stage.next_frame().pixel(40, 30), CROSSHAIR), "the move, in the next frame");
    stage.assert_no_frame_for(Duration::from_millis(100));

    stage.send(&ToStage::Pointer(PointerEvent::Leave));
    assert!(!color_close(stage.next_frame().pixel(40, 30), CROSSHAIR), "gone after Leave");
    stage.assert_no_frame_for(Duration::from_millis(100));
}

/// Pointer → frame latency on the stage side: time from sending `Pointer`
/// until a published frame shows the crosshair there. Excludes the GTK upload
/// and compositor. Uses the real GPU unless BACKSTAGE_WGPU_FALLBACK=1.
///
/// `cargo test -p backstage_stage --test stage_protocol -- --ignored --nocapture latency`
#[test]
#[ignore]
fn latency_probe() {
    let fallback = std::env::var(backstage_render::FALLBACK_ENV).is_ok_and(|v| v == "1");
    let mut stage = StageHarness::spawn_with_fallback(fallback);
    let adapter = stage.handshake();
    stage.load_sample();
    resize(&mut stage, 1280, 720);
    // Paused, as the editor's stage starts: only input makes frames.
    transport(&mut stage, None, 0.0, false);
    stage.next_frame();

    let mut samples = Vec::new();
    for i in 0..60u32 {
        // Alternate between two spots on the pasteboard, clear of the stage.
        let (x, y) = if i % 2 == 0 { (20.0, 20.0) } else { (1260.0, 700.0) };
        stage.send(&move_to(x, y));
        let sent = Instant::now();
        loop {
            if color_close(stage.next_frame().pixel(x as u32, y as u32), CROSSHAIR) {
                samples.push(sent.elapsed());
                break;
            }
            assert!(sent.elapsed() < Duration::from_secs(2), "crosshair never moved");
        }
    }
    samples.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!(
        "latency on {adapter}: min {:.1} ms, median {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        ms(samples[0]),
        ms(samples[samples.len() / 2]),
        ms(samples[samples.len() * 95 / 100]),
        ms(samples[samples.len() - 1]),
    );
}

/// Where stage point `p` is in a `size` viewport at scale 1, in the
/// editor's framing: what the tools process would report for it.
fn stage_to_view(size: (u32, u32), p: backstage_core::Vec2) -> (f32, f32) {
    let s = sample::bounce().settings;
    let framing = backstage_render::frame_stage(
        size,
        (s.stage_width as f32, s.stage_height as f32),
        1.0,
        backstage_render::Presentation::Editor,
        false,
    );
    framing.view.transform_point2(p).into()
}

/// The centre of everything `node` (a node of the sample's stage) draws at
/// time zero, in stage coordinates.
fn centre_of(node: backstage_core::NodeId) -> backstage_core::Vec2 {
    let project = sample::bounce();
    let scene = backstage_core::evaluate(&project, &backstage_core::RuntimeState::default(), Time::ZERO);
    let b = scene
        .items
        .iter()
        .filter(|i| i.node == node || i.instance.first() == Some(&node))
        .filter_map(backstage_render::pick::item_bounds)
        .reduce(backstage_render::pick::Rect::union)
        .unwrap();
    (b.min + b.max) / 2.0
}

const VIEW: (u32, u32) = (550, 400);

/// A click (press and release) at stage point `p`.
fn click(stage: &mut StageHarness, p: backstage_core::Vec2, modifiers: backstage_protocol::Modifiers) {
    let (x, y) = stage_to_view(VIEW, p);
    let at = PointerAt { x, y, modifiers };
    stage.send(&ToStage::Pointer(PointerEvent::Down(at)));
    stage.send(&ToStage::Pointer(PointerEvent::Up(at)));
}

/// The `Picked` messages the stage sent before answering a barrier.
fn picks(stage: &mut StageHarness) -> Vec<ToTools> {
    stage.send(&ToStage::Submit { request: u64::MAX, entry: Entry::Redo });
    let mut picked = Vec::new();
    stage.recv_until(|m| match m {
        ToTools::Picked { .. } => {
            picked.push(m.clone());
            None
        }
        ToTools::Rejected { request: u64::MAX, .. } => Some(()),
        _ => None,
    });
    picked
}

fn picked(nodes: Vec<backstage_core::NodeId>, mode: backstage_protocol::PickMode) -> Vec<ToTools> {
    vec![ToTools::Picked { comp: ids::STAGE, nodes, mode }]
}

fn ready_to_click() -> StageHarness {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    stage.load_sample();
    resize(&mut stage, VIEW.0, VIEW.1);
    transport(&mut stage, None, 0.0, false);
    stage
}

#[test]
fn clicks_pick_toggle_and_clear_the_selection() {
    use backstage_protocol::{Modifiers, PickMode};
    let shift = Modifiers { shift: true, ..Default::default() };
    let mut stage = ready_to_click();
    let (ground, ball) = (centre_of(ids::GROUND), centre_of(ids::FREE_BALL));

    click(&mut stage, ground, Modifiers::default());
    assert_eq!(picks(&mut stage), picked(vec![ids::GROUND], PickMode::Replace));
    click(&mut stage, ground, Modifiers::default());
    assert_eq!(picks(&mut stage), [], "already selected: unchanged");

    click(&mut stage, ball, shift);
    assert_eq!(picks(&mut stage), picked(vec![ids::FREE_BALL], PickMode::Toggle));
    click(&mut stage, ground, Modifiers::default());
    assert_eq!(picks(&mut stage), [], "part of a multi-selection: kept for a drag");
    click(&mut stage, ground, shift);
    assert_eq!(picks(&mut stage), picked(vec![ids::GROUND], PickMode::Toggle), "toggled out");

    let empty = backstage_core::Vec2::new(10.0, 10.0);
    click(&mut stage, empty, shift);
    assert_eq!(picks(&mut stage), [], "Shift on nothing: unchanged");
    click(&mut stage, empty, Modifiers::default());
    assert_eq!(picks(&mut stage), picked(vec![], PickMode::Replace), "cleared");
    click(&mut stage, empty, Modifiers::default());
    assert_eq!(picks(&mut stage), [], "already empty");
}

#[test]
fn locked_nodes_let_clicks_through() {
    use backstage_protocol::{Modifiers, PickMode};
    let mut stage = ready_to_click();
    click(&mut stage, centre_of(ids::FREE_BALL), Modifiers::default());
    assert_eq!(picks(&mut stage), picked(vec![ids::FREE_BALL], PickMode::Replace));

    let lock = Entry::Do(Command::SetNodeFlags {
        comp: ids::STAGE,
        node: ids::GROUND,
        flags: backstage_core::NodeFlags { locked: true, ..Default::default() },
    });
    assert!(matches!(stage.submit(1, lock), ToTools::Committed { .. }));
    click(&mut stage, centre_of(ids::GROUND), Modifiers::default());
    assert_eq!(picks(&mut stage), picked(vec![], PickMode::Replace), "nothing under the locked ground");
}

/// The pixel on the top edge of the box around stage rect `min..max`.
fn top_edge(min: backstage_core::Vec2, max: backstage_core::Vec2) -> (u32, u32) {
    let (x0, y0) = stage_to_view(VIEW, min);
    let (x1, _) = stage_to_view(VIEW, max);
    (((x0 + x1) / 2.0) as u32, y0.floor() as u32)
}

#[test]
fn the_selection_and_hover_get_boxes_without_waiting_for_the_editor() {
    use backstage_protocol::Modifiers;
    use backstage_render::{HOVER, SELECTION};
    let mut stage = ready_to_click();
    // The ground strip covers stage y 340..400 across the whole stage.
    let edge = top_edge(backstage_core::Vec2::new(0.0, 340.0), backstage_core::Vec2::new(550.0, 400.0));
    let ground = centre_of(ids::GROUND);

    // The edge row is the ground's anti-aliased top: half green, half white.
    let under = stage.next_frame().pixel(edge.0, edge.1).map(|c| c as f32 / 255.0);
    let (x, y) = stage_to_view(VIEW, ground);
    stage.send(&ToStage::Pointer(PointerEvent::Move(PointerAt::new(x, y))));
    let hovered = stage.next_frame().pixel(edge.0, edge.1);
    assert!(color_close(hovered, mix(HOVER, under)), "hover box over {under:?}: {hovered:?}");

    // The stage shows the pick at once; the editor's echo comes later.
    click(&mut stage, ground, Modifiers::default());
    assert!(color_close(stage.next_frame().pixel(edge.0, edge.1), SELECTION));

    stage.send(&ToStage::Selection { comp: ids::STAGE, nodes: vec![] });
    stage.send(&ToStage::Pointer(PointerEvent::Leave));
    let plain = stage.next_frame().pixel(edge.0, edge.1);
    assert!(color_close(plain, under), "no box: {plain:?}");
}

/// `top` (straight alpha) blended over opaque `under`.
fn mix(top: [f32; 4], under: [f32; 4]) -> [f32; 4] {
    let a = top[3];
    [0, 1, 2]
        .map(|i| top[i] * a + under[i] * (1.0 - a))
        .into_iter()
        .chain([1.0])
        .collect::<Vec<_>>()
        .try_into()
        .unwrap()
}

/// The `Framing` the stage reports next.
fn framing(stage: &mut StageHarness) -> ((f32, f32), f32) {
    stage.recv_until(|m| match m {
        ToTools::Framing { origin, scale } => Some((*origin, *scale)),
        _ => None,
    })
}

#[test]
fn the_stage_reports_where_it_sits_in_logical_pixels() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
    stage.load_sample();
    resize(&mut stage, VIEW.0, VIEW.1);
    let (origin, scale) = framing(&mut stage);
    let (x, y) = stage_to_view(VIEW, backstage_core::Vec2::ZERO);
    assert_eq!(origin, (x, y), "the stage's top-left corner");
    let (x1, _) = stage_to_view(VIEW, backstage_core::Vec2::new(550.0, 0.0));
    assert!((scale - (x1 - x) / 550.0).abs() < 1e-5, "{scale}");

    // The same section at scale 2: twice the pixels, the same logical
    // layout, so nothing new to report before its first frame.
    stage.send(&ToStage::Resize { width: VIEW.0 * 2, height: VIEW.1 * 2, scale: 2.0 });
    let generation = stage.recv_until(|m| match m {
        ToTools::Surface { generation, .. } => Some(*generation),
        _ => None,
    });
    let mut reported = Vec::new();
    stage.recv_until(|m| match m {
        ToTools::Framing { .. } => {
            reported.push(m.clone());
            None
        }
        ToTools::FrameReady { generation: g, .. } if *g == generation => Some(()),
        _ => None,
    });
    assert_eq!(reported, [], "unchanged");

    // A new stage size moves it.
    let settings = ProjectSettings { stage_width: 275, ..sample::bounce().settings };
    assert!(matches!(stage.submit(1, Entry::Do(Command::SetSettings(settings))), ToTools::Committed { .. }));
    assert_ne!(framing(&mut stage), (origin, scale));
}
