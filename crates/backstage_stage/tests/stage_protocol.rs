//! Integration tests: the real stage binary on the software adapter, driven
//! over the real protocol.

mod common;

use backstage_core::sample::{self, ids};
use backstage_core::{Color, Command, Document, Entry, ProjectSettings, Time};
use backstage_protocol::{Snapshot, ToStage, ToTools};
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
    assert!(matches!(stage.submit(u64::MAX, Entry::Redo), ToTools::Rejected { .. }));
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
    std::thread::sleep(Duration::from_millis(100));
    assert!(stage.next_frame().pixels == paused, "a paused stage draws the same frame");

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
    stage.send(&ToStage::Pointer(Some((40.0, 30.0))));
    let deadline = Instant::now() + common::TIMEOUT;
    while Instant::now() < deadline {
        if color_close(stage.next_frame().pixel(40, 30), CROSSHAIR) {
            return;
        }
    }
    panic!("crosshair never appeared");
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
    stage.next_frame();

    let mut samples = Vec::new();
    for i in 0..60u32 {
        // Alternate between two spots on the pasteboard, clear of the stage.
        let (x, y) = if i % 2 == 0 { (20.0, 20.0) } else { (1260.0, 700.0) };
        stage.send(&ToStage::Pointer(Some((x, y))));
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
