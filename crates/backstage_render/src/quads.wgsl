// Instanced, rotated solid quads in physical pixel coordinates.

struct Globals {
    viewport: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;

struct Quad {
    @location(0) center: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) angle: f32,
    @location(3) color: vec4<f32>,
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32, quad: Quad) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
        vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0),
    );
    let local = corners[index] * quad.half_size;
    let s = sin(quad.angle);
    let c = cos(quad.angle);
    let p = quad.center + vec2(local.x * c - local.y * s, local.x * s + local.y * c);
    let ndc = vec2(p.x / globals.viewport.x * 2.0 - 1.0, 1.0 - p.y / globals.viewport.y * 2.0);

    var out: VertexOut;
    out.position = vec4(ndc, 0.0, 1.0);
    out.color = quad.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}
