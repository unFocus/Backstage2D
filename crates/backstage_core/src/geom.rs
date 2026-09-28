//! Geometry and color value types. Coordinates are stage pixels with y down;
//! angles are degrees.

pub use glam::Vec2;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::fmt;
use std::str::FromStr;

/// An 8-bit-authored sRGB color with straight alpha, held as `f32` in 0..=1.
/// Written as `"#rrggbbaa"` (or `"#rrggbb"` when opaque) in files.
#[derive(Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const WHITE: Color = Color::rgb8(255, 255, 255);
    pub const BLACK: Color = Color::rgb8(0, 0, 0);
    pub const TRANSPARENT: Color = Color::rgba8(0, 0, 0, 0);

    pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
    }

    pub const fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Self::rgba8(r, g, b, 255)
    }

    pub fn to_rgba8(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b, a] = self.to_rgba8();
        if a == 255 {
            write!(f, "#{r:02x}{g:02x}{b:02x}")
        } else {
            write!(f, "#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }
}

impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid color {0:?}: expected #rrggbb or #rrggbbaa")]
pub struct ParseColorError(String);

impl FromStr for Color {
    type Err = ParseColorError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseColorError(s.to_owned());
        let hex = s.strip_prefix('#').filter(|h| h.is_ascii()).ok_or_else(err)?;
        if hex.len() != 6 && hex.len() != 8 {
            return Err(err());
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| err());
        let a = if hex.len() == 8 { byte(6)? } else { 255 };
        Ok(Color::rgba8(byte(0)?, byte(2)?, byte(4)?, a))
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(de::Error::custom)
    }
}

/// A node's local transform. Applied as: move by `-pivot`, scale, skew,
/// rotate, then move to `position`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    #[serde(skip_serializing_if = "is_zero")]
    pub position: Vec2,
    /// Degrees, clockwise (y is down).
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub rotation: f32,
    #[serde(skip_serializing_if = "is_one")]
    pub scale: Vec2,
    /// Degrees.
    #[serde(skip_serializing_if = "is_zero")]
    pub skew: Vec2,
    /// The point (in local coordinates) that `position` places and that
    /// rotation and scale pivot around. Flash's registration point.
    #[serde(skip_serializing_if = "is_zero")]
    pub pivot: Vec2,
}

impl Default for Transform {
    fn default() -> Self {
        Self { position: Vec2::ZERO, rotation: 0.0, scale: Vec2::ONE, skew: Vec2::ZERO, pivot: Vec2::ZERO }
    }
}

impl Transform {
    pub fn at(x: f32, y: f32) -> Self {
        Self { position: Vec2::new(x, y), ..Self::default() }
    }
}

/// Per-channel `color * multiply + add`, in straight-alpha 0..=1 units (like
/// Flash's ColorTransform). Values may go outside 0..=1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorTransform {
    #[serde(skip_serializing_if = "is_identity_multiply")]
    pub multiply: [f32; 4],
    #[serde(skip_serializing_if = "is_zero_add")]
    pub add: [f32; 4],
}

impl Default for ColorTransform {
    fn default() -> Self {
        Self { multiply: [1.0; 4], add: [0.0; 4] }
    }
}

impl ColorTransform {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

fn is_zero(v: &Vec2) -> bool {
    *v == Vec2::ZERO
}
fn is_one(v: &Vec2) -> bool {
    *v == Vec2::ONE
}
fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}
fn is_identity_multiply(v: &[f32; 4]) -> bool {
    *v == [1.0; 4]
}
fn is_zero_add(v: &[f32; 4]) -> bool {
    *v == [0.0; 4]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_hex_round_trips() {
        for s in ["#ff8800", "#00000000", "#12345678", "#ffffff"] {
            assert_eq!(s.parse::<Color>().unwrap().to_string(), s);
        }
        assert_eq!("#FF8800".parse::<Color>().unwrap().to_string(), "#ff8800");
        for bad in ["ff8800", "#ff88", "#ff88000", "#gg0000", "#ffé000"] {
            assert!(bad.parse::<Color>().is_err(), "{bad}");
        }
    }
}
