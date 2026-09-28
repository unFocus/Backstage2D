//! Time in flicks: 705,600,000 per second, which divides exactly into every
//! common frame rate and audio sample rate. See ADR 0003.
//!
//! In files, times are exact, human-readable strings: `"2s"`, `"0.25s"`,
//! `"7/24s"`, or raw flicks such as `"123456f"`.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::fmt;
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};
use std::str::FromStr;

/// Flicks per second.
pub const FLICKS_PER_SECOND: i64 = 705_600_000;

/// Denominators tried, in order, when writing a time as a fraction.
const FRACTION_DENOMINATORS: [i64; 8] = [24, 30, 48, 60, 90, 120, 144, 240];
/// Decimal digits tried when writing a time as decimal seconds.
const MAX_DECIMALS: u32 = 6;

/// A point in time or a duration, in flicks.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(i64);

impl Time {
    pub const ZERO: Time = Time(0);
    pub const MAX: Time = Time(i64::MAX);

    pub const fn from_flicks(flicks: i64) -> Self {
        Self(flicks)
    }

    pub const fn from_secs(secs: i64) -> Self {
        Self(secs * FLICKS_PER_SECOND)
    }

    /// `num / den` seconds, rounded to the nearest flick (exact whenever the
    /// ratio is a whole number of flicks, e.g. 1/24 s or 1/48000 s).
    pub fn from_ratio(num: i64, den: i64) -> Self {
        assert!(den != 0, "zero denominator");
        Self(div_round(num as i128 * FLICKS_PER_SECOND as i128, den as i128) as i64)
    }

    pub fn from_secs_f64(secs: f64) -> Self {
        Self((secs * FLICKS_PER_SECOND as f64).round() as i64)
    }

    pub const fn flicks(self) -> i64 {
        self.0
    }

    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / FLICKS_PER_SECOND as f64
    }

    pub fn is_negative(self) -> bool {
        self.0 < 0
    }
}

/// Rounds `n / d` to the nearest integer, halves away from zero.
fn div_round(n: i128, d: i128) -> i128 {
    let (n, d) = if d < 0 { (-n, -d) } else { (n, d) };
    if n >= 0 { (n + d / 2) / d } else { (n - d / 2) / d }
}

impl Add for Time {
    type Output = Time;
    fn add(self, rhs: Time) -> Time {
        Time(self.0 + rhs.0)
    }
}

impl Sub for Time {
    type Output = Time;
    fn sub(self, rhs: Time) -> Time {
        Time(self.0 - rhs.0)
    }
}

impl AddAssign for Time {
    fn add_assign(&mut self, rhs: Time) {
        self.0 += rhs.0;
    }
}

impl SubAssign for Time {
    fn sub_assign(&mut self, rhs: Time) {
        self.0 -= rhs.0;
    }
}

impl Neg for Time {
    type Output = Time;
    fn neg(self) -> Time {
        Time(-self.0)
    }
}

impl Mul<i64> for Time {
    type Output = Time;
    fn mul(self, rhs: i64) -> Time {
        Time(self.0 * rhs)
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let flicks = self.0 as i128;
        let fps = FLICKS_PER_SECOND as i128;
        let sign = if flicks < 0 { "-" } else { "" };
        let abs = flicks.abs();

        // Whole or decimal seconds, if exact within MAX_DECIMALS digits.
        let scale = 10i128.pow(MAX_DECIMALS);
        if (abs * scale) % fps == 0 {
            let scaled = abs * scale / fps;
            let (whole, frac) = (scaled / scale, scaled % scale);
            if frac == 0 {
                return write!(f, "{sign}{whole}s");
            }
            let digits = format!("{frac:0width$}", width = MAX_DECIMALS as usize);
            return write!(f, "{sign}{whole}.{}s", digits.trim_end_matches('0'));
        }
        // A fraction with a common frame-rate denominator.
        for den in FRACTION_DENOMINATORS {
            let unit = fps / den as i128;
            if abs % unit == 0 {
                return write!(f, "{sign}{}/{den}s", abs / unit);
            }
        }
        write!(f, "{}f", self.0)
    }
}

impl fmt::Debug for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid time {input:?}: {reason}")]
pub struct ParseTimeError {
    input: String,
    reason: &'static str,
}

