//! The driver's line commands, parsed. See the crate docs for the list.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;

/// Modifier keys held during a click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// A key the driver can press, as its Linux evdev code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Shift,
    Ctrl,
    Alt,
    Escape,
    Enter,
    Delete,
    Up,
    Down,
    Left,
    Right,
}

impl Key {
    fn parse(name: &str) -> Result<Key> {
        Ok(match name {
            "shift" => Key::Shift,
            "ctrl" => Key::Ctrl,
            "alt" => Key::Alt,
            "escape" => Key::Escape,
            "enter" => Key::Enter,
            "delete" => Key::Delete,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            _ => bail!("unknown key {name:?}"),
        })
    }

    /// `linux/input-event-codes.h`.
    pub fn evdev(self) -> u32 {
        match self {
            Key::Escape => 1,
            Key::Enter => 28,
            Key::Ctrl => 29,
            Key::Shift => 42,
            Key::Alt => 56,
            Key::Up => 103,
            Key::Left => 105,
            Key::Right => 106,
            Key::Down => 108,
            Key::Delete => 111,
        }
    }

    /// The xkb modifier bit it sets in the standard keymap, if it's a
    /// modifier: Shift, Control, Mod1.
    pub fn modifier_mask(self) -> u32 {
        match self {
            Key::Shift => 1,
            Key::Ctrl => 4,
            Key::Alt => 8,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Move the pointer to output coordinates (logical pixels).
    Move {
        x: f64,
        y: f64,
    },
    /// Press or release the primary button.
    Button {
        pressed: bool,
    },
    /// Move there, then press and release, holding `mods`.
    Click {
        x: f64,
        y: f64,
        mods: Mods,
    },
    Key {
        key: Key,
        pressed: bool,
    },
    /// Capture the output as a PNG.
    Shot(PathBuf),
    /// Pause (scripts only).
    Wait(u64),
}

/// Parses one line. Blank lines and `#` comments are `None`.
pub fn parse(line: &str) -> Result<Option<Command>> {
    let line = line.split('#').next().unwrap_or_default().trim();
    let mut words = line.split_whitespace();
    let Some(verb) = words.next() else { return Ok(None) };
    let rest: Vec<&str> = words.collect();
    let num = |i: usize| -> Result<f64> {
        let word = rest.get(i).with_context(|| format!("{verb}: missing argument {}", i + 1))?;
        word.parse().with_context(|| format!("{verb}: {word:?} is not a number"))
    };
    let command = match verb {
        "move" => Command::Move { x: num(0)?, y: num(1)? },
        "down" => Command::Button { pressed: true },
        "up" => Command::Button { pressed: false },
        "click" => {
            let mut mods = Mods::default();
            for word in rest.iter().skip(2) {
                match *word {
                    "shift" => mods.shift = true,
                    "ctrl" => mods.ctrl = true,
                    "alt" => mods.alt = true,
                    other => bail!("click: unknown modifier {other:?}"),
                }
            }
            Command::Click { x: num(0)?, y: num(1)?, mods }
        }
        "key" => {
            let name = rest.first().context("key: missing key name")?;
            let pressed = match rest.get(1).copied() {
                Some("down") => true,
                Some("up") => false,
                other => bail!("key: expected down or up, got {other:?}"),
            };
            Command::Key { key: Key::parse(name)?, pressed }
        }
        "shot" => Command::Shot(PathBuf::from(rest.first().context("shot: missing path")?)),
        "wait" => Command::Wait(num(0)? as u64),
        _ => bail!("unknown command {verb:?}"),
    };
    let expected = match &command {
        Command::Move { .. } => 2,
        Command::Button { .. } => 0,
        Command::Click { .. } => rest.len(), // modifiers checked above
        Command::Key { .. } => 2,
        Command::Shot(_) | Command::Wait(_) => 1,
    };
    if rest.len() != expected {
        bail!("{verb}: expected {expected} arguments, got {}", rest.len());
    }
    Ok(Some(command))
}

/// Screen pixels from wl_shm's `argb8888` / `xrgb8888` (little-endian, so
/// B, G, R, A in memory) to RGBA, dropping each row's padding.
pub fn bgra_to_rgba(pixels: &[u8], width: u32, height: u32, stride: u32, opaque: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for row in pixels.chunks(stride as usize).take(height as usize) {
        for px in row[..width as usize * 4].as_chunks::<4>().0 {
            out.extend_from_slice(&[px[2], px[1], px[0], if opaque { 255 } else { px[3] }]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_command() {
        assert_eq!(parse("move 10 20.5").unwrap(), Some(Command::Move { x: 10.0, y: 20.5 }));
        assert_eq!(parse("down").unwrap(), Some(Command::Button { pressed: true }));
        assert_eq!(parse("up").unwrap(), Some(Command::Button { pressed: false }));
        assert_eq!(
            parse("click 1 2 shift ctrl").unwrap(),
            Some(Command::Click { x: 1.0, y: 2.0, mods: Mods { shift: true, ctrl: true, alt: false } })
        );
        assert_eq!(
            parse("click 1 2").unwrap(),
            Some(Command::Click { x: 1.0, y: 2.0, mods: Mods::default() })
        );
        assert_eq!(parse("key escape down").unwrap(), Some(Command::Key { key: Key::Escape, pressed: true }));
        assert_eq!(parse("shot /tmp/a.png").unwrap(), Some(Command::Shot("/tmp/a.png".into())));
        assert_eq!(parse("  wait 250  # settle").unwrap(), Some(Command::Wait(250)));
        assert_eq!(parse("").unwrap(), None);
        assert_eq!(parse("# just a comment").unwrap(), None);
    }

    #[test]
    fn rejects_bad_lines() {
        for bad in [
            "jump",
            "move 1",
            "move 1 x",
            "move 1 2 3",
            "down now",
            "click 1 2 meta",
            "key f13 down",
            "key shift",
            "shot",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn modifier_keys_have_their_xkb_bits() {
        assert_eq!(Key::Shift.modifier_mask(), 1);
        assert_eq!(Key::Ctrl.modifier_mask(), 4);
        assert_eq!(Key::Alt.modifier_mask(), 8);
        assert_eq!(Key::Escape.modifier_mask(), 0);
    }

    #[test]
    fn screen_pixels_become_rgba_without_padding() {
        // 2 × 1 pixels in a 12-byte row: blue, then half-transparent red.
        let row = [255, 0, 0, 7, 0, 0, 255, 128, 9, 9, 9, 9];
        assert_eq!(bgra_to_rgba(&row, 2, 1, 12, true), [0, 0, 255, 255, 255, 0, 0, 255]);
        assert_eq!(bgra_to_rgba(&row, 2, 1, 12, false), [0, 0, 255, 7, 255, 0, 0, 128]);
    }
}
