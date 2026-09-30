//! End-to-end UI smoke test in a headless Wayland compositor (cage), so no
//! window appears on the desktop. The tools app runs in smoke mode
//! (`BACKSTAGE_SMOKE=1`): it waits for stage frames, kills the stage, waits
//! for frames from the restarted stage, and exits 0.
//!
//! `cargo test -p backstage_tools --test ui_smoke -- --ignored`

mod common;

use std::process::Command;
use std::time::Duration;

#[test]
#[ignore = "needs cage; run with --ignored"]
fn editor_shows_stage_frames_and_recovers_from_a_crash() {
    if Command::new("cage").arg("-v").output().is_err() {
        eprintln!("skipping: cage is not installed");
        return;
    }
    common::stage_binary();
    // Keep the editor's recovery directory out of the real state dir.
    let state = std::env::temp_dir().join(format!("backstage-ui-smoke-{}", std::process::id()));
    let output = Command::new("timeout")
        .arg("-k5")
        .arg(Duration::from_secs(60).as_secs().to_string())
        .args(["cage", "--"])
        .arg(env!("CARGO_BIN_EXE_backstage_tools"))
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env("WLR_BACKENDS", "headless")
        .env("WLR_RENDERER", "pixman")
        .env("WLR_LIBINPUT_NO_DEVICES", "1")
        .env("GSK_RENDERER", "cairo")
        .env("BACKSTAGE_WGPU_FALLBACK", "1")
        .env("BACKSTAGE_SMOKE", "1")
        .env("XDG_STATE_HOME", &state)
        .output()
        .expect("running cage");
    let _ = std::fs::remove_dir_all(&state);
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "smoke test failed ({}):\n{log}", output.status);
    assert!(log.contains("passed"), "no pass marker:\n{log}");
    assert!(log.contains(&format!("autosaving edits to {}", state.display())), "autosave is off:\n{log}");
}
