//! Shared helpers for tools integration tests.

#![allow(dead_code)] // each test binary uses a different subset

use std::path::PathBuf;
use std::sync::Once;

/// `backstage_stage` built into the same target directory as this test.
/// It's always (re)built first, which is a no-op when it's current: under
/// `cargo test -p backstage_tools` cargo doesn't rebuild another package's
/// binary, and a stale stage speaking an older protocol just crash-loops.
pub fn stage_binary() -> PathBuf {
    static BUILD: Once = Once::new();
    built("backstage_stage", &BUILD)
}

/// `backstage_uidriver` (real input and screenshots in headless cage),
/// built the same way.
pub fn uidriver_binary() -> PathBuf {
    static BUILD: Once = Once::new();
    built("backstage_uidriver", &BUILD)
}

fn built(package: &str, once: &Once) -> PathBuf {
    // target/<profile>/deps/<test-binary> → target/<profile>/<package>
    let exe = std::env::current_exe().unwrap();
    let path = exe.parent().unwrap().parent().unwrap().join(package);
    once.call_once(|| {
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "-p", package])
            .status()
            .expect("running cargo build");
        assert!(status.success(), "building {package} failed");
    });
    assert!(path.exists(), "{} missing", path.display());
    path
}