impl FromStr for Time {
    type Err = ParseTimeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = |reason| ParseTimeError { input: s.to_owned(), reason };
        if let Some(flicks) = s.strip_suffix('f') {
            return flicks.parse().map(Time).map_err(|_| err("bad flick count"));
        }
        let secs = s.strip_suffix('s').ok_or_else(|| err("expected an 's' or 'f' suffix"))?;
        let (negative, secs) = match secs.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, secs),
        };
        if secs.is_empty() || secs.starts_with(['+', '-']) {
            return Err(err("bad number"));
        }
        let fps = FLICKS_PER_SECOND as i128;
        let flicks: i128 = if let Some((num, den)) = secs.split_once('/') {
            let num: i128 = num.parse().map_err(|_| err("bad numerator"))?;
            let den: i128 = den.parse().map_err(|_| err("bad denominator"))?;
            if den <= 0 || num < 0 {
                return Err(err("bad fraction"));
            }
            if (num * fps) % den != 0 {
                return Err(err("not a whole number of flicks"));
            }
            num * fps / den
        } else {
            let (whole, frac) = secs.split_once('.').unwrap_or((secs, ""));
            let digits_ok = |d: &str| d.bytes().all(|b| b.is_ascii_digit());
            if whole.is_empty() || !digits_ok(whole) || !digits_ok(frac) || frac.len() > 18 {
                return Err(err("bad number"));
            }
            let whole: i128 = whole.parse().map_err(|_| err("number too large"))?;
            let frac_value: i128 = if frac.is_empty() { 0 } else { frac.parse().unwrap() };
            let scale = 10i128.pow(frac.len() as u32);
            if (frac_value * fps) % scale != 0 {
                return Err(err("not a whole number of flicks"));
            }
            whole * fps + frac_value * fps / scale
        };
        let flicks = if negative { -flicks } else { flicks };
        i64::try_from(flicks).map(Time).map_err(|_| err("out of range"))
    }
}

impl Serialize for Time {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Time {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(de::Error::custom)
    }
}

/// A snapping grid of `ticks_per_second` evenly spaced ticks. Only an editing
/// aid: it never affects playback speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TimeGrid {
    pub ticks_per_second: u32,
}

impl TimeGrid {
    pub const fn new(ticks_per_second: u32) -> Self {
        Self { ticks_per_second }
    }

    /// Duration of one tick.
    pub fn tick(self) -> Time {
        Time::from_ratio(1, self.ticks_per_second as i64)
    }

    /// The nearest tick to `t`.
    pub fn snap(self, t: Time) -> Time {
        let tps = self.ticks_per_second as i128;
        let ticks = div_round(t.0 as i128 * tps, FLICKS_PER_SECOND as i128);
        Time::from_ratio(ticks as i64, tps as i64)
    }
}

impl Default for TimeGrid {
    fn default() -> Self {
        Self::new(60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn common_rates_are_whole_flicks() {
        for rate in [24, 25, 30, 48, 50, 60, 90, 100, 120, 144, 240, 44_100, 48_000, 96_000] {
            assert_eq!(FLICKS_PER_SECOND % rate, 0, "1/{rate} s is not exact");
            assert_eq!(Time::from_ratio(1, rate) * rate, Time::from_secs(1));
        }
    }

    #[test]
    fn display_picks_the_most_readable_exact_form() {
        assert_eq!(Time::from_secs(2).to_string(), "2s");
        assert_eq!(Time::from_ratio(1, 4).to_string(), "0.25s");
        assert_eq!(Time::from_ratio(-3, 2).to_string(), "-1.5s");
        assert_eq!(Time::from_ratio(7, 24).to_string(), "7/24s");
        assert_eq!(Time::from_ratio(1, 60).to_string(), "1/60s");
        assert_eq!(Time::from_ratio(1, 30).to_string(), "1/30s");
        assert_eq!(Time::from_flicks(1).to_string(), "1f");
        assert_eq!(Time::ZERO.to_string(), "0s");
    }

    #[test]
    fn parse_accepts_all_forms() {
        assert_eq!("2s".parse(), Ok(Time::from_secs(2)));
        assert_eq!("0.5s".parse(), Ok(Time::from_ratio(1, 2)));
        assert_eq!("-0.5s".parse(), Ok(Time::from_ratio(-1, 2)));
        assert_eq!("7/24s".parse(), Ok(Time::from_ratio(7, 24)));
        assert_eq!("1/48000s".parse(), Ok(Time::from_ratio(1, 48_000)));
        assert_eq!("123f".parse(), Ok(Time::from_flicks(123)));
    }

    #[test]
    fn parse_rejects_malformed_or_inexact() {
        for bad in ["", "2", "s", "1.s5", "+1s", "--1s", "1/0s", "1/11s", "0.0000000001s", "x1s", "1e3s"] {
            assert!(bad.parse::<Time>().is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn grid_snaps_to_nearest_tick() {
        let g60 = TimeGrid::new(60);
        assert_eq!(g60.snap(Time::from_ratio(1, 50)), Time::from_ratio(1, 60));
        assert_eq!(g60.snap(Time::from_ratio(1, 200)), Time::ZERO);
        let g30 = TimeGrid::new(30);
        assert_eq!(g30.snap(Time::from_ratio(1, 50)), Time::from_ratio(1, 30));
        assert_eq!(g30.snap(Time::from_secs(-1) - Time::from_ratio(1, 100)), Time::from_secs(-1));
        assert_eq!(g30.tick(), Time::from_ratio(1, 30));
    }

    proptest! {
        #[test]
        fn string_form_round_trips(flicks in any::<i64>()) {
            let t = Time::from_flicks(flicks);
            prop_assert_eq!(t.to_string().parse::<Time>(), Ok(t));
        }

        #[test]
        fn frame_times_round_trip(n in -100_000i64..100_000, rate in prop::sample::select(vec![24i64, 30, 60, 120])) {
            let t = Time::from_ratio(n, rate);
            prop_assert_eq!(t.to_string().parse::<Time>(), Ok(t));
            prop_assert!(!t.to_string().ends_with('f'), "{} should be readable", t);
        }
    }
}
