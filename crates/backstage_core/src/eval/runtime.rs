//! Runtime state: what the document doesn't say, e.g. when a free-running
//! instance started and what it's playing. Small, data-only, serializable.

use crate::id::{AnimId, NodeId};
use crate::time::Time;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Chain of `Instance` node IDs from the root composition to a composition
/// instance. The root itself is the empty path.
pub type InstancePath = Vec<NodeId>;

/// Playback of a free-running composition instance. Replaced by a mixer
/// (several weighted animations) in the next slice.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Player {
    /// Overrides the instance's configured animation.
    pub animation: Option<AnimId>,
    /// Global time at which its clock read zero.
    pub started: Time,
}

/// Everything `evaluate` needs besides the document and the time. An empty
/// state is valid: free instances play their configured (or default)
/// animation from time zero.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RuntimeState {
    pub players: BTreeMap<InstancePath, Player>,
}
