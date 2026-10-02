//! The root composition's clock: which animation it plays, where its
//! playhead is, and whether it's moving. Set by the editor with
//! `ToStage::Transport`; until then the stage plays its default animation
//! from zero.

use backstage_core::{AnimId, CompId, RuntimeState, Time};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq)]
pub struct Transport {
    /// The composition shown; `None` is the root.
    composition: Option<CompId>,
    animation: Option<AnimId>,
    /// The clock reading at `since`.
    time: Time,
    playing: bool,
    since: Instant,
}

impl Transport {
    /// Playing the default animation from zero, starting at `since`.
    pub fn new(since: Instant) -> Self {
        Self { composition: None, animation: None, time: Time::ZERO, playing: true, since }
    }

    /// The editor's playhead, as received at `since`.
    pub fn set(
        composition: Option<CompId>,
        animation: Option<AnimId>,
        time: Time,
        playing: bool,
        since: Instant,
    ) -> Self {
        Self { composition, animation, time, playing, since }
    }

    /// The composition to show; `None` is the root.
    pub fn composition(&self) -> Option<CompId> {
        self.composition
    }

    /// The clock reading at `at`.
    pub fn now(&self, at: Instant) -> Time {
        if !self.playing {
            return self.time;
        }
        let elapsed = at.saturating_duration_since(self.since);
        self.time + Time::from_ratio(elapsed.as_nanos() as i64, 1_000_000_000)
    }

    /// What `evaluate` needs: the chosen animation on the shown
    /// composition (the empty instance path), from time zero. No mixer means
    /// its default animation.
    pub fn state(&self) -> RuntimeState {
        let mut state = RuntimeState::default();
        if let Some(anim) = self.animation {
            state.play(&Vec::new(), anim, Time::ZERO);
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::Transport;
    use backstage_core::sample::ids::*;
    use backstage_core::{Layer, Time, Weight};
    use std::time::{Duration, Instant};

    #[test]
    fn plays_the_default_from_zero_until_told_otherwise() {
        let t0 = Instant::now();
        let transport = Transport::new(t0);
        assert_eq!(transport.now(t0), Time::ZERO);
        assert_eq!(transport.now(t0 + Duration::from_millis(500)), Time::from_ratio(1, 2));
        assert!(transport.state().mixers.is_empty(), "the root's default animation");
    }

    #[test]
    fn a_paused_clock_stays_put_and_a_playing_one_advances() {
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(3);
        let paused = Transport::set(None, None, Time::from_secs(1), false, t0);
        assert_eq!(paused.now(later), Time::from_secs(1));
        let playing = Transport::set(None, None, Time::from_secs(1), true, t0);
        assert_eq!(playing.now(later), Time::from_secs(4));
        assert_eq!(playing.now(t0 - Duration::from_secs(1)), Time::from_secs(1), "never runs backwards");
    }

    #[test]
    fn the_chosen_animation_plays_on_the_root() {
        let transport = Transport::set(None, Some(STAGE_MAIN), Time::ZERO, false, Instant::now());
        let state = transport.state();
        assert_eq!(
            state.mixers[&Vec::new()].layers,
            [Layer { animation: STAGE_MAIN, started: Time::ZERO, weight: Weight::Fixed(1.0) }]
        );
    }
}
