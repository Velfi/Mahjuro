// Circular hold-to-act progress ring. One square instance per ring; the
// fragment shader masks an annular arc (clockwise from top) with AA.
//
// Instance layout matches `ArcRingQuadInstance`:
//   rect, fill_color, track_color, params
// params.x = inner radius in normalized UV (outer edge is |uv| = 1)
// params.y = fill progress in [0, 1]
// params.z = 1 when the hold action is invalid (pulsing red ring, no progress)

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) fill_color: vec4<f32>,
    @location(1) track_color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) params: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) fill_color: vec4<f32>,
    @location(3) track_color: vec4<f32>,
    @location(4) params: vec4<f32>,
) -> VsOut {
    let x = rect.x + corner.x * rect.z;
    let y = rect.y + corner.y * rect.w;
    let nx = (x / globals.screen.x) * 2.0 - 1.0;
    let ny = 1.0 - (y / globals.screen.y) * 2.0;
    var out: VsOut;
    out.clip_position = vec4<f32>(nx, ny, 0.0, 1.0);
    out.fill_color = fill_color;
    out.track_color = track_color;
    out.uv = corner * 2.0 - vec2<f32>(1.0, 1.0);
    out.params = params;
    return out;
}

fn ring_mask(dist: f32, inner_r: f32, outer_r: f32) -> f32 {
    let aa = max(fwidth(dist) * 1.6, 0.001);
    let outer = 1.0 - smoothstep(outer_r - aa, outer_r + aa, dist);
    let inner = smoothstep(inner_r - aa, inner_r + aa, dist);
    return outer * inner;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dist = length(in.uv);
    let inner_r = clamp(in.params.x, 0.0, 0.999);
    let progress = clamp(in.params.y, 0.0, 1.0);

    let band = ring_mask(dist, inner_r, 1.0);
    if band <= 0.0 {
        discard;
    }

    // Clockwise from top (-PI/2).
    let ang = atan2(in.uv.y, in.uv.x);
    const START: f32 = -1.57079632679;
    const TAU: f32 = 6.28318530718;
    var rel = (ang - START) / TAU;
    if rel < 0.0 {
        rel += 1.0;
    }

    let invalid = in.params.z > 0.5;
    let edge = max(fwidth(rel) * 2.0, 0.004);
    var fill_w = 1.0 - smoothstep(progress - edge, progress + edge, rel);
    var col = mix(in.track_color, in.fill_color, fill_w);
    if invalid {
        let pulse = 0.52 + 0.48 * sin(globals.time * 12.0);
        col = mix(in.track_color, in.fill_color, pulse * band);
    }

    let inv_g = 1.0 / max(globals.gamma, 0.01);
    let rgb = pow(col.rgb, vec3<f32>(inv_g));
    return vec4<f32>(rgb, col.a * band);
}
