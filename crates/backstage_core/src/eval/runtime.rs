//! Runtime state: what the document doesn't say, e.g. which animations a
//! free-running instance is playing, since when, and how strongly. Small,
//! data-only, serializable.

use crate::id::{AnimId, CompId, NodeId};
use crate::node::{NodeKind, TimeMode};
use crate::project::Project;
use crate::time::Time;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Chain of `Instance` node IDs from the root composition to a composition
/// instance. The root itself is the empty path.
pub type InstancePath = Vec<NodeId>;

/// How strongly a layer contributes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Weight {
    Fixed(f32),
    /// Linear fade from `from` to `to` over `duration`, starting at `start`.
    Fade {
        from: f32,
        to: f32,
        start: Time,
        duration: Time,
    },
}

impl Weight {
    pub fn at(self, now: Time) -> f32 {
        match self {
            Weight::Fixed(w) => w,
            Weight::Fade { from, to, start, duration } => {
                if duration <= Time::ZERO {
                    return to;
                }
                let u = ((now - start).flicks() as f64 / duration.flicks() as f64).clamp(0.0, 1.0);
                from + (to - from) * u as f32
            }
        }
    }

    /// True once the weight can no longer change and is zero.
    fn finished_at_zero(self, now: Time) -> bool {
        match self {
            Weight::Fixed(w) => w <= 0.0,
            Weight::Fade { to, start, duration, .. } => to <= 0.0 && now >= start + duration,
        }
    }
}

/// One animation playing in a mixer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub animation: AnimId,
    /// Global time at which this layer's clock read zero.
    pub started: Time,
    pub weight: Weight,
}

/// The animations playing on one free-running composition instance, blended
/// by weight. Later layers win ties.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Mixer {
    pub layers: Vec<Layer>,
}

/// Everything `evaluate` needs besides the document and the time. An empty
/// state is valid: instances without a mixer play their configured (or
/// default) animation at full weight from time zero.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RuntimeState {
    pub mixers: BTreeMap<InstancePath, Mixer>,
}

impl RuntimeState {
    /// Switches the instance at `path` to `anim` immediately, from its start.
    pub fn play(&mut self, path: &InstancePath, anim: AnimId, now: Time) {
        let layer = Layer { animation: anim, started: now, weight: Weight::Fixed(1.0) };
        self.mixers.insert(path.clone(), Mixer { layers: vec![layer] });
    }

    /// Fades the instance at `path` over to `anim` across `duration`. Every
    /// other layer fades out from wherever it is. If `anim` is already a
    /// layer (say, still fading out), it fades back in without restarting.
    pub fn crossfade(
        &mut self,
        project: &Project,
        path: &InstancePath,
        anim: AnimId,
        now: Time,
        duration: Time,
    ) {
        if duration <= Time::ZERO {
            self.play(path, anim, now);
            return;
        }
        let mixer = self.mixers.entry(path.clone()).or_insert_with(|| implicit_mixer(project, path));
        let mut found = false;
        for layer in &mut mixer.layers {
            let from = layer.weight.at(now);
            let to = if layer.animation == anim && !found {
                found = true;
                1.0
            } else {
                0.0
            };
            layer.weight = Weight::Fade { from, to, start: now, duration };
        }
        if !found {
            mixer.layers.push(Layer {
                animation: anim,
                started: now,
                weight: Weight::Fade { from: 0.0, to: 1.0, start: now, duration },
            });
        }
    }

    /// Drops layers that have faded out for good.
    pub fn prune(&mut self, now: Time) {
        for mixer in self.mixers.values_mut() {
            mixer.layers.retain(|l| !l.weight.finished_at_zero(now));
        }
    }
}

/// The mixer an instance behaves as if it had when there's none in the
/// state: its configured (or default) animation at weight 1 from time 0.
pub(crate) fn implicit_mixer(project: &Project, path: &InstancePath) -> Mixer {
    let layers = default_animation(project, project.root, path)
        .map(|animation| Layer { animation, started: Time::ZERO, weight: Weight::Fixed(1.0) })
        .into_iter()
        .collect();
    Mixer { layers }
}

