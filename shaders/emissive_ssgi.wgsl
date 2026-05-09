// Half-res gather: emissive-only buffer + depth → cheap one-bounce-style bleed for shop/hallway GLB.
// Fixed camera + slow scene motion: no temporal history in v1.

struct EmissiveGiParams {
    inv_view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    /// xy = full-res width/height (pixels); zw unused
    screen: vec4<f32>,
    /// x = strength scale (artist), y = depth edge sharpness, z = tap radius (full-res px), w unused
    tuning: vec4<f32>,
}

@group(0) @binding(0) var<uniform> gi: EmissiveGiParams;
@group(0) @binding(1) var emissive_tex: texture_2d<f32>;
@group(0) @binding(2) var depth_tex: texture_depth_2d;
@group(0) @binding(3) var samp: sampler;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    let p = pos[vid];
    var out: VsOut;
    out.clip_pos = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    return out;
}

fn world_at_uv_depth(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0, depth);
    let world = gi.inv_view_proj * vec4<f32>(ndc, 1.0);
    return world.xyz / max(world.w, 1e-6);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let full_w = max(gi.screen.x, 1.0);
    let full_h = max(gi.screen.y, 1.0);
    let half_w = max(i32(full_w * 0.5), 1);
    let half_h = max(i32(full_h * 0.5), 1);

    let hx = i32(clamp(in.uv.x * f32(half_w), 0.0, f32(half_w - 1)));
    let hy = i32(clamp(in.uv.y * f32(half_h), 0.0, f32(half_h - 1)));
    // Top-left pixel of the 2×2 full-res block this half-res texel represents.
    let pxf = f32(hx * 2);
    let pyf = f32(hy * 2);
    let center_uv = vec2<f32>((pxf + 0.5) / full_w, (pyf + 0.5) / full_h);
    let center_px = vec2<i32>(
        i32(clamp(pxf, 0.0, full_w - 1.0)),
        i32(clamp(pyf, 0.0, full_h - 1.0)),
    );

    let d_c = textureLoad(depth_tex, center_px, 0);
    if (d_c >= 0.9999) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    let p_c = world_at_uv_depth(center_uv, d_c);
    let str = gi.tuning.x;
    let depth_k = gi.tuning.y;
    let r_px = max(gi.tuning.z, 1.0);

    let offs = array<vec2<f32>, 8>(
        vec2<f32>(1.2, 0.4),
        vec2<f32>(-0.9, 1.1),
        vec2<f32>(1.6, -1.0),
        vec2<f32>(-1.3, -0.6),
        vec2<f32>(0.2, 2.0),
        vec2<f32>(-2.1, 0.1),
        vec2<f32>(0.8, -1.8),
        vec2<f32>(-0.4, -2.2),
    );

    var acc = vec3<f32>(0.0);
    let base_xy = vec2<f32>(pxf, pyf);
    for (var i: u32 = 0u; i < 8u; i = i + 1u) {
        let o = offs[i] * r_px;
        let tap_xy = base_xy + o;
        let tap_uv = tap_xy / vec2<f32>(full_w, full_h);
        if (tap_uv.x < 0.0 || tap_uv.x > 1.0 || tap_uv.y < 0.0 || tap_uv.y > 1.0) {
            continue;
        }
        let tap_px = vec2<i32>(
            i32(clamp(tap_uv.x * full_w - 0.5, 0.0, full_w - 1.0)),
            i32(clamp(tap_uv.y * full_h - 0.5, 0.0, full_h - 1.0)),
        );
        let d_t = textureLoad(depth_tex, tap_px, 0);
        if (d_t >= 0.9999) {
            continue;
        }
        let e = textureSampleLevel(emissive_tex, samp, tap_uv, 0.0).rgb;
        if (dot(e, e) < 1e-10) {
            continue;
        }
        let w_d = exp(-abs(d_t - d_c) * depth_k);
        let p_t = world_at_uv_depth(tap_uv, d_t);
        let dist = length(p_t - p_c);
        let w_dist = 1.0 / (1.0 + dist * dist * 0.015);
        acc = acc + e * w_d * w_dist;
    }

    let out_rgb = acc * (str / 8.0);
    return vec4<f32>(max(out_rgb, vec3<f32>(0.0)), 1.0);
}
