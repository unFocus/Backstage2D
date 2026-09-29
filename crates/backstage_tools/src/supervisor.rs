//! Starts, watches, and restarts the `backstage_stage` process.
//!
//! Each start is a new *session*. Events from older sessions are tagged with
//! their session number so the UI can ignore them after a restart.

use backstage_protocol::{FrameRing, PROTOCOL_VERSION, ToStage, ToTools, read_message, write_message};
use gtk::glib;
use relm4::gtk;
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Project directory for the stage to show; unset means its built-in sample.
pub const PROJECT_ENV: &str = "BACKSTAGE_PROJECT";

/// Session numbers are unique across all supervisors in this process, so
/// socket paths (`tools-<pid>-<session>.sock`) never collide.
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub enum StageEvent {
    Connected {
        pid: u32,
        adapter: String,
    },
    /// A new frame is waiting in [`Supervisor::take_frame`]. Sent at most
    /// once until the frame is taken, so a slow UI never queues frames.
    FrameAvailable,
    Alive,
    Log(String),
    Exited(String),
}

#[derive(Debug)]
pub struct Frame {
    pub pixels: glib::Bytes,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

#[derive(Default)]
struct LatestFrame {
    frame: Mutex<Option<Frame>>,
    pending: AtomicBool,
}

pub struct Supervisor {
    stage_binary: PathBuf,
    session: u64,
    child: Option<Child>,
    writer: Arc<Mutex<Option<UnixStream>>>,
    latest: Arc<LatestFrame>,
}

impl Supervisor {
    /// `stage_binary` is the `backstage_stage` executable to launch.
    pub fn new(stage_binary: PathBuf) -> Self {
        remove_stale_files();
        Self { stage_binary, session: 0, child: None, writer: Arc::default(), latest: Arc::default() }
    }

    pub fn session(&self) -> u64 {
        self.session
    }

