//! Helpers shared by unit tests.

use crate::animation::{Ease, Key, LoopMode, Property, Value, ValueKind};
use crate::command::{Command, Command::*};
use crate::id::{AnimId, NodeId};
use crate::node::{Instance, Node, NodeFlags, NodeKind, Props, TimeMode};
use crate::project::Project;
use crate::time::Time;
use proptest::prelude::*;
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

/// Random numbers that [`pick`] turns into a command.
#[derive(Debug, Clone, Copy)]
pub struct Seed {
    which: u8,
    n: [usize; 4],
    x: f32,
}

pub fn seed() -> impl Strategy<Value = Seed> {
    (any::<u8>(), any::<[usize; 4]>(), -100.0f32..100.0).prop_map(|(which, n, x)| Seed { which, n, x })
}

/// Picks concrete commands for the current project from random
/// numbers, so most of them refer to things that exist. Some are
/// still invalid, which is fine: they must fail cleanly.
pub fn pick(p: &Project, seed: &Seed) -> Command {
    let Seed { which, n, x } = *seed;
    let nth = |len: usize, i: usize| i % len.max(1);
    let comps: Vec<_> = p.compositions.keys().copied().collect();
    let comp = comps[nth(comps.len(), n[0])];
    let c = &p.compositions[&comp];
    let nodes: Vec<_> = c.nodes.keys().copied().collect();
    let other = nodes[nth(nodes.len(), n[2])];
    let anims: Vec<_> = c.animations.keys().copied().collect();
    let anim = anims.get(nth(anims.len(), n[2])).copied().unwrap_or(AnimId::from_raw(1));
    // Half the time, aim at an existing track so key and track edits
    // find something to work on.
    let tracks = c.animations.get(&anim).map_or(&[][..], |a| &a.tracks[..]);
    let (node, property) = match tracks.get(nth(tracks.len(), n[1] / 2)) {
        Some(t) if n[1] % 2 == 1 => (t.node, t.property),
        _ => (nodes[nth(nodes.len(), n[1])], Property::ALL[nth(Property::ALL.len(), n[3])]),
    };
    let duration = c.animations.get(&anim).map_or(Time::from_secs(1), |a| a.duration);
    let at = Time::from_flicks(duration.flicks() / 8 * (n[3] % 9) as i64);
    let value = match property.value_kind() {
        ValueKind::Number => Value::Number(x),
        ValueKind::Bool => Value::Bool(x > 0.0),
        ValueKind::Rgba => Value::Rgba([x, 0.5, 0.5, 1.0]),
        ValueKind::Blend => Value::Blend(Default::default()),
        ValueKind::Index => Value::Index(n[3] as u32 % 3),
        ValueKind::Time => Value::Time(at),
    };
    let children = c.nodes[&other].children.len();
    match which % 10 {
        0 => SetRest { comp, node, rest: Props::at(x, -x) },
        1 => SetKey { comp, anim, node, property, key: Key::new(at, value, Ease::Linear) },
        2 => {
            let at = c
                .animations
                .get(&anim)
                .and_then(|a| a.track(node, property))
                .map_or(at, |t| t.keys[nth(t.keys.len(), n[3])].at);
            RemoveKey { comp, anim, node, property, at }
        }
        3 => RemoveTrack { comp, anim, node, property },
        4 => {
            let id = NodeId::from_raw(0x00ee_0000 + n[0] as u64 % 64);
            let kind = if n[3].is_multiple_of(4) {
                NodeKind::Instance(Instance {
                    comp: comps[nth(comps.len(), n[1])],
                    time: TimeMode::Free { animation: None },
                })
            } else {
                NodeKind::Group
            };
            AddNode { comp, parent: other, index: nth(children + 1, n[3]), id, node: Node::new("n", kind) }
        }
        5 => RemoveNode { comp, node },
        6 => MoveNode { comp, node, parent: other, index: nth(children + 1, n[3]) },
        7 => RenameNode { comp, node, name: format!("n{}", n[3]) },
        8 => SetNodeFlags {
            comp,
            node,
            flags: NodeFlags { hidden: n[3] % 2 == 1, locked: n[3] % 3 == 1, outline: n[3] % 5 == 1 },
        },
        _ => SetAnimationTiming {
            comp,
            anim,
            duration: Time::from_flicks(duration.flicks() * (1 + n[3] as i64 % 3) / 2),
            looping: LoopMode::Loop,
            step: None,
        },
    }
}
