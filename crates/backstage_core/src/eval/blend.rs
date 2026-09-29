//! Blending one property's values from several weighted animation layers.

use crate::animation::{Property, Value};
use crate::time::Time;

/// Blends `(value, weight)` contributions for `property` over its `rest`
/// value. Returns `None` if nothing contributes (weight ≤ 0 everywhere).
///
/// - Numeric values: with total weight `W`, a normalized average when
///   `W ≥ 1`, otherwise `rest·(1 − W) + Σ wᵢvᵢ` (the remainder falls back
///   to rest). A crossfade is exactly `(1 − f)·A + f·B`.
/// - Angles (rotation, skew) take the shortest arc.
/// - Discrete values come from the strongest layer; later layers win ties.
pub fn blend(property: Property, rest: Value, contributions: &[(Value, f32)]) -> Option<Value> {
    let active: Vec<(Value, f32)> = contributions.iter().copied().filter(|(_, w)| *w > 0.0).collect();
    if active.is_empty() {
        return None;
    }
    if property.is_discrete() {
        return strongest(active.iter().copied());
    }
    let total: f32 = active.iter().map(|(_, w)| w).sum();
    let mix = |rest: f64, values: &mut dyn Iterator<Item = (f64, f32)>| -> f64 {
        let sum: f64 = values.map(|(v, w)| v * w as f64).sum();
        if total >= 1.0 { sum / total as f64 } else { rest * (1.0 - total as f64) + sum }
    };
    match rest {
        Value::Number(rest) => {
            let is_angle = matches!(property, Property::Rotation | Property::SkewX | Property::SkewY);
            let first = number(active[0].0)?;
            // Unwrap angles to within ±180° of the first contributor, and
            // the rest value too, so every step takes the shortest arc.
            let unwrap = |v: f32| if is_angle { first + wrap_degrees(v - first) } else { v };
            let mut values = active.iter().filter_map(|(v, w)| number(*v).map(|n| (unwrap(n) as f64, *w)));
            Some(Value::Number(mix(unwrap(rest) as f64, &mut values) as f32))
        }
        Value::Rgba(rest) => Some(Value::Rgba(std::array::from_fn(|i| {
            let mut values = active.iter().filter_map(|(v, w)| match v {
                Value::Rgba(c) => Some((c[i] as f64, *w)),
                _ => None,
            });
            mix(rest[i] as f64, &mut values) as f32
        }))),
        Value::Time(rest) => {
            let mut values = active.iter().filter_map(|(v, w)| match v {
                Value::Time(t) => Some((t.flicks() as f64, *w)),
                _ => None,
            });
            Some(Value::Time(Time::from_flicks(mix(rest.flicks() as f64, &mut values).round() as i64)))
        }
        _ => strongest(active.iter().copied()),
    }
}

/// The value with the highest weight; later entries win ties.
pub fn strongest<T>(items: impl Iterator<Item = (T, f32)>) -> Option<T> {
    let mut best: Option<(T, f32)> = None;
    for (item, weight) in items {
        if best.as_ref().is_none_or(|(_, w)| weight >= *w) {
            best = Some((item, weight));
        }
    }
    best.map(|(item, _)| item)
}

fn number(v: Value) -> Option<f32> {
    match v {
        Value::Number(n) => Some(n),
        _ => None,
    }
}

/// Wraps an angle difference into (-180°, 180°].
fn wrap_degrees(d: f32) -> f32 {
    let w = (d + 180.0).rem_euclid(360.0) - 180.0;
    if w == -180.0 { 180.0 } else { w }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::BlendMode;

    fn n(v: Option<Value>) -> f32 {
        match v {
            Some(Value::Number(n)) => n,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn crossfade_endpoints_are_exact() {
        let (a, b) = (Value::Number(10.0), Value::Number(30.0));
        let rest = Value::Number(-5.0);
        assert_eq!(n(blend(Property::X, rest, &[(a, 1.0), (b, 0.0)])), 10.0);
        assert_eq!(n(blend(Property::X, rest, &[(a, 0.0), (b, 1.0)])), 30.0);
        assert_eq!(n(blend(Property::X, rest, &[(a, 0.5), (b, 0.5)])), 20.0);
        assert_eq!(n(blend(Property::X, rest, &[(a, 1.0)])), 10.0);
    }

    #[test]
    fn partial_weight_falls_back_to_rest() {
        assert_eq!(n(blend(Property::X, Value::Number(0.0), &[(Value::Number(10.0), 0.5)])), 5.0);
        assert_eq!(blend(Property::X, Value::Number(0.0), &[(Value::Number(10.0), 0.0)]), None);
    }

    #[test]
    fn over_unity_weights_normalize() {
        let v =
            blend(Property::X, Value::Number(0.0), &[(Value::Number(10.0), 1.0), (Value::Number(20.0), 1.0)]);
        assert_eq!(n(v), 15.0);
    }

    #[test]
    fn angles_take_the_shortest_arc() {
        let v = n(blend(
            Property::Rotation,
            Value::Number(0.0),
            &[(Value::Number(350.0), 0.5), (Value::Number(10.0), 0.5)],
        ));
        assert!(v.abs() < 1e-4 || (v - 360.0).abs() < 1e-4, "{v}");
        let v = n(blend(
            Property::Rotation,
            Value::Number(0.0),
            &[(Value::Number(170.0), 0.5), (Value::Number(-170.0), 0.5)],
        ));
        assert!((v.rem_euclid(360.0) - 180.0).abs() < 1e-4, "{v}");
    }

    #[test]
    fn discrete_takes_the_strongest_later_wins_ties() {
        let rest = Value::Blend(BlendMode::Normal);
        let a = (Value::Blend(BlendMode::Add), 0.7);
        let b = (Value::Blend(BlendMode::Screen), 0.3);
        assert_eq!(blend(Property::Blend, rest, &[a, b]), Some(a.0));
        let tie = [(Value::Index(1), 0.5), (Value::Index(2), 0.5)];
        assert_eq!(blend(Property::Drawing, Value::Index(0), &tie), Some(Value::Index(2)));
    }

    #[test]
    fn rgba_and_time_blend() {
        let v = blend(
            Property::ColorAdd,
            Value::Rgba([0.0; 4]),
            &[(Value::Rgba([1.0, 0.0, 0.0, 0.0]), 0.5), (Value::Rgba([0.0, 1.0, 0.0, 0.0]), 0.5)],
        );
        assert_eq!(v, Some(Value::Rgba([0.5, 0.5, 0.0, 0.0])));
        let v =
            blend(Property::TimeOffset, Value::Time(Time::ZERO), &[(Value::Time(Time::from_secs(2)), 0.25)]);
        assert_eq!(v, Some(Value::Time(Time::from_ratio(1, 2))));
    }
}
