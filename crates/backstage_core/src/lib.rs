//! Backstage2D core: the document model (projects, compositions, animations)
//! and time. See `docs/adr/0003-document-model.md`.
//!
//! This crate must not depend on any renderer or GUI framework. See
//! `docs/architecture.md`.

pub mod animation;
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
pub use eval::{
    DrawContent, DrawItem, InstancePath, Layer, Mixer, RuntimeState, Scene, Weight, evaluate, evaluate_from,
};
pub use geom::{Color, ColorTransform, Transform, Vec2};
pub use id::{AnimId, AssetId, CompId, DrawingId, NodeId, ParseIdError};
pub use io::{LoadError, SaveError, load, save};
pub use node::{BlendMode, Drawing, Instance, Node, NodeKind, Props, Repeat, TimeMode};
pub use project::{Asset, AssetKind, Composition, EditorPrefs, FORMAT_VERSION, Project, ProjectSettings};
pub use shape::{GradientStop, LineCap, LineJoin, Paint, PathCmd, Shape, Stroke, StyledPath};
pub use time::{FLICKS_PER_SECOND, ParseTimeError, Time, TimeGrid};
pub use validate::ValidationError;

#[cfg(test)]
pub(crate) mod test_util {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fresh, empty directory under the system temp dir.
    pub fn tempdir(name: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "backstage-core-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