/// The animation a free instance (or the root, at the empty path) plays by
/// default: the instance's configured one, else its composition's default.
pub(crate) fn default_animation(project: &Project, root: CompId, path: &InstancePath) -> Option<AnimId> {
    let mut comp = project.compositions.get(&root)?;
    let mut configured = None;
    for node in path {
        let NodeKind::Instance(instance) = &comp.nodes.get(node)?.kind else { return None };
        configured = match instance.time {
            TimeMode::Free { animation } => animation,
            TimeMode::Synced { animation, .. } => Some(animation),
        };
        comp = project.compositions.get(&instance.comp)?;
    }
    configured.or(comp.default_animation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{self, ids::*};

    fn s(n: i64, d: i64) -> Time {
        Time::from_ratio(n, d)
    }

    #[test]
    fn weights_fade_linearly_and_clamp() {
        let w = Weight::Fade { from: 0.2, to: 1.0, start: s(1, 1), duration: s(1, 2) };
        assert_eq!(w.at(Time::ZERO), 0.2);
        assert_eq!(w.at(s(1, 1)), 0.2);
        assert!((w.at(s(5, 4)) - 0.6).abs() < 1e-6);
        assert_eq!(w.at(s(3, 2)), 1.0);
        assert_eq!(w.at(s(9, 1)), 1.0);
        assert_eq!(Weight::Fixed(0.3).at(s(5, 1)), 0.3);
    }

    #[test]
    fn default_animations_resolve_along_paths() {
        let p = sample::bounce();
        assert_eq!(default_animation(&p, p.root, &vec![]), Some(STAGE_MAIN));
        assert_eq!(default_animation(&p, p.root, &vec![FREE_BALL]), Some(BALL_BOUNCE));
        assert_eq!(default_animation(&p, p.root, &vec![GROUND]), None, "not an instance");
    }

    #[test]
    fn crossfade_from_implicit_default() {
        let p = sample::bounce();
        let mut state = RuntimeState::default();
        let path = vec![FREE_BALL];
        state.crossfade(&p, &path, BALL_SQUASH, s(1, 1), s(1, 5));
        let layers = &state.mixers[&path].layers;
        assert_eq!(layers.len(), 2);
        assert_eq!((layers[0].animation, layers[0].started), (BALL_BOUNCE, Time::ZERO));
        assert_eq!((layers[1].animation, layers[1].started), (BALL_SQUASH, s(1, 1)));
        assert_eq!(layers[0].weight.at(s(11, 10)), 0.5);
        assert_eq!(layers[1].weight.at(s(11, 10)), 0.5);
    }

    #[test]
    fn crossfading_back_reuses_the_layer() {
        let p = sample::bounce();
        let mut state = RuntimeState::default();
        let path = vec![FREE_BALL];
        state.crossfade(&p, &path, BALL_SQUASH, s(1, 1), s(1, 5));
        // Halfway through, go back to bounce: it fades up from 0.5, not restarting.
        state.crossfade(&p, &path, BALL_BOUNCE, s(11, 10), s(1, 5));
        let layers = &state.mixers[&path].layers;
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].started, Time::ZERO);
        assert_eq!(layers[0].weight.at(s(11, 10)), 0.5);
        assert_eq!(layers[0].weight.at(s(13, 10)), 1.0);
        assert_eq!(layers[1].weight.at(s(13, 10)), 0.0);
    }

    #[test]
    fn prune_drops_only_finished_fade_outs() {
        let p = sample::bounce();
        let mut state = RuntimeState::default();
        let path = vec![FREE_BALL];
        state.crossfade(&p, &path, BALL_SQUASH, s(1, 1), s(1, 5));
        state.prune(s(11, 10));
        assert_eq!(state.mixers[&path].layers.len(), 2, "still fading");
        state.prune(s(6, 5));
        let layers = &state.mixers[&path].layers;
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].animation, BALL_SQUASH);
    }

    #[test]
    fn zero_duration_crossfade_is_play() {
        let p = sample::bounce();
        let (mut a, mut b) = (RuntimeState::default(), RuntimeState::default());
        let path = vec![FREE_BALL];
        a.crossfade(&p, &path, BALL_SQUASH, s(1, 1), Time::ZERO);
        b.play(&path, BALL_SQUASH, s(1, 1));
        assert_eq!(a, b);
    }
}
