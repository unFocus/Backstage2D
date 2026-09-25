//! Self-driving smoke test for headless UI runs (`BACKSTAGE_SMOKE=1`):
//! wait for frames, kill the stage, wait for frames from the restarted stage,
//! then exit 0. See `tests/ui_smoke.rs`.

use crate::supervisor::Supervisor;
use std::time::Duration;

pub const SMOKE_ENV: &str = "BACKSTAGE_SMOKE";
pub const SMOKE_TIMEOUT: Duration = Duration::from_secs(20);
const FRAMES_PER_PHASE: u32 = 30;

#[derive(Debug, Default)]
pub struct SmokeTest {
    first_session: Option<u64>,
    frames: u32,
    killed: bool,
}

impl SmokeTest {
    pub fn from_env() -> Option<Self> {
        std::env::var(SMOKE_ENV).is_ok_and(|v| v == "1").then(Self::default)
    }

    /// Called for every frame shown. Exits the process when the test passes.
    pub fn frame(&mut self, session: u64, supervisor: &mut Supervisor) {
        let first = *self.first_session.get_or_insert(session);
        if session == first && self.killed {
            return;
        }
        self.frames += 1;
        if self.frames < FRAMES_PER_PHASE {
            return;
        }
        if session == first {
            eprintln!("smoke: {} frames from session {session}, killing stage", self.frames);
            self.killed = true;
            self.frames = 0;
            supervisor.kill();
        } else {
            eprintln!("smoke: {} frames from restarted session {session}, passed", self.frames);
            supervisor.stop();
            std::process::exit(0);
        }
    }
}