    /// Stops any running stage and starts a new one. `emit` is called from a
    /// background thread with the session number and each event.
    pub fn start(&mut self, emit: impl Fn(u64, StageEvent) + Send + 'static) -> io::Result<u64> {
        self.stop();
        let session = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        self.session = session;
        self.writer = Arc::default();
        self.latest = Arc::default();

        let socket_path = runtime_dir()?.join(format!("tools-{}-{session}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path)?;
        let mut command = Command::new(&self.stage_binary);
        command.arg("--socket").arg(&socket_path);
        // Stopgap until File → Open (M3): which project the stage shows.
        if let Some(project) = std::env::var_os(PROJECT_ENV) {
            command.arg("--project").arg(project);
        }
        let child = command.spawn()?;
        let stage_pid = child.id();
        self.child = Some(child);

        let writer = self.writer.clone();
        let latest = self.latest.clone();
        std::thread::Builder::new().name(format!("stage-session-{session}")).spawn(move || {
            let reason = match accept(&listener, &socket_path) {
                Ok(stream) => run_session(stream, &writer, &latest, &|e| emit(session, e)),
                Err(e) => format!("stage did not connect: {e}"),
            };
            writer.lock().unwrap().take();
            remove_ring_files(stage_pid);
            emit(session, StageEvent::Exited(reason));
        })?;
        Ok(session)
    }

    /// Sends a message to the stage. Silently dropped if it isn't connected;
    /// the stage gets the current state again when it (re)connects.
    pub fn send(&self, msg: &ToStage) {
        if let Some(stream) = self.writer.lock().unwrap().as_mut() {
            let _ = write_message(stream, msg);
        }
    }

    pub fn take_frame(&self) -> Option<Frame> {
        self.latest.pending.store(false, Ordering::Release);
        self.latest.frame.lock().unwrap().take()
    }

    /// Simulates a crash: SIGKILL, no goodbye.
    pub fn kill(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
        }
    }

    /// Asks the stage to exit, then makes sure it has.
    pub fn stop(&mut self) {
        self.send(&ToStage::Shutdown);
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_millis(500);
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Reaps the child after it exited on its own.
    pub fn reap(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept(listener: &UnixListener, socket_path: &Path) -> io::Result<UnixStream> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let result = loop {
        match listener.accept() {
            Ok((stream, _)) => break Ok(stream),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                break Err(io::Error::new(io::ErrorKind::TimedOut, "timed out"));
            }
            Err(e) => break Err(e),
        }
    };
    let _ = std::fs::remove_file(socket_path);
    let stream = result?;
    stream.set_nonblocking(false)?;
    Ok(stream)
}

/// Reads stage messages until the connection ends. Returns why it ended.
fn run_session(
    mut stream: UnixStream,
    writer: &Mutex<Option<UnixStream>>,
    latest: &LatestFrame,
    emit: &dyn Fn(StageEvent),
) -> String {
    let mut write_half = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => return format!("socket error: {e}"),
    };
    if let Err(e) = write_message(&mut write_half, &ToStage::Hello { version: PROTOCOL_VERSION }) {
        return format!("handshake failed: {e}");
    }
    *writer.lock().unwrap() = Some(write_half);

    let mut surface: Option<(u64, FrameRing, u32, u32, u32)> = None;
    loop {
        let msg = match read_message::<ToTools>(&mut stream) {
            Ok(msg) => msg,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return "stage exited".into(),
            Err(e) => return format!("connection error: {e}"),
        };
        match msg {
            ToTools::Hello { version, .. } if version != PROTOCOL_VERSION => {
                return format!("protocol mismatch: stage {version}, tools {PROTOCOL_VERSION}");
            }
            ToTools::Hello { pid, adapter, .. } => emit(StageEvent::Connected { pid, adapter }),
            ToTools::Surface { generation, path, width, height, stride } => {
                surface = match FrameRing::open(Path::new(&path), stride, height) {
                    Ok(ring) => Some((generation, ring, width, height, stride)),
                    Err(e) => {
                        emit(StageEvent::Log(format!("cannot map {path}: {e}")));
                        None
                    }
                };
            }
            ToTools::FrameReady { generation, slot, seq } => {
                let Some((current, ring, width, height, stride)) = &surface else { continue };
                if *current != generation {
                    continue;
                }
                // `None` means the stage already overwrote this slot: drop it.
                if let Some(pixels) = ring.read(slot, seq) {
                    *latest.frame.lock().unwrap() = Some(Frame {
                        pixels: glib::Bytes::from_owned(pixels),
                        width: *width,
                        height: *height,
                        stride: *stride,
                    });
                    if !latest.pending.swap(true, Ordering::AcqRel) {
                        emit(StageEvent::FrameAvailable);
                    }
                }
            }
            ToTools::Heartbeat => emit(StageEvent::Alive),
            ToTools::Log(line) => emit(StageEvent::Log(line)),
        }
    }
}

/// Directory holding the stage socket and frame rings.
pub fn runtime_dir() -> io::Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
    let dir = PathBuf::from(base).join("backstage2d");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A killed stage can't clean up its frame rings; do it for it.
fn remove_ring_files(stage_pid: u32) {
    let Ok(dir) = runtime_dir() else { return };
    let prefix = format!("stage-{stage_pid}-");
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Removes sockets and frame rings left behind by processes that died without
/// cleaning up (for example when the whole process group was killed).
pub fn remove_stale_files() {
    let Ok(dir) = runtime_dir() else { return };
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let owner = owner_pid(&entry.file_name().to_string_lossy());
        if owner.is_some_and(|pid| !Path::new(&format!("/proc/{pid}")).exists()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The pid that owns a runtime file (`stage-<pid>-…` or `tools-<pid>-…`).
fn owner_pid(file_name: &str) -> Option<u32> {
    file_name
        .strip_prefix("stage-")
        .or_else(|| file_name.strip_prefix("tools-"))
        .and_then(|rest| rest.split('-').next())
        .and_then(|pid| pid.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::owner_pid;

    #[test]
    fn owner_pid_parses_runtime_file_names() {
        assert_eq!(owner_pid("stage-1234-7.frames"), Some(1234));
        assert_eq!(owner_pid("tools-99-1.sock"), Some(99));
        assert_eq!(owner_pid("stage-abc-1.frames"), None);
        assert_eq!(owner_pid("other-1234-1"), None);
        assert_eq!(owner_pid("stage-"), None);
    }
}
