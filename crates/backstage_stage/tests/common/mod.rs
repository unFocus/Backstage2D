//! A minimal stand-in for the tools process: listens on a socket, starts the
//! real `backstage_stage` binary, and speaks the protocol to it.

#![allow(dead_code)] // each test binary uses a different subset

use backstage_protocol::{FrameRing, PROTOCOL_VERSION, ToStage, ToTools, read_message, write_message};
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

pub const TIMEOUT: Duration = Duration::from_secs(20);

pub struct Frame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

impl Frame {
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y * self.stride + x * 4) as usize;
        self.pixels[i..i + 4].try_into().unwrap()
    }
}

pub fn color_close(actual: [u8; 4], expected: [f32; 4]) -> bool {
    actual.iter().zip(expected).all(|(&a, e)| (a as f32 - e * 255.0).abs() <= 2.0)
}

pub struct StageHarness {
    pub child: Child,
    pub stream: Option<UnixStream>,
    runtime_dir: PathBuf,
    surface: Option<(u64, FrameRing, u32, u32, u32)>,
}

impl StageHarness {
    /// Starts the stage on the software adapter.
    pub fn spawn() -> Self {
        Self::spawn_with_fallback(true)
    }

    pub fn spawn_with_fallback(fallback: bool) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let runtime_dir = std::env::temp_dir().join(format!(
            "backstage-stage-it-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&runtime_dir);
        std::fs::create_dir_all(&runtime_dir).unwrap();
        let socket_path = runtime_dir.join("tools.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_backstage_stage"));
        cmd.arg("--socket").arg(&socket_path).env("XDG_RUNTIME_DIR", &runtime_dir);
        if fallback {
            cmd.env(backstage_render::FALLBACK_ENV, "1");
        }
        let child = cmd.spawn().unwrap();
        let stream = accept(&listener).expect("stage did not connect");
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        Self { child, stream: Some(stream), runtime_dir, surface: None }
    }

    fn stream(&mut self) -> &mut UnixStream {
        self.stream.as_mut().expect("socket already closed")
    }

    pub fn send(&mut self, msg: &ToStage) {
        write_message(self.stream(), msg).unwrap();
    }

    pub fn recv(&mut self) -> ToTools {
        read_message(self.stream()).expect("reading from stage")
    }

    /// Sends our `Hello` and returns the adapter name from the stage's.
    pub fn handshake(&mut self) -> String {
        self.send(&ToStage::Hello { version: PROTOCOL_VERSION });
        match self.recv() {
            ToTools::Hello { version, adapter, .. } => {
                assert_eq!(version, PROTOCOL_VERSION);
                adapter
            }
            other => panic!("expected Hello, got {other:?}"),
        }
    }

    /// Receives messages until `f` returns `Some`, tracking the current ring.
    pub fn recv_until<T>(&mut self, mut f: impl FnMut(&ToTools) -> Option<T>) -> T {
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            let msg = self.recv();
            if let ToTools::Surface { generation, path, width, height, stride } = &msg {
                let ring = FrameRing::open(Path::new(path), *stride, *height).unwrap();
                self.surface = Some((*generation, ring, *width, *height, *stride));
            }
            if let Some(v) = f(&msg) {
                return v;
            }
        }
        panic!("timed out waiting for stage message");
    }

    /// Returns the next frame of the current generation.
    pub fn next_frame(&mut self) -> Frame {
        loop {
            let ready = self.recv_until(|m| match m {
                ToTools::FrameReady { generation, slot, seq } => Some((*generation, *slot, *seq)),
                _ => None,
            });
            let Some((current, ring, width, height, stride)) = &self.surface else { continue };
            if *current != ready.0 {
                continue;
            }
            if let Some(pixels) = ring.read(ready.1, ready.2) {
                return Frame { pixels, width: *width, height: *height, stride: *stride };
            }
        }
    }

    /// Frame-ring files the stage currently has in its runtime directory.
    pub fn ring_files(&self) -> Vec<String> {
        let dir = self.runtime_dir.join("backstage2d");
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Polls until `ring_files()` satisfies `f`.
    pub fn wait_for_ring_files(&self, f: impl Fn(&[String]) -> bool) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let files = self.ring_files();
            if f(&files) || Instant::now() > deadline {
                return files;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn close_socket(&mut self) {
        self.stream.take();
    }

    pub fn wait_exit(&mut self) -> ExitStatus {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "stage did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for StageHarness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.runtime_dir);
    }
}

fn accept(listener: &UnixListener) -> io::Result<UnixStream> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                return Ok(stream);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    }
}
