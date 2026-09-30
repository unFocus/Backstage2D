//! Integration tests: the real `Supervisor` managing the real stage binary
//! (software adapter), without a GTK main loop.

mod common;

use backstage_core::sample::ids;
use backstage_core::{Command, Entry, Props};
use backstage_protocol::{Snapshot, ToStage};
use backstage_tools::recovery::Recovery;
use backstage_tools::supervisor::{self, StageEvent, Supervisor};
use backstage_tools::working_copy::WorkingCopy;
use std::path::Path;
use std::sync::Once;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(20);

/// All tests in this binary share one private runtime dir and the software
/// adapter. Set once, before any test spawns anything.
fn setup() {
    static ENV: Once = Once::new();
    ENV.call_once(|| {
        let dir = std::env::temp_dir().join(format!("backstage-tools-it-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: runs once, before this binary's tests start processes or
        // read these variables.
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", &dir);
            std::env::set_var("BACKSTAGE_WGPU_FALLBACK", "1");
        }
    });
}

type Events = Receiver<(u64, StageEvent)>;

fn start(sup: &mut Supervisor) -> (u64, Events) {
    let (tx, rx) = channel();
    let session = sup.start(move |s, e| drop(tx.send((s, e)))).unwrap();
    (session, rx)
}

fn wait_for<T>(events: &Events, mut f: impl FnMut(u64, StageEvent) -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let (session, event) = events.recv_timeout(left).expect("timed out waiting for event");
        if let Some(v) = f(session, event) {
            return v;
        }
    }
}

/// Sends the sample project; the stage shows nothing without a document.
fn load_sample(sup: &Supervisor) {
    let base = Snapshot::of(&backstage_core::sample::bounce()).unwrap();
    sup.send(&ToStage::Load { base, log: Vec::new() });
}

fn connected_pid(events: &Events, session: u64) -> u32 {
    wait_for(events, |s, e| match e {
        StageEvent::Connected { pid, .. } if s == session => Some(pid),
        _ => None,
    })
}

fn rings_of(pid: u32) -> Vec<String> {
    let prefix = format!("stage-{pid}-");
    std::fs::read_dir(supervisor::runtime_dir().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&prefix))
        .collect()
}

#[test]
fn streams_frames_to_take_frame() {
    setup();
    let mut sup = Supervisor::new(common::stage_binary());
    let (session, events) = start(&mut sup);
    connected_pid(&events, session);
    load_sample(&sup);
    sup.send(&ToStage::Resize { width: 200, height: 100, scale: 1.0 });
    wait_for(&events, |_, e| matches!(e, StageEvent::FrameAvailable).then_some(()));
    let frame = sup.take_frame().expect("frame after FrameAvailable");
    assert_eq!((frame.width, frame.height), (200, 100));
    assert!(frame.stride >= 800);
    assert_eq!(frame.pixels.len(), (frame.stride * frame.height) as usize);
}

#[test]
fn kill_reports_exit_and_cleans_up_rings() {
    setup();
    let mut sup = Supervisor::new(common::stage_binary());
    let (session, events) = start(&mut sup);
    let pid = connected_pid(&events, session);
    load_sample(&sup);
    sup.send(&ToStage::Resize { width: 64, height: 64, scale: 1.0 });
    wait_for(&events, |_, e| matches!(e, StageEvent::FrameAvailable).then_some(()));
    assert!(!rings_of(pid).is_empty());

    sup.kill();
    wait_for(&events, |s, e| (s == session && matches!(e, StageEvent::Exited(_))).then_some(()));
    sup.reap();
    assert_eq!(rings_of(pid), Vec::<String>::new(), "killed stage's rings must be removed");
}

#[test]
fn restart_starts_a_new_session() {
    setup();
    let mut sup = Supervisor::new(common::stage_binary());
    let (first, events_1) = start(&mut sup);
    let pid_1 = connected_pid(&events_1, first);

    let (second, events_2) = start(&mut sup);
    assert!(second > first);
    assert_eq!(sup.session(), second);
    let pid_2 = connected_pid(&events_2, second);
    assert_ne!(pid_1, pid_2);
    // The old session ends with its own session number, which the UI ignores.
    wait_for(&events_1, |s, e| (s == first && matches!(e, StageEvent::Exited(_))).then_some(()));
}

