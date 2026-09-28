//! Projects and compositions.

use crate::animation::Animation;
use crate::geom::Color;
use crate::id::{AnimId, AssetId, CompId, NodeId};
use crate::node::Node;
use crate::time::TimeGrid;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Current version of the on-disk project format.
pub const FORMAT_VERSION: u32 = 1;

/// The document: a library of compositions and assets.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub settings: ProjectSettings,
    pub editor: EditorPrefs,
    /// The composition shown on the stage.
    pub root: CompId,
    pub compositions: BTreeMap<CompId, Composition>,
    pub assets: BTreeMap<AssetId, Asset>,
}

impl Project {
    pub fn root_composition(&self) -> Option<&Composition> {
        self.compositions.get(&self.root)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    pub stage_width: u32,
    pub stage_height: u32,
    pub background: Color,
    /// Whole-pixel positions and nearest-neighbor bitmap sampling.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pixel_art: bool,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self { stage_width: 550, stage_height: 400, background: Color::WHITE, pixel_art: false }
    }
}

/// Editing aids stored with the project. None of these affect playback.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPrefs {
    /// Keys snap to this many ticks per second.
    pub time_grid: TimeGrid,
    pub time_snap: bool,
    /// Spatial grid spacing in stage pixels.
    pub spatial_grid: f32,
    pub spatial_snap: bool,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self { time_grid: TimeGrid::new(60), time_snap: true, spatial_grid: 10.0, spatial_snap: false }
    }
}

/// A reusable unit: a fixed node tree plus named animations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Composition {
    pub id: CompId,
    pub name: String,
    pub root: NodeId,
    pub nodes: BTreeMap<NodeId, Node>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub animations: BTreeMap<AnimId, Animation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_animation: Option<AnimId>,
}

impl Composition {
    pub fn root_node(&self) -> Option<&Node> {
        self.nodes.get(&self.root)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    /// Relative to the project directory, e.g. `assets/logo.png`.
    pub path: PathBuf,
    pub kind: AssetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    Bitmap,
    Font,
    Audio,
}

fn is_false(v: &bool) -> bool {
    !*v
}
