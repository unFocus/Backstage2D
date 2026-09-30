//! Self-driving smoke test for headless UI runs (`BACKSTAGE_SMOKE=1`). It
//! runs M2's whole loop in the real editor, through the same paths as the
//! user: wait for frames, nudge, kill the stage once the edit is committed,
//! check the restarted stage replayed it, then undo and exit 0. See
//! `tests/ui_smoke.rs`.
//!
//! The app calls the hooks below and carries out the [`SmokeStep`]s they
//! return.

use std::time::Duration;

pub const SMOKE_ENV: &str = "BACKSTAGE_SMOKE";
pub const SMOKE_TIMEOUT: Duration = Duration::from_secs(20);
const FRAMES_PER_PHASE: u32 = 30;

/// What the app should do next.
#[derive(Debug, PartialEq)]
pub enum SmokeStep {
    Nudge,
    Undo,
    Kill,
    Pass,
    Fail(String),
}

#[derive(Debug, PartialEq)]
enum Phase {
    /// Frames from the first stage.
    FirstFrames(u32),
    /// Nudged; waiting for the commit.
    AwaitNudge,
    /// Killed; waiting for a new stage to replay the log.
    AwaitReplay,
    /// Frames from the restarted stage.
    SecondFrames(u32),
    /// Undid; waiting for the commit.
    AwaitUndo,
    Done,
}

#[derive(Debug)]
pub struct SmokeTest {
    first_session: Option<u64>,
    phase: Phase,
}

impl SmokeTest {
    fn new() -> Self {
        Self { first_session: None, phase: Phase::FirstFrames(0) }
    }

    pub fn from_env() -> Option<Self> {
        std::env::var(SMOKE_ENV).is_ok_and(|v| v == "1").then(Self::new)
    }

    /// A frame was shown.
    pub fn frame(&mut self, session: u64) -> Option<SmokeStep> {
        let first = *self.first_session.get_or_insert(session);
        match &mut self.phase {
            Phase::FirstFrames(n) if session == first => {
                *n += 1;
                if *n < FRAMES_PER_PHASE {
                    return None;
                }
                eprintln!("smoke: {n} frames from session {session}, nudging");
                self.phase = Phase::AwaitNudge;
                Some(SmokeStep::Nudge)
            }
            Phase::SecondFrames(n) if session != first => {
                *n += 1;
                if *n < FRAMES_PER_PHASE {
                    return None;
                }
                eprintln!("smoke: {n} frames from restarted session {session}, undoing");
                self.phase = Phase::AwaitUndo;
                Some(SmokeStep::Undo)
            }
            _ => None,
        }
    }

    /// The editor's copy applied committed entry `seq`.
    pub fn committed(&mut self, seq: u64) -> Option<SmokeStep> {
        match (&self.phase, seq) {
            (Phase::AwaitNudge, 1) => {
                eprintln!("smoke: nudge committed, killing stage");
                self.phase = Phase::AwaitReplay;
                Some(SmokeStep::Kill)
            }
            (Phase::AwaitUndo, 2) => {
                eprintln!("smoke: undo committed on the restarted stage, passed");
                self.phase = Phase::Done;
                Some(SmokeStep::Pass)
            }
            (phase, seq) => Some(SmokeStep::Fail(format!("unexpected commit {seq} in phase {phase:?}"))),
        }
    }

    /// A stage finished loading; `matches` is the editor's hash check.
    pub fn loaded(&mut self, session: u64, seq: u64, matches: bool) -> Option<SmokeStep> {
        if !matches {
            return Some(SmokeStep::Fail(format!("session {session}: replayed document differs")));
        }
        if self.phase == Phase::AwaitReplay && Some(session) != self.first_session {
            if seq != 1 {
                return Some(SmokeStep::Fail(format!("restarted stage replayed {seq} entries, expected 1")));
            }
            eprintln!("smoke: stage replayed 1 entry and matches the editor");
            self.phase = Phase::SecondFrames(0);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{FRAMES_PER_PHASE, SmokeStep, SmokeTest};

    fn frames(smoke: &mut SmokeTest, session: u64, n: u32) -> Vec<SmokeStep> {
        (0..n).filter_map(|_| smoke.frame(session)).collect()
    }

    #[test]
    fn runs_edit_kill_replay_undo_in_order() {
        let mut smoke = SmokeTest::new();
        assert_eq!(smoke.loaded(1, 0, true), None);
        assert_eq!(frames(&mut smoke, 1, FRAMES_PER_PHASE), [SmokeStep::Nudge]);
        assert_eq!(smoke.committed(1), Some(SmokeStep::Kill));
        // Frames still in flight from the killed stage don't count.
        assert_eq!(frames(&mut smoke, 1, FRAMES_PER_PHASE), []);
        // Nor do frames from the new stage before its replay checked out.
        assert_eq!(frames(&mut smoke, 2, FRAMES_PER_PHASE), []);
        assert_eq!(smoke.loaded(2, 1, true), None);
        assert_eq!(frames(&mut smoke, 2, FRAMES_PER_PHASE), [SmokeStep::Undo]);
        assert_eq!(smoke.committed(2), Some(SmokeStep::Pass));
    }

    #[test]
    fn a_mismatched_or_empty_replay_fails() {
        let mut smoke = SmokeTest::new();
        assert!(matches!(smoke.loaded(1, 0, false), Some(SmokeStep::Fail(_))));

        let mut smoke = SmokeTest::new();
        frames(&mut smoke, 1, FRAMES_PER_PHASE);
        smoke.committed(1);
        assert!(matches!(smoke.loaded(2, 0, true), Some(SmokeStep::Fail(_))), "the edit was lost");
    }

    #[test]
    fn unexpected_commits_fail() {
        let mut smoke = SmokeTest::new();
        assert!(matches!(smoke.committed(1), Some(SmokeStep::Fail(_))));
    }
}
