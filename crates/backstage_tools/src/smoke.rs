//! Self-driving smoke test for headless UI runs. See `tests/ui_smoke.rs`.
//!
//! - `BACKSTAGE_SMOKE=1` runs the editing loop in the real editor, through
//!   the same paths as the user: wait for frames, click on the stage (only
//!   under `backstage_uidriver`, which gives it real input: click the
//!   ground, Shift-click a ball, click empty stage), scrub the timeline,
//!   select the first node, nudge, kill the stage once
//!   the edit is committed, check the restarted stage replayed it, save, then
//!   undo and exit 0 without cleaning up, like a crash. That leaves unsaved
//!   edits (the undo) in the recovery directory.
//! - `BACKSTAGE_SMOKE=restore`, run next with the same state directory,
//!   answers the restore prompt with Restore and passes once the stage has
//!   loaded the restored document, history included.
//!
//! The app calls the hooks below and carries out the [`SmokeStep`]s they
//! return.

use backstage_core::sample::ids::{EYES, FREE_BALL, GROUND, SYNCED_BALL};
use backstage_core::{CompId, NodeId, Project, RuntimeState, Time, Vec2, evaluate_from};
use std::time::Duration;

pub const SMOKE_ENV: &str = "BACKSTAGE_SMOKE";
pub const SMOKE_TIMEOUT: Duration = Duration::from_secs(20);
/// The editor's stage starts paused and draws only when something changes,
/// so a phase sees just a frame or two; one proves the stage draws.
const FRAMES_PER_PHASE: u32 = 1;

/// Where a smoke click on the stage lands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClickTarget {
    /// The middle of what this node of the edited composition draws.
    Node(NodeId),
    /// A point in stage coordinates.
    Stage(Vec2),
}

/// One click or drag on the stage, and the selection it should leave.
struct Pick {
    what: &'static str,
    target: ClickTarget,
    /// Drag from `target` to here instead of clicking.
    drag_to: Option<ClickTarget>,
    shift: bool,
    expected: &'static [NodeId],
}

/// The sample's stage, clicked and dragged on through the real GUI.
const PICKS: [Pick; 4] = [
    Pick {
        what: "clicked the ground",
        target: ClickTarget::Node(GROUND),
        drag_to: None,
        shift: false,
        expected: &[GROUND],
    },
    Pick {
        what: "shift-clicked the free ball",
        target: ClickTarget::Node(FREE_BALL),
        drag_to: None,
        shift: true,
        expected: &[GROUND, FREE_BALL],
    },
    Pick {
        what: "clicked empty stage",
        target: ClickTarget::Stage(Vec2::new(10.0, 10.0)),
        drag_to: None,
        shift: false,
        expected: &[],
    },
    // Everything above the ground (stage y 340..400), bottom to top.
    Pick {
        what: "marquee-selected the eyes and both balls",
        target: ClickTarget::Stage(Vec2::new(10.0, 10.0)),
        drag_to: Some(ClickTarget::Stage(Vec2::new(540.0, 330.0))),
        shift: false,
        expected: &[EYES, FREE_BALL, SYNCED_BALL],
    },
];

/// What the app should do next.
#[derive(Debug, PartialEq)]
pub enum SmokeStep {
    /// Click on the stage through the UI driver, or drag to `drag_to`.
    StageInput {
        target: ClickTarget,
        drag_to: Option<ClickTarget>,
        shift: bool,
    },
    /// Scrub the timeline to this time.
    Scrub(Time),
    /// Select the first node, as a click on its timeline row does.
    Select,
    /// Enter the first instance's composition and come back out.
    Enter,
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
    /// Clicked `PICKS[i]` on the stage; waiting for the pick.
    AwaitPick(usize),
    /// Asked to scrub.
    AwaitScrub,
    /// Asked to select.
    AwaitSelect,
    /// Asked to enter a composition and come back.
    AwaitEnter,
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
    /// Real input is available (`backstage_uidriver`): click on the stage.
    stage_clicks: bool,
}

impl SmokeTest {
    fn new(stage_clicks: bool) -> Self {
        Self { first_session: None, phase: Phase::FirstFrames(0), stage_clicks }
    }

    fn restore() -> Self {
        Self { first_session: None, phase: Phase::AwaitOffer, stage_clicks: false }
    }

