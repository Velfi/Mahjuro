// Procedural golden-dust vignette with divine light rays.
//
// Fullscreen triangle — no vertex buffers. Fine golden motes float
// slowly in shafts of light radiating from top-center. God-rays are
// modulated by noise for organic breakup. Reverent, restrained mood.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
    cursor_pos: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// ── Fullscreen triangle ───────────────────────────────────────────────
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 3.0,  1.0),
    );
    let p = pos[vid];
    var out: VsOut;
    out.clip_pos = vec4<f32>(p, 0.9999, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    return out;
}

// ── Hash helpers ──────────────────────────────────────────────────────
fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    let h = vec2<f32>(
        dot(p, vec2<f32>(127.1, 311.7)),
        dot(p, vec2<f32>(269.5, 183.3)),
    );
    return fract(sin(h) * 43758.5453);
}

// ── Smooth noise (value noise with bicubic interpolation) ─────────────
fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f); // smoothstep interpolant

    let a = hash21(i + vec2<f32>(0.0, 0.0));
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));

    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// ── Fractal brownian motion (2 octaves — enough for soft breakup) ─────
fn fbm2(p: vec2<f32>) -> f32 {
    var val = 0.0;
    val += value_noise(p) * 0.6;
    val += value_noise(p * 2.1 + 3.7) * 0.4;
    return val;
}

// ── God-ray radial light ──────────────────────────────────────────────
// Rays emanate from a source point (top-center). Each ray's brightness
// is modulated by noise so they look organic rather than uniform.
fn god_rays(uv: vec2<f32>, aspect: f32, time: f32) -> f32 {
    let source = vec2<f32>(0.5, -0.05); // just above top edge
    let delta = (uv - source) * vec2<f32>(aspect, 1.0);

    // Distance from source (rays fade with distance)
    let dist = length(delta);

    // Angular noise to break rays into fingers of light.
    // Use normalised direction as a 2D noise coordinate instead of
    // raw atan2 — avoids the -pi/+pi seam that would cause a hard
    // discontinuity straight below the source.
    let dir = delta / max(dist, 0.0001);
    let noise_coord = vec2<f32>(dir.x * 3.0 + time * 0.08,
                                dir.y * 3.0 - time * 0.06);
    let ray_noise = fbm2(noise_coord * 4.0);

    // Sharp ray fingers with soft edges
    let ray_pattern = smoothstep(0.3, 0.7, ray_noise);

    // Radial falloff — strongest near source, fading outward
    let radial_fade = smoothstep(1.6, 0.1, dist);

    // Vertical bias — rays are strongest in upper half
    let vertical_fade = smoothstep(1.0, 0.2, uv.y);

    // Gentle overall pulse
    let pulse = 0.85 + 0.15 * sin(time * 0.4);

    return ray_pattern * radial_fade * vertical_fade * pulse * 0.12;
}

// ── Dust mote layer ───────────────────────────────────────────────────
// Grid-based floating particles. Each mote drifts slowly and brightens
// when it crosses a god-ray (checked via the same radial angle noise).
fn dust_layer(uv: vec2<f32>, scale: f32, density: f32, size: f32,
              time: f32) -> f32 {
    // Gentle drift
    let drifted = vec2<f32>(
        uv.x + sin(time * 0.15) * 0.01,
        uv.y - time * 0.008
    );
    let grid_uv = drifted * scale;
    let cell = floor(grid_uv);
    let frac_uv = fract(grid_uv);

    let rng = hash22(cell);
    if rng.x > density { return 0.0; }

    // Mote position with slow individual wander, clamped to stay
    // within [0.05, 0.95] so motes don't bleed across cell boundaries.
    let phase = hash21(cell + 53.7) * 6.2831853;
    let wander_x = sin(time * 0.3 + phase) * 0.10;
    let wander_y = cos(time * 0.25 + phase * 1.3) * 0.08;
    let mote_pos = clamp(
        vec2<f32>(0.15 + rng.x * 0.7 + wander_x,
                  0.15 + rng.y * 0.7 + wander_y),
        vec2<f32>(0.05), vec2<f32>(0.95)
    );

    let d = length(frac_uv - mote_pos);

    // Soft point with glow
    let brightness = smoothstep(size, size * 0.1, d);
    let glow = smoothstep(size * 3.5, size * 0.3, d) * 0.2;

    // Twinkle
    let twinkle_phase = hash21(cell + 77.1) * 6.2831853;
    let twinkle = 0.6 + 0.4 * sin(time * (1.0 + rng.y * 1.5) + twinkle_phase);

    return (brightness + glow) * twinkle;
}

// ── Fragment main ─────────────────────────────────────────────────────
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = globals.time;
    let aspect = globals.screen.x / globals.screen.y;
    let cursor_uv = globals.cursor_pos / globals.screen;
    let cursor_ndc = (cursor_uv - 0.5) * 2.0;

    // ── God-rays ────────────────────────────────────────────────────
    let rays = god_rays(uv, aspect, t);
    let ray_color = vec3<f32>(1.0, 0.88, 0.50); // champagne gold
    var color = ray_color * rays;

    // ── Dust mote layers with parallax ──────────────────────────────
    // Layer 0: foreground — larger, brighter
    let uv0 = uv - cursor_ndc * 0.010;
    let d0 = dust_layer(uv0, 20.0, 0.25, 0.025, t);

    // Layer 1: mid
    let uv1 = uv - cursor_ndc * 0.005;
    let d1 = dust_layer(uv1, 40.0, 0.30, 0.016, t);

    // Layer 2: background — fine, dense
    let uv2 = uv - cursor_ndc * 0.002;
    let d2 = dust_layer(uv2, 70.0, 0.35, 0.010, t);

    // Motes glow brighter in god-ray regions (capped to stay sane
    // if god_rays intensity is ever tuned higher).
    let ray_boost = min(1.0 + rays * 8.0, 3.0);

    let mote_color = vec3<f32>(1.0, 0.92, 0.55); // bright gold
    color += mote_color * (d0 * 0.7 + d1 * 0.4 + d2 * 0.2) * ray_boost;

    // ── Vignette mask ───────────────────────────────────────────────
    // Rays and dust are strongest in upper-center, fading at edges.
    // This is a softer vignette than the edge-focused ember one.
    let vig_center = vec2<f32>(0.5, 0.3);
    let vig_d = length((uv - vig_center) * vec2<f32>(aspect * 0.6, 0.8));
    let vignette = smoothstep(0.8, 0.15, vig_d);
    color *= vignette;

    // ── Gamma correction ────────────────────────────────────────────
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(inv_g));

    return vec4<f32>(color, 0.0);
}
