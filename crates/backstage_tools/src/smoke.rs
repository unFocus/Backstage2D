//! Self-driving smoke test for headless UI runs. See `tests/ui_smoke.rs`.
//!
//! - `BACKSTAGE_SMOKE=1` runs the editing loop in the real editor, through
//!   the same paths as the user: wait for frames, scrub the timeline, select
//!   the first node, nudge, kill the stage once
//!   the edit is committed, check the restarted stage replayed it, save, then
//!   undo and exit 0 without cleaning up, like a crash. That leaves unsaved
//!   edits (the undo) in the recovery directory.
//! - `BACKSTAGE_SMOKE=restore`, run next with the same state directory,
//!   answers the restore prompt with Restore and passes once the stage has
//!   loaded the restored document, history included.
//!
//! The app calls the hooks below and carries out the [`SmokeStep`]s they
//! return.

use backstage_core::Time;
use std::time::Duration;

pub const SMOKE_ENV: &str = "BACKSTAGE_SMOKE";
pub const SMOKE_TIMEOUT: Duration = Duration::from_secs(20);
const FRAMES_PER_PHASE: u32 = 30;

/// What the app should do next.
#[derive(Debug, PartialEq)]
pub enum SmokeStep {
    /// Scrub the timeline to this time.
    Scrub(Time),
    /// Select the first node, as a click on its timeline row does.
    Select,
    Nudge,
    Save,
    Restore,
    Undo,
    Kill,
    Pass,
    Fail(String),
}

#[derive(Debug, PartialEq)]
enum Phase {
    /// Frames from the first stage.
    FirstFrames(u32),
    /// Asked to scrub.
    AwaitScrub,
    /// Asked to select.
    AwaitSelect,
    /// Nudged; waiting for the commit.
    AwaitNudge,
    /// Killed; waiting for a new stage to replay the log.
    AwaitReplay,
    /// Frames from the restarted stage.
    SecondFrames(u32),
    /// Asked to save.
    AwaitSave,
    /// Undid; waiting for the commit.
    AwaitUndo,
    /// Restore mode: waiting for the restore prompt.
    AwaitOffer,
    /// Restored; waiting for the stage to load it.
    AwaitRestoredLoad,
    Done,
}

/// A stage finished loading.
#[derive(Debug, Clone, Copy)]
pub struct Loaded {
    pub seq: u64,
    /// The editor's hash check passed.
    pub matches: bool,
    /// The editor's copy has unsaved changes.
    pub dirty: bool,
    pub can_redo: bool,
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

    fn restore() -> Self {
        Self { first_session: None, phase: Phase::AwaitOffer }
    }

    pub fn from_env() -> Option<Self> {
        match std::env::var(SMOKE_ENV).ok()?.as_str() {
            "1" => Some(Self::new()),
            "restore" => Some(Self::restore()),
            _ => None,
        }
    }

