//! Golden-image regression tests: the sample project (`sample::bounce()`)
//! evaluated at fixed times and rendered on the software adapter.
//!
//! `BACKSTAGE_BLESS=1 cargo test -p backstage_render --test golden` rewrites
//! the reference images in `tests/golden/`. On a mismatch, the actual image
//! and a diff are written to `target/golden-failures/`.

use backstage_core::{RuntimeState, Time, evaluate, sample};
use backstage_render::{FALLBACK_ENV, Frame, HeadlessGpu, OFFSCREEN_FORMAT, OffscreenTarget, Renderer};
use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};

/// A channel may differ by this much (rasterizer rounding)…
const CHANNEL_TOLERANCE: u8 = 2;
/// …and at most this many pixels may exceed it. Kept small on purpose: a
/// 1px line moving is only ~100 pixels, and a percentage budget hid that.
/// Goldens come from lavapipe; after a Mesa upgrade, review the diff images
/// and re-bless.
const MAX_BAD_PIXELS: usize = 16;

/// Renders the sample at `at` into a `size` image.
fn render(size: (u32, u32), at: Time, scale: f32, pointer: Option<(f32, f32)>) -> RgbaImage {
    // SAFETY: set before any threads of ours read the environment.
    unsafe { std::env::set_var(FALLBACK_ENV, "1") };
    let gpu = pollster::block_on(HeadlessGpu::new("golden test")).expect("software adapter (lavapipe)");
    let mut renderer = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
    let mut target = OffscreenTarget::new(&gpu.device, size);
    let project = sample::bounce();
    let scene = evaluate(&project, &RuntimeState::default(), at);
    let frame = Frame { project: &project, scene: &scene, scale, pointer };
    let stride = target.stride() as usize;
    let mut image = RgbaImage::new(size.0, size.1);
    target
        .render_and_read(&gpu, &mut renderer, &frame, |pixels| {
            for (y, row) in pixels.chunks(stride).enumerate() {
                let row = &row[..size.0 as usize * 4];
                let start = y * size.0 as usize * 4;
                image.as_mut()[start..start + row.len()].copy_from_slice(row);
            }
        })
        .unwrap();
    image
}

fn check(name: &str, actual: RgbaImage) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let golden_path = dir.join(format!("{name}.png"));
    if std::env::var("BACKSTAGE_BLESS").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(&dir).unwrap();
        actual.save(&golden_path).unwrap();
        eprintln!("blessed {}", golden_path.display());
        return;
    }
    let golden = image::open(&golden_path)
        .unwrap_or_else(|e| panic!("{}: {e} (run with BACKSTAGE_BLESS=1 to create)", golden_path.display()))
        .to_rgba8();
    assert_eq!(golden.dimensions(), actual.dimensions(), "{name}: size changed");

    let mut diff = RgbaImage::new(actual.width(), actual.height());
    let mut bad = 0usize;
    for ((g, a), d) in golden.pixels().zip(actual.pixels()).zip(diff.pixels_mut()) {
        let worst = g.0.iter().zip(a.0).map(|(g, a)| g.abs_diff(a)).max().unwrap();
        if worst > CHANNEL_TOLERANCE {
            bad += 1;
            *d = Rgba([255, 0, 255, 255]);
        } else {
            *d = Rgba([a[0] / 4, a[1] / 4, a[2] / 4, 255]);
        }
    }
    if bad > MAX_BAD_PIXELS {
        let ratio = bad as f64 / (actual.width() * actual.height()) as f64;
        let out = failures_dir();
        actual.save(out.join(format!("{name}.actual.png"))).unwrap();
        diff.save(out.join(format!("{name}.diff.png"))).unwrap();
        panic!("{name}: {bad} pixels ({:.3}%) differ; see {}", ratio * 100.0, out.display());
    }
}

fn failures_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/golden-failures");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn secs(num: i64, den: i64) -> Time {
    Time::from_ratio(num, den)
}

#[test]
fn sample_at_start() {
    check("sample_t0_550x400_1x", render((550, 400), Time::ZERO, 1.0, None));
}

#[test]
fn sample_mid_bounce_at_1_5x_with_pointer() {
    check("sample_t0_5_800x600_1_5x_pointer", render((800, 600), secs(1, 2), 1.5, Some((120.0, 80.0))));
}

#[test]
fn sample_blink() {
    // At 1.6 s the eye flipbook shows its closed drawing.
    check("sample_t1_6_blink_550x400_1x", render((550, 400), secs(8, 5), 1.0, None));
}

#[test]
fn tiny_viewport() {
    check("tiny_64x64", render((64, 64), Time::ZERO, 1.0, None));
}