#[test]
fn stop_shuts_the_stage_down() {
    setup();
    let mut sup = Supervisor::new(common::stage_binary());
    let (session, events) = start(&mut sup);
    let pid = connected_pid(&events, session);
    sup.stop();
    wait_for(&events, |s, e| (s == session && matches!(e, StageEvent::Exited(_))).then_some(()));
    assert!(!Path::new(&format!("/proc/{pid}")).exists(), "stage {pid} still running");
    assert_eq!(rings_of(pid), Vec::<String>::new());
}

#[test]
fn stale_files_from_dead_processes_are_swept() {
    setup();
    let dir = supervisor::runtime_dir().unwrap();
    let mut dead = std::process::Command::new("true").spawn().unwrap();
    let dead_pid = dead.id();
    dead.wait().unwrap();
    let own_pid = std::process::id();
    let stale = [format!("stage-{dead_pid}-1.frames"), format!("tools-{dead_pid}-1.sock")];
    let live = format!("stage-{own_pid}-999.frames");
    for name in stale.iter().chain([&live]) {
        std::fs::write(dir.join(name), b"").unwrap();
    }

    let _sup = Supervisor::new(common::stage_binary());

    for name in &stale {
        assert!(!dir.join(name).exists(), "{name} should have been swept");
    }
    assert!(dir.join(&live).exists(), "files of live processes must be kept");
    std::fs::remove_file(dir.join(live)).unwrap();
}

/// Applies the `Committed` event for `request` (from `session`) to `copy`.
fn commit(copy: &mut WorkingCopy, sup: &Supervisor, events: &Events, session: u64, entry: Entry) -> u64 {
    let ToStage::Submit { request, .. } = copy.submit(entry.clone()) else { unreachable!() };
    sup.send(&ToStage::Submit { request, entry });
    let (seq, entry) = wait_for(events, |s, e| match e {
        StageEvent::Committed { seq, request: Some(r), entry } if s == session && r == request => {
            Some((seq, entry))
        }
        StageEvent::Rejected { request: r, reason } if s == session && r == request => {
            panic!("rejected: {reason}")
        }
        _ => None,
    });
    copy.committed(seq, &entry).unwrap();
    seq
}

fn loaded(events: &Events, session: u64) -> (u64, u64) {
    wait_for(events, |s, e| match e {
        StageEvent::Loaded { seq, hash } if s == session => Some((seq, hash)),
        _ => None,
    })
}

/// M2's promise: edits survive a stage crash. The editor's copy follows the
/// stage's commits; after a kill, the restarted stage replays the copy's
/// log into an identical document and carries on numbering from there.
#[test]
fn a_restarted_stage_replays_the_editors_log() {
    setup();
    let nudge =
        |x| Entry::Do(Command::SetRest { comp: ids::STAGE, node: ids::GROUND, rest: Props::at(x, 0.0) });
    let mut copy = WorkingCopy::new(backstage_core::sample::bounce(), None, None).unwrap();
    let mut sup = Supervisor::new(common::stage_binary());

    let (first, events) = start(&mut sup);
    connected_pid(&events, first);
    sup.send(&copy.load_message());
    let (seq, hash) = loaded(&events, first);
    copy.check_loaded(seq, hash).unwrap();
    for entry in [nudge(10.0), nudge(20.0), Entry::Undo] {
        commit(&mut copy, &sup, &events, first, entry);
    }

    sup.kill();
    wait_for(&events, |s, e| (s == first && matches!(e, StageEvent::Exited(_))).then_some(()));
    sup.reap();

    let (second, events) = start(&mut sup);
    connected_pid(&events, second);
    sup.send(&copy.load_message());
    let (seq, hash) = loaded(&events, second);
    assert_eq!(seq, 3);
    copy.check_loaded(seq, hash).unwrap();
    // The history came through too: redo works on the new stage.
    assert_eq!(commit(&mut copy, &sup, &events, second, Entry::Redo), 4);
    assert_eq!(
        copy.document().project().compositions[&ids::STAGE].nodes[&ids::GROUND].rest,
        Props::at(20.0, 0.0)
    );
}

