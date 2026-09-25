//! Integration tests: the real stage binary on the software adapter, driven
//! over the real protocol.

mod common;

use backstage_protocol::{ToStage, ToTools};
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

#[test]
fn handshake_reports_adapter() {
    let mut stage = StageHarness::spawn();
    assert!(!stage.handshake().is_empty());
}

#[test]
fn streams_frames_after_resize() {
    let mut stage = StageHarness::spawn();
    stage.handshake();
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
