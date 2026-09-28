//! Mapping a clock to animation-local time. Exact integer math on flicks.

use crate::animation::{Animation, LoopMode};
use crate::node::Repeat;
use crate::time::Time;

/// Local time within `anim` for a clock reading, honoring the loop mode,
/// the instance's `repeat`, and the animation's `step`. Always within
/// `0..=duration`.
pub fn local_time(anim: &Animation, clock: Time, repeat: Repeat) -> Time {
    let d = anim.duration.flicks().max(1);
    let c = clock.flicks();
    let mode = match repeat {
        Repeat::Natural => anim.looping,
        Repeat::Once => LoopMode::Once,
        Repeat::Hold(at) => return step(anim, Time::from_flicks(at.flicks().clamp(0, d))),
    };
    let local = match mode {
        LoopMode::Once => c.clamp(0, d),
        LoopMode::Loop => c.rem_euclid(d),
        LoopMode::PingPong => {
            let p = c.rem_euclid(2 * d);
            if p <= d { p } else { 2 * d - p }
        }
    };
    step(anim, Time::from_flicks(local))
}

fn step(anim: &Animation, local: Time) -> Time {
    match anim.step {
        Some(s) if s > Time::ZERO => {
            let (l, s) = (local.flicks(), s.flicks());
            Time::from_flicks(l - l.rem_euclid(s))
        }
        _ => local,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn anim(mode: LoopMode) -> Animation {
        Animation::new("a", Time::from_secs(2), mode)
    }
    fn s(num: i64, den: i64) -> Time {
        Time::from_ratio(num, den)
    }

    #[test]
    fn once_clamps() {
        let a = anim(LoopMode::Once);
        assert_eq!(local_time(&a, s(-1, 1), Repeat::Natural), Time::ZERO);
        assert_eq!(local_time(&a, s(3, 2), Repeat::Natural), s(3, 2));
        assert_eq!(local_time(&a, s(5, 1), Repeat::Natural), s(2, 1));
    }

    #[test]
    fn loop_wraps_exactly() {
        let a = anim(LoopMode::Loop);
        assert_eq!(local_time(&a, s(2, 1), Repeat::Natural), Time::ZERO);
        assert_eq!(local_time(&a, s(1000, 1), Repeat::Natural), Time::ZERO);
        assert_eq!(local_time(&a, s(5, 2), Repeat::Natural), s(1, 2));
        assert_eq!(local_time(&a, s(-1, 2), Repeat::Natural), s(3, 2));
    }

    #[test]
    fn ping_pong_reflects() {
        let a = anim(LoopMode::PingPong);
        assert_eq!(local_time(&a, s(1, 1), Repeat::Natural), s(1, 1));
        assert_eq!(local_time(&a, s(2, 1), Repeat::Natural), s(2, 1));
        assert_eq!(local_time(&a, s(5, 2), Repeat::Natural), s(3, 2));
        assert_eq!(local_time(&a, s(4, 1), Repeat::Natural), Time::ZERO);
        assert_eq!(local_time(&a, s(-1, 2), Repeat::Natural), s(1, 2));
    }

    #[test]
    fn repeat_overrides_loop_mode() {
        let a = anim(LoopMode::Loop);
        assert_eq!(local_time(&a, s(5, 1), Repeat::Once), s(2, 1));
        assert_eq!(local_time(&a, s(5, 1), Repeat::Hold(s(1, 4))), s(1, 4));
        assert_eq!(local_time(&a, s(5, 1), Repeat::Hold(s(9, 1))), s(2, 1), "hold is clamped");
    }

    #[test]
    fn step_quantizes() {
        let mut a = anim(LoopMode::Loop);
        a.step = Some(s(1, 12));
        assert_eq!(local_time(&a, s(1, 10), Repeat::Natural), s(1, 12));
        assert_eq!(local_time(&a, s(1, 13), Repeat::Natural), Time::ZERO);
        assert_eq!(local_time(&a, s(2, 12), Repeat::Natural), s(2, 12));
    }

    proptest! {
        #[test]
        fn always_within_duration(clock in any::<i64>(), mode in 0..3u8, dur in 1i64..10_000_000_000) {
            let mode = [LoopMode::Once, LoopMode::Loop, LoopMode::PingPong][mode as usize];
            let a = Animation::new("a", Time::from_flicks(dur), mode);
            let t = local_time(&a, Time::from_flicks(clock / 4), Repeat::Natural);
            prop_assert!(t >= Time::ZERO && t <= a.duration);
        }
    }
}
