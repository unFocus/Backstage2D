//! Headless smoke test: the player opens a Wayland window inside a headless
//! cage compositor, presents 30 frames on the software adapter, and exits 0.
//! Covers surface creation and the present loop without touching the desktop.
//!
//! `cargo test -p backstage_player --test player_smoke -- --ignored`

use std::process::Command;

#[test]
#[ignore = "needs cage; run with --ignored"]
fn plays_frames_in_a_headless_compositor() {
    if Command::new("cage").arg("-v").output().is_err() {
        eprintln!("skipping: cage is not installed");
        return;
    }
    let sample = concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/bounce.bs2d");
    let output = Command::new("timeout")
        .args(["-k5", "60", "cage", "--"])
        .arg(env!("CARGO_BIN_EXE_backstage_player"))
        .args([sample, "--frames", "30"])
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env("WLR_BACKENDS", "headless")
        .env("WLR_RENDERER", "pixman")
        .env("WLR_LIBINPUT_NO_DEVICES", "1")
        .env("BACKSTAGE_WGPU_FALLBACK", "1")
        .output()
        .expect("running cage");
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "player failed ({}):\n{log}", output.status);
    assert!(log.contains("backstage_player: using"), "no adapter line:\n{log}");
}
