//! Golden-image regression tests for the renderer, on the software adapter.
//!
//! `BACKSTAGE_BLESS=1 cargo test -p backstage_render --test golden` rewrites
//! the reference images in `tests/golden/`. On a mismatch, the actual image
//! and a diff are written to `target/golden-failures/`.

use backstage_render::{FALLBACK_ENV, HeadlessGpu, OFFSCREEN_FORMAT, OffscreenTarget, Renderer, TestScene};
use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};

/// A channel may differ by this much (rasterizer rounding)…
const CHANNEL_TOLERANCE: u8 = 2;
/// …and at most this many pixels may exceed it. Kept small on purpose: a
/// 1px line moving is only ~100 pixels, and a percentage budget hid that.
/// Goldens come from lavapipe; after a Mesa upgrade, review the diff images
/// and re-bless.
const MAX_BAD_PIXELS: usize = 16;

fn render(size: (u32, u32), scene: &TestScene) -> RgbaImage {
    // SAFETY: set before any threads of ours read the environment.
    unsafe { std::env::set_var(FALLBACK_ENV, "1") };
    let gpu = pollster::block_on(HeadlessGpu::new("golden test")).expect("software adapter (lavapipe)");
    let mut renderer = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
    let mut target = OffscreenTarget::new(&gpu.device, size);
    let stride = target.stride() as usize;
    let mut image = RgbaImage::new(size.0, size.1);
    target
        .render_and_read(&gpu, &mut renderer, scene, |pixels| {
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

#[test]
fn stage_at_1x() {
    check("stage_550x400_1x", render((550, 400), &TestScene { time: 0.0, scale: 1.0, pointer: None }));
}

#[test]
fn stage_at_1_5x_with_pointer() {
    let scene = TestScene { time: 2.5, scale: 1.5, pointer: Some((120.0, 80.0)) };
    check("stage_800x600_1_5x_pointer", render((800, 600), &scene));
}

#[test]
fn tiny_viewport() {
    check("tiny_64x64", render((64, 64), &TestScene { time: 1.0, scale: 1.0, pointer: None }));
}