    /// The smoke test the environment asks for; `stage_clicks` when the
    /// editor runs under the UI driver.
    pub fn from_env(stage_clicks: bool) -> Option<Self> {
        match std::env::var(SMOKE_ENV).ok()?.as_str() {
            "1" => Some(Self::new(stage_clicks)),
            "restore" => Some(Self::restore()),
            _ => None,
        }
    }

    fn click(i: usize) -> SmokeStep {
        let Pick { target, drag_to, shift, .. } = PICKS[i];
        SmokeStep::StageInput { target, drag_to, shift }
    }

    /// A pick on the stage was applied; `selection` is the editor's now.
    pub fn picked(&mut self, selection: &[NodeId]) -> Option<SmokeStep> {
        let Phase::AwaitPick(i) = self.phase else {
            return Some(SmokeStep::Fail(format!("unexpected pick in phase {:?}", self.phase)));
        };
        let pick = &PICKS[i];
        if selection != pick.expected {
            return Some(SmokeStep::Fail(format!(
                "{}: selection {selection:?}, expected {:?}",
                pick.what, pick.expected
            )));
        }
        eprintln!("smoke: {} on stage", pick.what);
        if i + 1 < PICKS.len() {
            self.phase = Phase::AwaitPick(i + 1);
            return Some(Self::click(i + 1));
        }
        eprintln!("smoke: stage clicks done, scrubbing");
        self.phase = Phase::AwaitScrub;
        Some(SmokeStep::Scrub(Time::from_secs(1)))
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
                if self.stage_clicks {
                    eprintln!("smoke: {n} frames from session {session}, clicking on the stage");
                    self.phase = Phase::AwaitPick(0);
                    return Some(Self::click(0));
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
                eprintln!("smoke: selected, entering a composition");
                self.phase = Phase::AwaitEnter;
                Some(SmokeStep::Enter)
            }
            (phase, ok) => {
                Some(SmokeStep::Fail(format!("unexpected selection (ok: {ok}) in phase {phase:?}")))
            }
        }
    }

    /// Entered a composition and came back; `ok` if both worked.
    pub fn entered(&mut self, ok: bool) -> Option<SmokeStep> {
        match (&self.phase, ok) {
            (Phase::AwaitEnter, true) => {
                eprintln!("smoke: entered a composition and came back, nudging");
                self.phase = Phase::AwaitNudge;
                Some(SmokeStep::Nudge)
            }
            (phase, ok) => Some(SmokeStep::Fail(format!("unexpected enter (ok: {ok}) in phase {phase:?}"))),
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

/// Where to click for `target` in composition `comp` at `time` (the
/// default animation, as the editor's stage shows it unpaused), in stage
/// coordinates: for a node, the middle of the first shape it draws.
pub fn stage_point(project: &Project, comp: CompId, time: Time, target: ClickTarget) -> Option<Vec2> {
    let node = match target {
        ClickTarget::Stage(p) => return Some(p),
        ClickTarget::Node(node) => node,
    };
    let scene = evaluate_from(project, &RuntimeState::default(), comp, time);
    let item = scene.items.iter().find(|i| i.instance.first().copied().unwrap_or(i.node) == node)?;
    let backstage_core::DrawContent::Shape(shape) = item.content else { return None };
    let ends = shape.paths.first()?.path.iter().filter_map(|cmd| match *cmd {
        backstage_core::PathCmd::MoveTo(p)
        | backstage_core::PathCmd::LineTo(p)
        | backstage_core::PathCmd::QuadTo(_, p)
        | backstage_core::PathCmd::CubicTo(_, _, p) => Some(p),
        backstage_core::PathCmd::Close => None,
    });
    let (min, max) = ends.fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(lo, hi), p| (lo.min(p), hi.max(p)));
    min.is_finite().then(|| item.transform.transform_point2((min + max) / 2.0))
}

#[cfg(test)]
mod tests {
    use super::{ClickTarget, FRAMES_PER_PHASE, Loaded, PICKS, SmokeStep, SmokeTest, Time, stage_point};
    use backstage_core::sample::{self, ids::*};
    use backstage_core::{NodeId, Vec2};

    #[test]
    fn stage_clicks_run_before_the_scrub_and_check_each_selection() {
        let mut smoke = SmokeTest::new(true);
        let clicks = frames(&mut smoke, 1, FRAMES_PER_PHASE);
        let ground = SmokeStep::StageInput { target: ClickTarget::Node(GROUND), drag_to: None, shift: false };
        assert_eq!(clicks, [ground]);
        assert!(matches!(smoke.picked(&[GROUND]), Some(SmokeStep::StageInput { shift: true, .. })));
        assert!(matches!(
            smoke.picked(&[GROUND, FREE_BALL]),
            Some(SmokeStep::StageInput { shift: false, drag_to: None, .. })
        ));
        assert!(
            matches!(smoke.picked(&[]), Some(SmokeStep::StageInput { drag_to: Some(_), .. })),
            "a marquee"
        );
        assert_eq!(smoke.picked(&[EYES, FREE_BALL, SYNCED_BALL]), Some(SmokeStep::Scrub(Time::from_secs(1))));
        assert_eq!(smoke.scrubbed(), Some(SmokeStep::Select));

        let mut smoke = SmokeTest::new(true);
        frames(&mut smoke, 1, FRAMES_PER_PHASE);
        assert!(matches!(smoke.picked(&[FREE_BALL]), Some(SmokeStep::Fail(_))), "the wrong node");
        assert!(matches!(SmokeTest::new(true).picked(&[]), Some(SmokeStep::Fail(_))), "before any click");
    }

    #[test]
    fn click_targets_land_inside_what_they_name() {
        let project = sample::bounce();
        let at = |target| stage_point(&project, STAGE, Time::ZERO, target).unwrap();
        assert_eq!(at(ClickTarget::Node(GROUND)), Vec2::new(275.0, 370.0), "the strip's middle");
        assert_eq!(at(ClickTarget::Stage(Vec2::ONE)), Vec2::ONE);
        // The free ball's body is a circle around the instance's origin.
        let ball = at(ClickTarget::Node(FREE_BALL));
        let scene = backstage_core::evaluate(&project, &backstage_core::RuntimeState::default(), Time::ZERO);
        let body = scene.items.iter().find(|i| i.instance.first() == Some(&FREE_BALL)).unwrap();
        assert!(ball.distance(body.transform.translation) < 30.0, "{ball} vs {}", body.transform.translation);
        assert_eq!(stage_point(&project, STAGE, Time::ZERO, ClickTarget::Node(NodeId::from_raw(1))), None);
        assert!(PICKS.iter().all(|p| stage_point(&project, STAGE, Time::ZERO, p.target).is_some()));
    }

    fn ok(seq: u64) -> Loaded {
        Loaded { seq, matches: true, dirty: false, can_redo: false }
    }

    fn frames(smoke: &mut SmokeTest, session: u64, n: u32) -> Vec<SmokeStep> {
        (0..n).filter_map(|_| smoke.frame(session)).collect()
    }

    #[test]
    fn runs_edit_kill_replay_undo_in_order() {
        let mut smoke = SmokeTest::new(false);
        assert_eq!(smoke.loaded(1, ok(0)), None);
        assert_eq!(frames(&mut smoke, 1, FRAMES_PER_PHASE), [SmokeStep::Scrub(Time::from_secs(1))]);
        assert_eq!(smoke.scrubbed(), Some(SmokeStep::Select));
        assert_eq!(smoke.selected(true), Some(SmokeStep::Enter));
        assert_eq!(smoke.entered(true), Some(SmokeStep::Nudge));
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
        let mut smoke = SmokeTest::new(false);
        assert!(matches!(smoke.loaded(1, Loaded { matches: false, ..ok(0) }), Some(SmokeStep::Fail(_))));

        let mut smoke = SmokeTest::new(false);
        frames(&mut smoke, 1, FRAMES_PER_PHASE);
        smoke.scrubbed();
        smoke.selected(true);
        smoke.entered(true);
        smoke.committed(1);
        assert!(matches!(smoke.loaded(2, ok(0)), Some(SmokeStep::Fail(_))), "the edit was lost");
    }

    #[test]
    fn unexpected_commits_or_saves_fail() {
        let mut smoke = SmokeTest::new(false);
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
        let mut smoke = SmokeTest::new(false);
        assert!(matches!(smoke.restore_offered(), Some(SmokeStep::Fail(_))));
    }
}