/// File → Open keeps the running stage and sends it the new document. A
/// commit for the old document still in flight is ignored by the new copy,
/// which takes commits again once the stage has loaded it.
#[test]
fn a_second_load_replaces_the_document() {
    setup();
    let nudge =
        |x| Entry::Do(Command::SetRest { comp: ids::STAGE, node: ids::GROUND, rest: Props::at(x, 0.0) });
    let mut old = WorkingCopy::new(backstage_core::sample::bounce(), None, None).unwrap();
    let mut sup = Supervisor::new(common::stage_binary());
    let (session, events) = start(&mut sup);
    connected_pid(&events, session);
    sup.send(&old.load_message());
    let (seq, hash) = loaded(&events, session);
    old.check_loaded(seq, hash).unwrap();
    commit(&mut old, &sup, &events, session, nudge(10.0));

    // Another project: the sample with the ground somewhere else.
    let mut project = backstage_core::sample::bounce();
    let Entry::Do(moved) = nudge(99.0) else { unreachable!() };
    moved.apply(&mut project).unwrap();
    let mut new = WorkingCopy::new(project, None, None).unwrap();

    // An edit to the old document is in flight when the new one is sent.
    sup.send(&old.submit(nudge(20.0)));
    sup.send(&new.load_message());
    let mut ignored = 0;
    let (seq, hash) = wait_for(&events, |s, e| match e {
        StageEvent::Committed { seq, entry, .. } if s == session => {
            assert!(!new.committed(seq, &entry).unwrap(), "the old document's commit is ignored");
            ignored += 1;
            None
        }
        StageEvent::Loaded { seq, hash } if s == session => Some((seq, hash)),
        _ => None,
    });
    assert_eq!((ignored, seq), (1, 0));
    new.check_loaded(seq, hash).unwrap();
    assert_eq!(commit(&mut new, &sup, &events, session, nudge(30.0)), 1);
    assert_eq!(
        new.document().project().compositions[&ids::STAGE].nodes[&ids::GROUND].rest,
        Props::at(30.0, 0.0)
    );
    assert_eq!(commit(&mut new, &sup, &events, session, Entry::Undo), 2);
    assert_eq!(
        new.document().project().compositions[&ids::STAGE].nodes[&ids::GROUND].rest,
        Props::at(99.0, 0.0)
    );
}

/// After an editor crash, a copy restored from its recovery directory
/// loads onto a fresh stage as the same document, and its undo history
/// carries on there.
#[test]
fn a_restored_copy_loads_with_its_history() {
    setup();
    let nudge =
        |x| Entry::Do(Command::SetRest { comp: ids::STAGE, node: ids::GROUND, rest: Props::at(x, 0.0) });
    let dir = std::env::temp_dir().join(format!("backstage-tools-it-restore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = backstage_core::sample::bounce();
    let recovery = Recovery::create(dir.clone(), &project, None).unwrap();
    let mut copy = WorkingCopy::new(project, None, Some(recovery)).unwrap();
    let mut sup = Supervisor::new(common::stage_binary());

    let (first, events) = start(&mut sup);
    connected_pid(&events, first);
    sup.send(&copy.load_message());
    let (seq, hash) = loaded(&events, first);
    copy.check_loaded(seq, hash).unwrap();
    for entry in [nudge(10.0), nudge(20.0), Entry::Undo] {
        commit(&mut copy, &sup, &events, first, entry);
    }
    // The editor crashes: its copy is gone without cleaning up.
    drop(copy);
    sup.stop();

    let mut restored = WorkingCopy::restore(&dir).unwrap();
    let (second, events) = start(&mut sup);
    connected_pid(&events, second);
    sup.send(&restored.load_message());
    let (seq, hash) = loaded(&events, second);
    assert_eq!(seq, 3);
    restored.check_loaded(seq, hash).unwrap();
    assert_eq!(commit(&mut restored, &sup, &events, second, Entry::Redo), 4);
    assert_eq!(
        restored.document().project().compositions[&ids::STAGE].nodes[&ids::GROUND].rest,
        Props::at(20.0, 0.0)
    );
    restored.discard();
    assert!(!dir.exists());
}
