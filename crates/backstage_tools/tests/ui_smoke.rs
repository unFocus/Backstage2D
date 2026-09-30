//! End-to-end UI smoke test in a headless Wayland compositor (cage), so no
//! window appears on the desktop. The tools app runs in smoke mode
//! (`BACKSTAGE_SMOKE=1`, see `src/smoke.rs`): it waits for stage frames,
//! nudges, kills the stage once the edit is committed, checks the restarted
//! stage replayed it into the same document, saves (to a temporary copy of
//! the sample project), undoes the edit on the new stage, and exits 0
//! without cleaning up, like a crash. A second run in restore mode
//! (`BACKSTAGE_SMOKE=restore`) finds that unsaved undo, restores it, and
//! checks the stage loaded it with its history.
//!
//! `cargo test -p backstage_tools --test ui_smoke -- --ignored`

mod common;

use backstage_core::sample::ids::{GROUND, STAGE};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Runs the editor in headless cage in smoke mode `mode`, and returns
/// whether it passed and its stderr.
fn run_editor(mode: &str, state: &Path, project: &Path) -> (bool, String) {
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
        .env("BACKSTAGE_SMOKE", mode)
        .env("XDG_STATE_HOME", state)
        .env("BACKSTAGE_PROJECT", project)
        .output()
        .expect("running cage");
    (output.status.success(), String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
#[ignore = "needs cage; run with --ignored"]
fn edits_survive_stage_and_editor_crashes_in_the_real_editor() {
    if Command::new("cage").arg("-v").output().is_err() {
        eprintln!("skipping: cage is not installed");
        return;
    }
    common::stage_binary();
    // Keep the editor's recovery directories out of the real state dir.
    let state = std::env::temp_dir().join(format!("backstage-ui-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&state);
    let project_dir = state.join("smoke.bs2d");
    let sample = backstage_core::sample::bounce();
    backstage_core::save(&sample, &project_dir).unwrap();

    let (passed, log) = run_editor("1", &state, &project_dir);
    let saved = backstage_core::load(&project_dir);
    assert!(passed, "smoke test failed:\n{log}");
    assert!(log.contains("passed"), "no pass marker:\n{log}");
    assert!(log.contains("smoke: stage replayed 1 entry and matches the editor"), "no replay:\n{log}");
    assert!(log.contains("smoke: scrubbed, nudging"), "no scrub:\n{log}");
    assert!(log.contains("smoke: saved, undoing"), "no save:\n{log}");
    assert!(log.contains(&format!("autosaving edits to {}", state.display())), "autosave is off:\n{log}");
    // Saved after the nudge and before the undo.
    let ground = |p: &backstage_core::Project| p.compositions[&STAGE].nodes[&GROUND].rest.transform.position;
    let (before, after) = (ground(&sample), ground(&saved.expect("loading the saved project")));
    assert_eq!((after.x, after.y), (before.x + 10.0, before.y), "the nudge was saved");

    // The first run exited without cleaning up, leaving the undo unsaved.
    let (passed, log) = run_editor("restore", &state, &project_dir);
    let _ = std::fs::remove_dir_all(&state);
    assert!(passed, "restore smoke test failed:\n{log}");
    assert!(
        log.contains("smoke: restored 2 entries, and the stage matches the editor"),
        "no restore:\n{log}"
    );
}
