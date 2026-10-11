//! Integration tests: the real stage binary on the software adapter, driven
//! over the real protocol.

mod common;

use backstage_core::sample::{self, ids};
use backstage_core::{Color, Command, Document, Entry, ProjectSettings, Time};
use backstage_protocol::{PointerAt, PointerEvent, Snapshot, ToStage, ToTools};
use backstage_render::{HOVER, PASTEBOARD};
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
fn moving_onto_a_node_draws_its_hover_box() {
    let mut stage = ready_to_click();
    let (x, y) = stage_to_view(VIEW, centre_of(ids::GROUND));
    stage.send(&move_to(x, y));
    let (ex, ey) = hover_edge(VIEW);
    let deadline = Instant::now() + common::TIMEOUT;
    while Instant::now() < deadline {
        if hovered(stage.next_frame().pixel(ex, ey)) {
            return;
        }
    }
    panic!("the hover box never appeared");
}

/// The pixel on the top edge of the ground's hover box (2 px above the
/// strip, on white) in a `size` viewport at scale 1.
fn hover_edge(size: (u32, u32)) -> (u32, u32) {
    let (x, y) = stage_to_view(size, backstage_core::Vec2::new(275.0, 340.0));
    (x as u32, (y - 2.0).floor() as u32)
}

fn hovered(pixel: [u8; 4]) -> bool {
    color_close(pixel, mix(HOVER, [1.0; 4]))
}

fn move_to(x: f32, y: f32) -> ToStage {
    ToStage::Pointer(PointerEvent::Move(PointerAt::new(x, y)))
}

/// Paused and untouched, the stage draws nothing; input is drawn right
/// away, in one frame, without waiting for a tick.
#[test]
fn a_paused_stage_idles_until_input_arrives() {
    let mut stage = ready_to_click();
    // The barrier may leave one more frame in flight; let it land.
    stage.next_frame();
    stage.frames_for(Duration::from_millis(200));
    stage.assert_no_frame_for(Duration::from_millis(300));

    let (x, y) = stage_to_view(VIEW, centre_of(ids::GROUND));
    let (ex, ey) = hover_edge(VIEW);
    stage.send(&move_to(x, y));
    assert!(hovered(stage.next_frame().pixel(ex, ey)), "the move, in the next frame");
    stage.assert_no_frame_for(Duration::from_millis(100));

    stage.send(&ToStage::Pointer(PointerEvent::Leave));
    assert!(!hovered(stage.next_frame().pixel(ex, ey)), "gone after Leave");
    stage.assert_no_frame_for(Duration::from_millis(100));
}