    /// The editor found unsaved edits from a crashed run.
    pub fn restore_offered(&mut self) -> Option<SmokeStep> {
        if self.phase != Phase::AwaitOffer {
            return Some(SmokeStep::Fail(format!("unexpected restore offer in phase {:?}", self.phase)));
        }
        eprintln!("smoke: restore offered, restoring");
        self.phase = Phase::AwaitRestoredLoad;
        Some(SmokeStep::Restore)
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
                eprintln!("smoke: {n} frames from session {session}, scrubbing");
                self.phase = Phase::AwaitScrub;
                Some(SmokeStep::Scrub(Time::from_secs(1)))
            }
            Phase::SecondFrames(n) if session != first => {
                *n += 1;
                if *n < FRAMES_PER_PHASE {
                    return None;
                }
                eprintln!("smoke: {n} frames from restarted session {session}, saving");
                self.phase = Phase::AwaitSave;
                Some(SmokeStep::Save)
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

    /// The scrub was carried out.
    pub fn scrubbed(&mut self) -> Option<SmokeStep> {
        if self.phase != Phase::AwaitScrub {
            return Some(SmokeStep::Fail(format!("unexpected scrub in phase {:?}", self.phase)));
        }
        eprintln!("smoke: scrubbed, selecting");
        self.phase = Phase::AwaitSelect;
        Some(SmokeStep::Select)
    }

    /// The selection was made; `ok` if a node is selected.
    pub fn selected(&mut self, ok: bool) -> Option<SmokeStep> {
        match (&self.phase, ok) {
            (Phase::AwaitSelect, true) => {
                eprintln!("smoke: selected, nudging");
                self.phase = Phase::AwaitNudge;
                Some(SmokeStep::Nudge)
            }
            (phase, ok) => {
                Some(SmokeStep::Fail(format!("unexpected selection (ok: {ok}) in phase {phase:?}")))
            }
        }
    }

    /// A save finished; `ok` if the project was written.
    pub fn saved(&mut self, ok: bool) -> Option<SmokeStep> {
        match (&self.phase, ok) {
            (Phase::AwaitSave, true) => {
                eprintln!("smoke: saved, undoing");
                self.phase = Phase::AwaitUndo;
                Some(SmokeStep::Undo)
            }
            (phase, ok) => Some(SmokeStep::Fail(format!("unexpected save (ok: {ok}) in phase {phase:?}"))),
        }
    }

    /// A stage finished loading; `matches` is the editor's hash check.
    pub fn loaded(&mut self, session: u64, loaded: Loaded) -> Option<SmokeStep> {
        let Loaded { seq, matches, dirty, can_redo } = loaded;
        if !matches {
            return Some(SmokeStep::Fail(format!("session {session}: replayed document differs")));
        }
        if self.phase == Phase::AwaitRestoredLoad {
            // The first run left [nudge, undo], saved after the nudge.
            if (seq, dirty, can_redo) != (2, true, true) {
                return Some(SmokeStep::Fail(format!(
                    "restored document: seq {seq}, dirty {dirty}, can redo {can_redo}; expected 2, true, true"
                )));
            }
            eprintln!("smoke: restored 2 entries, and the stage matches the editor");
            self.phase = Phase::Done;
            return Some(SmokeStep::Pass);
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
    use super::{FRAMES_PER_PHASE, Loaded, SmokeStep, SmokeTest, Time};

    fn ok(seq: u64) -> Loaded {
        Loaded { seq, matches: true, dirty: false, can_redo: false }
    }

    fn frames(smoke: &mut SmokeTest, session: u64, n: u32) -> Vec<SmokeStep> {
        (0..n).filter_map(|_| smoke.frame(session)).collect()
    }

    #[test]
    fn runs_edit_kill_replay_undo_in_order() {
        let mut smoke = SmokeTest::new();
        assert_eq!(smoke.loaded(1, ok(0)), None);
        assert_eq!(frames(&mut smoke, 1, FRAMES_PER_PHASE), [SmokeStep::Scrub(Time::from_secs(1))]);
        assert_eq!(smoke.scrubbed(), Some(SmokeStep::Select));
        assert_eq!(smoke.selected(true), Some(SmokeStep::Nudge));
        assert_eq!(smoke.committed(1), Some(SmokeStep::Kill));
        // Frames still in flight from the killed stage don't count.
        assert_eq!(frames(&mut smoke, 1, FRAMES_PER_PHASE), []);
        // Nor do frames from the new stage before its replay checked out.
        assert_eq!(frames(&mut smoke, 2, FRAMES_PER_PHASE), []);
        assert_eq!(smoke.loaded(2, ok(1)), None);
        assert_eq!(frames(&mut smoke, 2, FRAMES_PER_PHASE), [SmokeStep::Save]);
        assert_eq!(smoke.saved(true), Some(SmokeStep::Undo));
        assert_eq!(smoke.committed(2), Some(SmokeStep::Pass));
    }

    #[test]
    fn a_mismatched_or_empty_replay_fails() {
        let mut smoke = SmokeTest::new();
        assert!(matches!(smoke.loaded(1, Loaded { matches: false, ..ok(0) }), Some(SmokeStep::Fail(_))));

        let mut smoke = SmokeTest::new();
        frames(&mut smoke, 1, FRAMES_PER_PHASE);
        smoke.scrubbed();
        smoke.selected(true);
        smoke.committed(1);
        assert!(matches!(smoke.loaded(2, ok(0)), Some(SmokeStep::Fail(_))), "the edit was lost");
    }

    #[test]
    fn unexpected_commits_or_saves_fail() {
        let mut smoke = SmokeTest::new();
        assert!(matches!(smoke.committed(1), Some(SmokeStep::Fail(_))));
        assert!(matches!(smoke.saved(true), Some(SmokeStep::Fail(_))));
    }

    #[test]
    fn restore_mode_restores_and_checks_the_history() {
        let mut smoke = SmokeTest::restore();
        assert_eq!(smoke.restore_offered(), Some(SmokeStep::Restore));
        assert!(matches!(smoke.loaded(1, ok(2)), Some(SmokeStep::Fail(_))), "not dirty, no redo");
        let mut smoke = SmokeTest::restore();
        smoke.restore_offered();
        let restored = Loaded { dirty: true, can_redo: true, ..ok(2) };
        assert_eq!(smoke.loaded(1, restored), Some(SmokeStep::Pass));
    }

    #[test]
    fn an_offer_outside_restore_mode_fails() {
        let mut smoke = SmokeTest::new();
        assert!(matches!(smoke.restore_offered(), Some(SmokeStep::Fail(_))));
    }
}
