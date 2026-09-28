//! Easing curves.

use crate::animation::Ease;

/// Maps progress `u` (clamped to 0..=1) through `ease`. `Hold` stays at 0 so
/// the value holds until the next key.
pub fn ease(ease: Ease, u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    match ease {
        Ease::Hold => 0.0,
        Ease::Linear => u,
        Ease::CubicBezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, u),
        Ease::Preset(p) => {
            let (x1, y1, x2, y2) = p.control_points();
            cubic_bezier(x1, y1, x2, y2, u)
        }
    }
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` at `x`: find the curve parameter `t`
/// whose x is `x`, then return its y. Like CSS, x control points are clamped
/// to 0..=1 so the curve is a function of x. Endpoints are exact.
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let (x1, x2) = (x1.clamp(0.0, 1.0) as f64, x2.clamp(0.0, 1.0) as f64);
    let (y1, y2, x) = (y1 as f64, y2 as f64, x as f64);
    // B(t) = 3(1-t)^2 t p1 + 3(1-t) t^2 p2 + t^3, written as a t^3 + b t^2 + c t.
    let coeffs = |p1: f64, p2: f64| {
        let c = 3.0 * p1;
        let b = 3.0 * (p2 - p1) - c;
        (1.0 - c - b, b, c)
    };
    let (ax, bx, cx) = coeffs(x1, x2);
    let (ay, by, cy) = coeffs(y1, y2);
    let sample_x = |t: f64| ((ax * t + bx) * t + cx) * t;
    let slope_x = |t: f64| (3.0 * ax * t + 2.0 * bx) * t + cx;

    // Newton's method from t = x, then bisection if it doesn't converge.
    let mut t = x;
    for _ in 0..8 {
        let err = sample_x(t) - x;
        if err.abs() < 1e-7 {
            return (((ay * t + by) * t + cy) * t) as f32;
        }
        let d = slope_x(t);
        if d.abs() < 1e-6 {
            break;
        }
        t -= err / d;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    t = x;
    for _ in 0..60 {
        let err = sample_x(t) - x;
        if err.abs() < 1e-7 {
            break;
        }
        if err > 0.0 {
            hi = t;
        } else {
            lo = t;
        }
        t = (lo + hi) / 2.0;
    }
    (((ay * t + by) * t + cy) * t) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::EasePreset;
    use proptest::prelude::*;

    #[test]
    fn endpoints_are_exact() {
        for e in [
            Ease::Linear,
            Ease::Preset(EasePreset::EaseIn),
            Ease::Preset(EasePreset::EaseOut),
            Ease::Preset(EasePreset::EaseInOut),
            Ease::CubicBezier(0.3, -0.5, 0.7, 1.5),
        ] {
            assert_eq!(ease(e, 0.0), 0.0, "{e:?}");
            assert_eq!(ease(e, 1.0), 1.0, "{e:?}");
        }
        assert_eq!(ease(Ease::Hold, 0.99), 0.0);
    }

    #[test]
    fn ease_in_out_is_symmetric() {
        let e = Ease::Preset(EasePreset::EaseInOut);
        assert!((ease(e, 0.5) - 0.5).abs() < 1e-5);
        for u in [0.1f32, 0.25, 0.4] {
            assert!((ease(e, u) + ease(e, 1.0 - u) - 1.0).abs() < 1e-5, "{u}");
        }
    }

    /// Brute force: sample the curve densely and interpolate.
    fn reference(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
        let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
        let point = |t: f32, p1: f32, p2: f32| {
            let s = 1.0 - t;
            3.0 * s * s * t * p1 + 3.0 * s * t * t * p2 + t * t * t
        };
        let n = 20_000;
        let mut prev = (0.0f32, 0.0f32);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let cur = (point(t, x1, x2), point(t, y1, y2));
            if cur.0 >= x {
                let span = (cur.0 - prev.0).max(1e-9);
                return prev.1 + (cur.1 - prev.1) * (x - prev.0) / span;
            }
            prev = cur;
        }
        1.0
    }

    proptest! {
        #[test]
        fn matches_reference(x1 in 0.0f32..1.0, y1 in -1.0f32..2.0, x2 in 0.0f32..1.0, y2 in -1.0f32..2.0, x in 0.001f32..0.999) {
            let got = cubic_bezier(x1, y1, x2, y2, x);
            let want = reference(x1, y1, x2, y2, x);
            prop_assert!((got - want).abs() < 1e-3, "got {got}, want {want}");
        }

        #[test]
        fn monotonic_for_monotonic_curves(x1 in 0.0f32..1.0, y1 in 0.0f32..1.0, x2 in 0.0f32..1.0, y2 in 0.0f32..1.0, a in 0.0f32..1.0, b in 0.0f32..1.0) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(cubic_bezier(x1, y1, x2, y2, lo) <= cubic_bezier(x1, y1, x2, y2, hi) + 1e-5);
        }
    }
}
