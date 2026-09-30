//! Integration tests: the real `Supervisor` managing the real stage binary
//! (software adapter), without a GTK main loop.

mod common;

use backstage_protocol::{Snapshot, ToStage};
use backstage_tools::supervisor::{self, StageEvent, Supervisor};
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
