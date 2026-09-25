//! Shared helpers for tools integration tests.

#![allow(dead_code)] // each test binary uses a different subset

use std::path::PathBuf;
use std::sync::Once;

/// `backstage_stage` built into the same target directory as this test,
/// building it first if needed (e.g. under `cargo test -p backstage_tools`).
pub fn stage_binary() -> PathBuf {
    static BUILD: Once = Once::new();
    // target/<profile>/deps/<test-binary> → target/<profile>/backstage_stage
    let exe = std::env::current_exe().unwrap();
    let path = exe.parent().unwrap().parent().unwrap().join("backstage_stage");
    BUILD.call_once(|| {
        if !path.exists() {
            let status = std::process::Command::new(env!("CARGO"))
                .args(["build", "-p", "backstage_stage"])
                .status()
                .expect("running cargo build");
            assert!(status.success(), "building backstage_stage failed");
        }
    });
    assert!(path.exists(), "{} missing", path.display());
    path
}
