//! Sampling keyframe tracks.

use super::ease::ease;
use crate::animation::{Track, Value};
use crate::time::Time;

/// The track's value at `local` time: the first key's value before it, the
/// last key's after it, and in between the eased interpolation from key `i`
/// (using key `i`'s ease) to key `i + 1`. Discrete values hold.
pub fn sample_track(track: &Track, local: Time) -> Option<Value> {
    let keys = &track.keys;
    let first = keys.first()?;
    let after = keys.partition_point(|k| k.at <= local);
    if after == 0 {
        return Some(first.value);
    }
    let a = &keys[after - 1];
    let Some(b) = keys.get(after) else { return Some(a.value) };
    if track.property.is_discrete() {
        return Some(a.value);
    }
    let span = (b.at - a.at).flicks() as f64;
    let u = ((local - a.at).flicks() as f64 / span) as f32;
    Some(interpolate(a.value, b.value, ease(a.ease, u)))
}

fn lerp(a: f32, b: f32, w: f32) -> f32 {
    a + (b - a) * w
}

/// Interpolates matching values; mismatched or discrete ones hold `a`.
pub fn interpolate(a: Value, b: Value, w: f32) -> Value {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => Value::Number(lerp(a, b, w)),
        (Value::Rgba(a), Value::Rgba(b)) => Value::Rgba(std::array::from_fn(|i| lerp(a[i], b[i], w))),
        (Value::Time(a), Value::Time(b)) => {
            let d = (b - a).flicks() as f64 * w as f64;
            Value::Time(a + Time::from_flicks(d.round() as i64))
        }
        _ => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Ease, Key, Property};
    use crate::id::NodeId;

    fn track(property: Property, keys: Vec<Key>) -> Track {
        Track { node: NodeId::from_raw(1), property, keys }
    }
    fn s(n: i64, d: i64) -> Time {
        Time::from_ratio(n, d)
    }
    fn num(v: Option<Value>) -> f32 {
        match v {
            Some(Value::Number(n)) => n,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn before_after_and_on_keys() {
        let t = track(
            Property::X,
            vec![
                Key::new(s(1, 1), Value::Number(10.0), Ease::Linear),
                Key::new(s(2, 1), Value::Number(20.0), Ease::Linear),
            ],
        );
        assert_eq!(num(sample_track(&t, Time::ZERO)), 10.0);
        assert_eq!(num(sample_track(&t, s(1, 1))), 10.0);
        assert_eq!(num(sample_track(&t, s(3, 2))), 15.0);
        assert_eq!(num(sample_track(&t, s(2, 1))), 20.0);
        assert_eq!(num(sample_track(&t, s(9, 1))), 20.0);
        assert_eq!(sample_track(&track(Property::X, vec![]), Time::ZERO), None);
    }

    #[test]
    fn hold_ease_and_discrete_properties_hold() {
        let t = track(
            Property::X,
            vec![
                Key::new(Time::ZERO, Value::Number(0.0), Ease::Hold),
                Key::new(s(1, 1), Value::Number(1.0), Ease::Linear),
            ],
        );
        assert_eq!(num(sample_track(&t, s(99, 100))), 0.0);
        let t = track(
            Property::Visible,
            vec![
                Key::new(Time::ZERO, Value::Bool(true), Ease::Linear),
                Key::new(s(1, 1), Value::Bool(false), Ease::Linear),
            ],
        );
        assert_eq!(sample_track(&t, s(99, 100)), Some(Value::Bool(true)));
        assert_eq!(sample_track(&t, s(1, 1)), Some(Value::Bool(false)));
    }

    #[test]
    fn rgba_and_time_interpolate() {
        let t = track(
            Property::ColorAdd,
            vec![
                Key::new(Time::ZERO, Value::Rgba([0.0; 4]), Ease::Linear),
                Key::new(s(1, 1), Value::Rgba([1.0, 0.5, 0.0, 0.0]), Ease::Linear),
            ],
        );
        assert_eq!(sample_track(&t, s(1, 2)), Some(Value::Rgba([0.5, 0.25, 0.0, 0.0])));
        let t = track(
            Property::TimeOffset,
            vec![
                Key::new(Time::ZERO, Value::Time(Time::ZERO), Ease::Linear),
                Key::new(s(1, 1), Value::Time(s(2, 1)), Ease::Linear),
            ],
        );
        assert_eq!(sample_track(&t, s(1, 4)), Some(Value::Time(s(1, 2))));
    }
}