/// Pointer → frame latency on the stage side: time from sending `Pointer`
/// until a published frame shows its effect (the ground's hover box
/// appearing or going). Excludes the GTK upload
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

    // Alternate onto the ground and off it, onto the pasteboard: its hover
    // box comes and goes (picking and drawing both count).
    let size = (1280, 720);
    let ground = stage_to_view(size, centre_of(ids::GROUND));
    let (ex, ey) = hover_edge(size);
    let mut samples = Vec::new();
    for i in 0..60u32 {
        let on = i % 2 == 0;
        let (x, y) = if on { ground } else { (20.0, 20.0) };
        stage.send(&move_to(x, y));
        let sent = Instant::now();
        loop {
            if hovered(stage.next_frame().pixel(ex, ey)) == on {
                samples.push(sent.elapsed());
                break;
            }
            assert!(sent.elapsed() < Duration::from_secs(2), "the hover box never changed");
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
    let b = bounds_of(node);
    (b.min + b.max) / 2.0
}

/// The bounds of everything `node` draws at time zero.
fn bounds_of(node: backstage_core::NodeId) -> backstage_render::pick::Rect {
    let project = sample::bounce();
    let scene = backstage_core::evaluate(&project, &backstage_core::RuntimeState::default(), Time::ZERO);
    scene
        .items
        .iter()
        .filter(|i| i.node == node || i.instance.first() == Some(&node))
        .filter_map(backstage_render::pick::item_bounds)
        .reduce(backstage_render::pick::Rect::union)
        .unwrap()
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

/// A pixel on the top edge of the box around stage rect `min..max`, drawn
/// 2 px outside it: a quarter of the way along, clear of the handles.
fn top_edge(min: backstage_core::Vec2, max: backstage_core::Vec2) -> (u32, u32) {
    let (x0, y0) = stage_to_view(VIEW, min);
    let (x1, _) = stage_to_view(VIEW, max);
    ((x0 + (x1 - x0) / 4.0) as u32, (y0 - 2.0).floor() as u32)
}

#[test]
fn the_selection_and_hover_get_boxes_without_waiting_for_the_editor() {
    use backstage_protocol::Modifiers;
    use backstage_render::{HOVER, SELECTION};
    let mut stage = ready_to_click();
    // The ground strip covers stage y 340..400 across the whole stage.
    let edge = top_edge(backstage_core::Vec2::new(0.0, 340.0), backstage_core::Vec2::new(550.0, 400.0));
    let ground = centre_of(ids::GROUND);

    // The box's row is just above the ground, on white.
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
    // A frame can come between the two (the ground hovered, no longer
    // selected): read one drawn after both, past a barrier.
    picks(&mut stage);
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

/// A drag from stage point `from` to `to`, with moves in between, as GTK
/// reports one.
fn drag(
    stage: &mut StageHarness,
    from: backstage_core::Vec2,
    to: backstage_core::Vec2,
    modifiers: backstage_protocol::Modifiers,
) {
    let at = |p| {
        let (x, y) = stage_to_view(VIEW, p);
        PointerAt { x, y, modifiers }
    };
    stage.send(&ToStage::Pointer(PointerEvent::Down(at(from))));
    for t in [0.25, 0.5, 1.0] {
        stage.send(&ToStage::Pointer(PointerEvent::Move(at(from.lerp(to, t)))));
    }
    stage.send(&ToStage::Pointer(PointerEvent::Up(at(to))));
}

#[test]
fn a_marquee_selects_what_it_touches() {
    use backstage_core::Vec2;
    use backstage_protocol::{Modifiers, PickMode};
    let shift = Modifiers { shift: true, ..Default::default() };
    let mut stage = ready_to_click();
    click(&mut stage, centre_of(ids::GROUND), Modifiers::default());
    picks(&mut stage);

    // From an empty corner to just past the free ball's centre.
    let corner = Vec2::new(300.0, 10.0);
    drag(&mut stage, corner, centre_of(ids::FREE_BALL), Modifiers::default());
    assert_eq!(
        picks(&mut stage),
        [picked(vec![], PickMode::Replace), picked(vec![ids::FREE_BALL], PickMode::Replace)].concat(),
        "cleared on the press, then the marquee"
    );

    // Shift adds, and its press clears nothing.
    let synced = centre_of(ids::SYNCED_BALL);
    drag(&mut stage, Vec2::new(10.0, 10.0), synced, shift);
    assert_eq!(picks(&mut stage), picked(vec![ids::SYNCED_BALL], PickMode::Add));
    drag(&mut stage, Vec2::new(10.0, 10.0), synced, shift);
    assert_eq!(picks(&mut stage), [], "nothing new to add");

    // Too short to be a marquee: just a click on empty stage. It's inside
    // the free ball's bounds, off the ball, so a marquee would pick it.
    let corner = bounds_of(ids::FREE_BALL).min + Vec2::splat(2.0);
    drag(&mut stage, corner, corner + Vec2::splat(1.0), Modifiers::default());
    assert_eq!(picks(&mut stage), picked(vec![], PickMode::Replace), "only the press's clear");
}

#[test]
fn a_marquee_is_drawn_while_dragging_and_can_be_cancelled() {
    use backstage_core::Vec2;
    use backstage_render::MARQUEE;
    let mut stage = ready_to_click();
    let inside = stage_to_view(VIEW, Vec2::new(60.0, 60.0));
    let pixel = |stage: &mut StageHarness| stage.next_frame().pixel(inside.0 as u32, inside.1 as u32);
    let white = pixel(&mut stage);

    let at = |p: Vec2| {
        let (x, y) = stage_to_view(VIEW, p);
        PointerAt::new(x, y)
    };
    stage.send(&ToStage::Pointer(PointerEvent::Down(at(Vec2::new(10.0, 10.0)))));
    stage.send(&ToStage::Pointer(PointerEvent::Move(at(Vec2::new(120.0, 120.0)))));
    let tinted = mix(MARQUEE, white.map(|c| c as f32 / 255.0));
    // Each message may get a frame of its own: wait for the last one's.
    let shows = |stage: &mut StageHarness, color| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if color_close(pixel(stage), color) {
                return true;
            }
        }
        false
    };
    assert!(shows(&mut stage, tinted), "the marquee's fill");

    stage.send(&ToStage::CancelGesture);
    assert!(shows(&mut stage, white.map(|c| c as f32 / 255.0)), "gone");
    stage.send(&ToStage::Pointer(PointerEvent::Up(at(Vec2::new(120.0, 120.0)))));
    assert_eq!(picks(&mut stage), [], "a cancelled marquee picks nothing");
}

/// The picks and commits the stage sent before answering a barrier: an
/// entry always rejected (a redo could commit, after an undo).
fn edits(stage: &mut StageHarness) -> Vec<ToTools> {
    let never = Entry::Do(Command::RemoveNode { comp: ids::STAGE, node: ids::STAGE_ROOT });
    stage.send(&ToStage::Submit { request: u64::MAX, entry: never });
    let mut seen = Vec::new();
    stage.recv_until(|m| match m {
        ToTools::Picked { .. } | ToTools::Committed { .. } => {
            seen.push(m.clone());
            None
        }
        ToTools::Rejected { request: u64::MAX, .. } => Some(()),
        _ => None,
    });
    seen
}

/// The single drag commit among `edits`.
fn drag_commit(edits: &[ToTools]) -> Command {
    let commits: Vec<_> = edits
        .iter()
        .filter_map(|m| match m {
            ToTools::Committed { request: None, entry: Entry::Do(command), .. } => Some(command.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(commits.len(), 1, "{edits:?}");
    commits[0].clone()
}

/// Waits (up to 5 s of frames) for the pixel at stage point `p` to satisfy
/// `ok`.
fn frames_show(stage: &mut StageHarness, p: backstage_core::Vec2, ok: impl Fn([u8; 4]) -> bool) -> bool {
    let (x, y) = stage_to_view(VIEW, p);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if ok(stage.next_frame().pixel(x as u32, y as u32)) {
            return true;
        }
    }
    false
}

fn green(p: [u8; 4]) -> bool {
    color_close(p, [0x6a as f32 / 255.0, 0xbf as f32 / 255.0, 0x4b as f32 / 255.0, 1.0])
}

fn press_and_move(stage: &mut StageHarness, from: backstage_core::Vec2, to: backstage_core::Vec2) {
    let at = |p| {
        let (x, y) = stage_to_view(VIEW, p);
        PointerAt::new(x, y)
    };
    stage.send(&ToStage::Pointer(PointerEvent::Down(at(from))));
    stage.send(&ToStage::Pointer(PointerEvent::Move(at(from.lerp(to, 0.5)))));
    stage.send(&ToStage::Pointer(PointerEvent::Move(at(to))));
}

fn release(stage: &mut StageHarness, p: backstage_core::Vec2) {
    let (x, y) = stage_to_view(VIEW, p);
    stage.send(&ToStage::Pointer(PointerEvent::Up(PointerAt::new(x, y))));
}

#[test]
fn dragging_a_node_commits_one_undoable_move() {
    use backstage_core::Vec2;
    use backstage_protocol::Modifiers;
    let mut stage = ready_to_click();
    let ground = centre_of(ids::GROUND);
    drag(&mut stage, ground, ground + Vec2::new(30.0, 0.0), Modifiers::default());
    let edits = edits(&mut stage);
    assert!(matches!(&edits[0], ToTools::Picked { nodes, .. } if nodes == &[ids::GROUND]), "{edits:?}");
    let command = drag_commit(&edits);
    let Command::SetRest { node: ids::GROUND, rest, .. } = &command else { panic!("{command:?}") };
    assert!(rest.transform.position.abs_diff_eq(Vec2::new(30.0, 0.0), 1e-2), "{rest:?}");

    // One entry, so one undo takes it all back.
    assert!(matches!(stage.submit(1, Entry::Undo), ToTools::Committed { .. }));
    let undone = Document::replay(sample::bounce(), &[Entry::Do(command), Entry::Undo]).unwrap();
    assert_eq!(undone.project(), &sample::bounce());

    // A press and release without moving changes nothing.
    click(&mut stage, ground, Modifiers::default());
    assert_eq!(self::edits(&mut stage), [], "already selected, not moved");
}

/// The centre of what `node` draws at `secs` on the stage's main animation.
fn centre_at(node: backstage_core::NodeId, secs: f64) -> backstage_core::Vec2 {
    let project = sample::bounce();
    let scene = backstage_core::evaluate(
        &project,
        &backstage_core::RuntimeState::default(),
        Time::from_secs_f64(secs),
    );
    let b = scene
        .items
        .iter()
        .filter(|i| i.node == node || i.instance.first() == Some(&node))
        .filter_map(backstage_render::pick::item_bounds)
        .reduce(backstage_render::pick::Rect::union)
        .unwrap();
    (b.min + b.max) / 2.0
}

#[test]
fn dragging_an_animated_node_keys_it_at_the_playhead() {
    use backstage_core::{Property, Vec2};
    use backstage_protocol::Modifiers;
    let mut stage = ready_to_click();
    transport(&mut stage, None, 1.0, false);
    // The synced ball's X is animated on `main`; its Y isn't.
    let ball = centre_at(ids::SYNCED_BALL, 1.0);
    drag(&mut stage, ball, ball + Vec2::new(20.0, 10.0), Modifiers::default());
    let command = drag_commit(&edits(&mut stage));
    let Command::Batch(parts) = &command else { panic!("{command:?}") };
    assert!(
        matches!(&parts[..], [
            Command::SetKey { node: ids::SYNCED_BALL, property: Property::X, key, .. },
            Command::SetRest { node: ids::SYNCED_BALL, .. },
        ] if key.at == Time::from_secs(1)),
        "a key for X at 1 s, the rest for Y: {parts:?}"
    );
}

#[test]
fn a_drag_previews_live_and_can_be_cancelled() {
    use backstage_core::Vec2;
    let mut stage = ready_to_click();
    let ground = centre_of(ids::GROUND);
    // 60 up, the strip (stage y 340..400) covers y 300. Off to the side,
    // clear of the pointer's crosshair.
    let above = Vec2::new(100.0, 300.0);
    press_and_move(&mut stage, ground, ground - Vec2::new(0.0, 60.0));
    assert!(frames_show(&mut stage, above, green), "the ground moved up, before any commit");
    assert!(drag_commits(&edits(&mut stage)).is_empty(), "nothing committed while dragging");

    stage.send(&ToStage::CancelGesture);
    assert!(frames_show(&mut stage, above, |p| color_close(p, [1.0; 4])), "back in place");
    release(&mut stage, ground - Vec2::new(0.0, 60.0));
    assert!(drag_commits(&edits(&mut stage)).is_empty(), "a cancelled drag commits nothing");
}

#[test]
fn an_edit_committed_mid_drag_cancels_it() {
    use backstage_core::Vec2;
    let mut stage = ready_to_click();
    let ground = centre_of(ids::GROUND);
    press_and_move(&mut stage, ground, ground + Vec2::new(40.0, 0.0));
    assert!(matches!(stage.submit(1, set_background(Color::BLACK)), ToTools::Committed { .. }));
    release(&mut stage, ground + Vec2::new(40.0, 0.0));
    assert!(drag_commits(&edits(&mut stage)).is_empty(), "the drag was on the old document");
}

/// The stage's own commits among `edits`.
fn drag_commits(edits: &[ToTools]) -> Vec<&ToTools> {
    edits.iter().filter(|m| matches!(m, ToTools::Committed { request: None, .. })).collect()
}

/// `node`'s handle box on the stage, for an unturned node: its bounds,
/// padded 2 logical px as the stage draws it.
fn handle_box(node: backstage_core::NodeId) -> backstage_render::pick::Rect {
    let fit =
        stage_to_view(VIEW, backstage_core::Vec2::X).0 - stage_to_view(VIEW, backstage_core::Vec2::ZERO).0;
    let pad = backstage_core::Vec2::splat(2.0 / fit);
    let b = bounds_of(node);
    backstage_render::pick::Rect { min: b.min - pad, max: b.max + pad }
}

/// The node's rest transform after `command`.
fn transform_after(command: &Command, node: backstage_core::NodeId) -> backstage_core::Transform {
    let mut project = sample::bounce();
    command.apply(&mut project).unwrap();
    project.compositions[&ids::STAGE].nodes[&node].rest.transform
}

#[test]
fn an_edge_handle_scales_one_axis_about_the_opposite_edge() {
    use backstage_core::Vec2;
    use backstage_protocol::Modifiers;
    let mut stage = ready_to_click();
    click(&mut stage, centre_of(ids::GROUND), Modifiers::default());
    let b = handle_box(ids::GROUND);
    let right = Vec2::new(b.max.x, (b.min.y + b.max.y) / 2.0);
    drag(&mut stage, right, right + Vec2::new(50.0, 30.0), Modifiers::default());
    let command = drag_commit(&edits(&mut stage));
    let t = transform_after(&command, ids::GROUND);
    let expected = (b.max.x + 50.0 - b.min.x) / (b.max.x - b.min.x);
    assert!((t.scale.x - expected).abs() < 1e-3 && t.scale.y == 1.0, "only X: {t:?}");
    // The left edge stays where it was.
    let left = t.position.x + t.scale.x * b.min.x;
    assert!((left - b.min.x).abs() < 1e-2, "left edge at {left}");
}

#[test]
fn a_corner_with_shift_scales_evenly() {
    use backstage_core::Vec2;
    use backstage_protocol::Modifiers;
    let shift = Modifiers { shift: true, ..Default::default() };
    let mut stage = ready_to_click();
    click(&mut stage, centre_of(ids::FREE_BALL), Modifiers::default());
    let corner = handle_box(ids::FREE_BALL).max;
    drag(&mut stage, corner, corner + Vec2::new(40.0, 10.0), shift);
    let t = transform_after(&drag_commit(&edits(&mut stage)), ids::FREE_BALL);
    assert!((t.scale.x - t.scale.y).abs() < 1e-5 && t.scale.x > 1.0, "{t:?}");
}

#[test]
fn just_outside_a_corner_rotates_in_steps_with_shift() {
    use backstage_core::Vec2;
    use backstage_protocol::Modifiers;
    let shift = Modifiers { shift: true, ..Default::default() };
    let mut stage = ready_to_click();
    click(&mut stage, centre_of(ids::FREE_BALL), Modifiers::default());
    // Past the top-right corner, then swept around the pivot.
    let b = handle_box(ids::FREE_BALL);
    let outside = Vec2::new(b.max.x, b.min.y) + Vec2::new(8.0, -8.0);
    drag(&mut stage, outside, outside + Vec2::new(40.0, 60.0), shift);
    let t = transform_after(&drag_commit(&edits(&mut stage)), ids::FREE_BALL);
    assert!(t.rotation != 0.0 && (t.rotation / 15.0).fract() == 0.0, "a multiple of 15: {}", t.rotation);
    assert_eq!(
        t.position,
        sample::bounce().compositions[&ids::STAGE].nodes[&ids::FREE_BALL].rest.transform.position
    );
}

/// The next `Cursor` the stage asks for.
fn cursor(stage: &mut StageHarness) -> backstage_protocol::CursorKind {
    stage.recv_until(|m| match m {
        ToTools::Cursor(c) => Some(*c),
        _ => None,
    })
}

#[test]
fn the_cursor_shows_what_a_press_would_do() {
    use backstage_core::Vec2;
    use backstage_protocol::{CursorKind, Modifiers};
    let mut stage = ready_to_click();
    let over = |stage: &mut StageHarness, p: Vec2| {
        let (x, y) = stage_to_view(VIEW, p);
        stage.send(&move_to(x, y));
    };
    over(&mut stage, centre_of(ids::FREE_BALL));
    assert_eq!(cursor(&mut stage), CursorKind::Pointer, "a node to pick");
    click(&mut stage, centre_of(ids::FREE_BALL), Modifiers::default());
    over(&mut stage, centre_of(ids::FREE_BALL) + Vec2::new(1.0, 0.0));
    assert_eq!(cursor(&mut stage), CursorKind::Move, "selected: a press moves it");
    over(&mut stage, handle_box(ids::FREE_BALL).min);
    assert_eq!(cursor(&mut stage), CursorKind::ResizeNwse, "the top-left handle");
    over(&mut stage, handle_box(ids::FREE_BALL).min - Vec2::splat(8.0));
    assert_eq!(cursor(&mut stage), CursorKind::Rotate, "past the corner");
    over(&mut stage, Vec2::new(10.0, 10.0));
    assert_eq!(cursor(&mut stage), CursorKind::Default);
}

/// Selects both balls and returns their shared handle box.
fn both_balls(stage: &mut StageHarness) -> backstage_render::pick::Rect {
    use backstage_protocol::Modifiers;
    click(stage, centre_of(ids::FREE_BALL), Modifiers::default());
    click(stage, centre_of(ids::SYNCED_BALL), Modifiers { shift: true, ..Default::default() });
    // Padded 6 logical px, clear of each ball's own box.
    let fit =
        stage_to_view(VIEW, backstage_core::Vec2::X).0 - stage_to_view(VIEW, backstage_core::Vec2::ZERO).0;
    let pad = backstage_core::Vec2::splat(6.0 / fit);
    let (a, b) = (bounds_of(ids::FREE_BALL), bounds_of(ids::SYNCED_BALL));
    backstage_render::pick::Rect { min: a.min.min(b.min) - pad, max: a.max.max(b.max) + pad }
}

#[test]
fn a_shared_corner_scales_several_nodes_evenly() {
    use backstage_core::Vec2;
    use backstage_protocol::Modifiers;
    let mut stage = ready_to_click();
    let shared = both_balls(&mut stage);
    drag(&mut stage, shared.max, shared.max + Vec2::new(60.0, 20.0), Modifiers::default());
    let command = drag_commit(&edits(&mut stage));
    let (free, synced) =
        (transform_after(&command, ids::FREE_BALL), transform_after(&command, ids::SYNCED_BALL));
    assert!(free.scale.x > 1.0 && free.scale.x == free.scale.y, "{free:?}");
    assert_eq!(free.scale, synced.scale, "the same factor");
    let rest = |n| sample::bounce().compositions[&ids::STAGE].nodes[&n].rest.transform.position;
    assert_ne!(free.position, rest(ids::FREE_BALL), "moved away from the fixed corner");
}

#[test]
fn past_a_shared_corner_turns_several_nodes_together() {
    use backstage_core::Vec2;
    use backstage_protocol::Modifiers;
    let shift = Modifiers { shift: true, ..Default::default() };
    let mut stage = ready_to_click();
    let shared = both_balls(&mut stage);
    let outside = shared.max + Vec2::splat(8.0);
    drag(&mut stage, outside, outside + Vec2::new(-60.0, 60.0), shift);
    let command = drag_commit(&edits(&mut stage));
    for node in [ids::FREE_BALL, ids::SYNCED_BALL] {
        let t = transform_after(&command, node);
        assert!(t.rotation != 0.0 && (t.rotation / 15.0).fract() == 0.0, "{node:?}: {}", t.rotation);
    }
    assert_eq!(
        transform_after(&command, ids::FREE_BALL).rotation,
        transform_after(&command, ids::SYNCED_BALL).rotation,
        "the same turn"
    );
}
