// Scene items: tessellated shapes in local coordinates, placed by a per-item
// transform, painted by solid colors or gradients, then color-transformed.

struct Globals {
    viewport: vec2<f32>,
}

// Must match `GpuPaint` in tessellate.rs (192 bytes).
struct Paint {
    kind: u32,        // 0 solid, 1 linear, 2 radial
    stop_count: u32,
    radius: f32,
    _pad: f32,
    p0: vec2<f32>,
    p1: vec2<f32>,
    stops: array<vec4<f32>, 8>,
    offsets: array<vec4<f32>, 2>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<storage, read> paints: array<Paint>;

struct VertexIn {
    @location(0) pos: vec2<f32>,
    @location(1) paint: u32,
    // Per item: world transform columns, opacity, color transform.
    @location(2) m0: vec2<f32>,
    @location(3) m1: vec2<f32>,
    @location(4) m2: vec2<f32>,
    @location(5) multiply: vec4<f32>,
    @location(6) add: vec4<f32>,
    @location(7) opacity: f32,
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) paint: u32,
    @location(2) @interpolate(flat) opacity: f32,
    @location(3) @interpolate(flat) multiply: vec4<f32>,
    @location(4) @interpolate(flat) add: vec4<f32>,
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let p = v.m0 * v.pos.x + v.m1 * v.pos.y + v.m2;
    let ndc = vec2(p.x / globals.viewport.x * 2.0 - 1.0, 1.0 - p.y / globals.viewport.y * 2.0);
    var out: VertexOut;
    out.position = vec4(ndc, 0.0, 1.0);
    out.local = v.pos;
    out.paint = v.paint;
    out.opacity = v.opacity;
    out.multiply = v.multiply;
    out.add = v.add;
    return out;
}

fn stop_offset(p: Paint, i: u32) -> f32 {
    return p.offsets[i / 4u][i % 4u];
}

fn paint_color(p: Paint, local: vec2<f32>) -> vec4<f32> {
    if p.kind == 0u || p.stop_count <= 1u {
        return p.stops[0];
    }
    var t: f32;
    if p.kind == 1u {
        let d = p.p1 - p.p0;
        t = dot(local - p.p0, d) / max(dot(d, d), 1e-12);
    } else {
        t = length(local - p.p0) / max(p.radius, 1e-6);
    }
    t = clamp(t, 0.0, 1.0);
    if t <= stop_offset(p, 0u) {
        return p.stops[0];
    }
    for (var i = 1u; i < p.stop_count; i++) {
        let hi = stop_offset(p, i);
        if t <= hi {
            let lo = stop_offset(p, i - 1u);
            return mix(p.stops[i - 1u], p.stops[i], (t - lo) / max(hi - lo, 1e-6));
        }
    }
    return p.stops[p.stop_count - 1u];
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    var c = clamp(paint_color(paints[in.paint], in.local) * in.multiply + in.add, vec4(0.0), vec4(1.0));
    c.a *= in.opacity;
    return c;
}
