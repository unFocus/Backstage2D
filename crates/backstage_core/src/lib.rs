//! Backstage2D core: the document model (projects, compositions, animations)
//! and time. See `docs/adr/0003-document-model.md`.
//!
//! This crate must not depend on any renderer or GUI framework. See
//! `docs/architecture.md`.

pub mod animation;
pub mod command;
pub mod document;
pub mod editor;
pub mod eval;
pub mod geom;
pub mod id;
pub mod io;
pub mod node;
pub mod project;
pub mod sample;
pub mod shape;
pub mod time;
pub mod validate;

pub use animation::{Animation, Ease, EasePreset, Key, LoopMode, Marker, Property, Track, Value, ValueKind};
pub use command::{Command, CommandError};
pub use document::{Document, Entry, EntryError, ReplayError};
pub use eval::{
    DrawContent, DrawItem, InstancePath, Layer, Mixer, RuntimeState, Scene, Weight, evaluate, evaluate_from,
};
pub use geom::{Color, ColorTransform, Transform, Vec2};
pub use id::{AnimId, AssetId, CompId, DrawingId, NodeId, ParseIdError};
pub use io::{LoadError, SaveError, from_files, load, save, to_files};
pub use node::{BlendMode, Drawing, Instance, Node, NodeFlags, NodeKind, Props, Repeat, TimeMode};
pub use project::{Asset, AssetKind, Composition, EditorPrefs, FORMAT_VERSION, Project, ProjectSettings};
pub use shape::{GradientStop, LineCap, LineJoin, Paint, PathCmd, Shape, Stroke, StyledPath};
pub use time::{FLICKS_PER_SECOND, ParseTimeError, Time, TimeGrid};
pub use validate::ValidationError;

#[cfg(test)]
pub(crate) mod test_util;
