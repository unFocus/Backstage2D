//! `backstage_uidriver`: real pointer and keyboard input, and screenshots,
//! for UI tests in a headless wlroots compositor. A test tool, never
//! shipped.
//!
//! ```sh
//! cage -- backstage_uidriver [--script FILE] -- APP [ARGS…]
//! ```
//!
//! It starts `APP` with `BACKSTAGE_UI_DRIVER` set to a socket. Each line
//! written there (or read from `--script`) is a command, answered with
//! `ok` or `error: …` once the compositor has handled it:
//!
//! - `move X Y`: move the pointer to output pixel (X, Y).
//! - `down`, `up`: press or release the primary button.
//! - `click X Y [shift] [ctrl] [alt]`: move, press, and release, holding the
//!   modifiers.
//! - `key NAME down|up`: shift, ctrl, alt, escape, enter, delete, up, down,
//!   left, right.
//! - `shot PATH`: capture the output as a PNG.
//! - `wait MS`: pause (scripts).
//!
//! The driver exits with the app's status. With `--script`, it runs the
//! script, then stops the app and exits 0 (or 1 if a command failed).

mod commands;
mod wayland;

use anyhow::{Context, Result, bail};
use commands::{Command, Key};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, ExitCode};
use std::sync::mpsc;
use std::time::Duration;

/// The socket the app is given, as an environment variable.
pub const DRIVER_ENV: &str = "BACKSTAGE_UI_DRIVER";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("backstage_uidriver: {e:#}");
            ExitCode::FAILURE
        }
    }
}

struct Args {
    script: Option<PathBuf>,
    app: Vec<String>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args> {
    let usage = "usage: backstage_uidriver [--script FILE] -- APP [ARGS…]";
    let mut args = args.peekable();
    let mut script = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--script" => script = Some(PathBuf::from(args.next().context(usage)?)),
            "--" => break,
            other => bail!("unknown argument {other:?}; {usage}"),
        }
    }
    let app: Vec<String> = args.collect();
    if app.is_empty() {
        bail!("{usage}");
    }
    Ok(Args { script, app })
}

/// A command from the app, and where its answer goes.
type Request = (String, mpsc::Sender<String>);

fn run() -> Result<ExitCode> {
    let Args { script, app } = parse_args(std::env::args().skip(1))?;
    let mut driver = wayland::Driver::connect()?;

    let dir = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    let socket = PathBuf::from(dir).join(format!("backstage-uidriver-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).with_context(|| format!("binding {}", socket.display()))?;
    let (tx, requests) = mpsc::channel::<Request>();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || serve(stream, tx));
        }
    });

    let mut child = std::process::Command::new(&app[0])
        .args(&app[1..])
        .env(DRIVER_ENV, &socket)
        .spawn()
        .with_context(|| format!("starting {}", app[0]))?;
    let result = match script {
        Some(path) => run_script(&mut driver, &path, &mut child),
        None => serve_app(&mut driver, &requests, &mut child),
    };
    let _ = std::fs::remove_file(&socket);
    if result.is_err() || child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

/// Carries out the app's commands until it exits, then exits likewise.
fn serve_app(
    driver: &mut wayland::Driver,
    requests: &mpsc::Receiver<Request>,
    child: &mut Child,
) -> Result<ExitCode> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(match status.code() {
                Some(0) => ExitCode::SUCCESS,
                Some(code) => ExitCode::from(code.clamp(1, 255) as u8),
                None => ExitCode::FAILURE, // killed by a signal
            });
        }
        if let Ok((line, reply)) = requests.recv_timeout(Duration::from_millis(50)) {
            let result = commands::parse(&line).and_then(|c| match c {
                Some(Command::Wait(_)) => bail!("wait is for scripts; the app waits on its own"),
                Some(c) => execute(driver, c),
                None => Ok(()),
            });
            let answer = match result {
                Ok(()) => "ok".to_owned(),
                Err(e) => format!("error: {e:#}"),
            };
            let _ = reply.send(answer);
        }
    }
}

fn run_script(driver: &mut wayland::Driver, path: &std::path::Path, child: &mut Child) -> Result<ExitCode> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    for (n, line) in text.lines().enumerate() {
        if let Some(status) = child.try_wait()? {
            bail!("the app exited ({status}) before line {}", n + 1);
        }
        let command = commands::parse(line).with_context(|| format!("line {}", n + 1))?;
        if let Some(command) = command {
            execute(driver, command).with_context(|| format!("line {}: {line}", n + 1))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn execute(driver: &mut wayland::Driver, command: Command) -> Result<()> {
    match command {
        Command::Move { x, y } => driver.move_to(x, y),
        Command::Button { pressed } => driver.button(pressed),
        Command::Click { x, y, mods } => {
            let held: Vec<Key> = [(mods.shift, Key::Shift), (mods.ctrl, Key::Ctrl), (mods.alt, Key::Alt)]
                .into_iter()
                .filter_map(|(on, key)| on.then_some(key))
                .collect();
            for &key in &held {
                driver.key(key, true)?;
            }
            driver.move_to(x, y)?;
            driver.button(true)?;
            driver.button(false)?;
            for &key in held.iter().rev() {
                driver.key(key, false)?;
            }
            Ok(())
        }
        Command::Key { key, pressed } => driver.key(key, pressed),
        Command::Shot(path) => driver.screenshot(&path),
        Command::Wait(ms) => {
            std::thread::sleep(Duration::from_millis(ms));
            Ok(())
        }
    }
}

/// Reads one app connection's lines, answering each in turn.
fn serve(stream: UnixStream, tx: mpsc::Sender<Request>) {
    let Ok(mut writer) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        let (reply_tx, reply) = mpsc::channel();
        if tx.send((line, reply_tx)).is_err() {
            return;
        }
        let Ok(answer) = reply.recv() else { return };
        if writeln!(writer, "{answer}").is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_args;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn parses_the_app_and_an_optional_script() {
        let a = parse_args(args(&["--", "editor", "-x"])).unwrap();
        assert_eq!((a.script, a.app), (None, vec!["editor".to_owned(), "-x".to_owned()]));
        let a = parse_args(args(&["--script", "s.txt", "--", "editor"])).unwrap();
        assert_eq!(a.script.as_deref(), Some(std::path::Path::new("s.txt")));
        for bad in [&[][..], &["--"], &["editor"], &["--script"], &["--script", "s", "--"]] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?}");
        }
    }
}
