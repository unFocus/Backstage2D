//! The editor's end of `backstage_uidriver`, the test tool that gives a
//! headless UI test real pointer and keyboard input (see that crate). Only
//! the smoke test uses it: when the editor runs under the driver,
//! `BACKSTAGE_UI_DRIVER` names its socket.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

pub const DRIVER_ENV: &str = "BACKSTAGE_UI_DRIVER";

pub struct UiDriver {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl UiDriver {
    /// Connects to the driver named in the environment, if there is one.
    pub fn from_env() -> Option<Result<Self, String>> {
        let path = std::env::var_os(DRIVER_ENV)?;
        Some(
            UnixStream::connect(&path)
                .and_then(|writer| Ok(Self { reader: BufReader::new(writer.try_clone()?), writer }))
                .map_err(|e| format!("connecting to the UI driver at {}: {e}", path.to_string_lossy())),
        )
    }

    /// Sends one command and waits until the compositor has it. Blocks the
    /// caller for a roundtrip, which only a test run ever does.
    pub fn send(&mut self, command: &str) -> Result<(), String> {
        writeln!(self.writer, "{command}").map_err(|e| format!("UI driver: {e}"))?;
        let mut answer = String::new();
        self.reader.read_line(&mut answer).map_err(|e| format!("UI driver: {e}"))?;
        match answer.trim() {
            "ok" => Ok(()),
            "" => Err("UI driver: connection closed".into()),
            other => Err(format!("UI driver: {command}: {other}")),
        }
    }
}
