// Procedural rainfall vignette for the main-menu exterior.
//
// Fullscreen triangle — no vertex buffers. Cool streaks fall down-screen with
// a steady rightward lean (wind); a faint lower mist softens the waterfront mood.

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

// Streak aligned to `fall_dir` (down-screen + wind lean).
fn streak_intensity(
    frac_uv: vec2<f32>,
    center: vec2<f32>,
    half_len: f32,
    half_w: f32,
    fall_dir: vec2<f32>,
) -> f32 {
    let d = normalize(fall_dir);
    let n = vec2<f32>(-d.y, d.x);
    let delta = frac_uv - center;
    let along = abs(dot(delta, d));
    let across = abs(dot(delta, n));
    let along_mask = smoothstep(half_len, half_len * 0.15, along);
    let across_mask = smoothstep(half_w, half_w * 0.2, across);
    return along_mask * across_mask;
}

// Apparent fall speed in UV/s along `fall_dir` (y=0 bottom). Moderate shower:
// near ~2.5s across frame height, mid ~3.5s, far ~5.5s (parallax).
const SCROLL_NEAR: f32 = 0.28;
const SCROLL_MID: f32 = 0.20;
const SCROLL_FAR: f32 = 0.12;

fn rain_layer(
    uv: vec2<f32>,
    scale: f32,
    density: f32,
    half_len: f32,
    half_w: f32,
    scroll: f32,
    time: f32,
    fall_dir: vec2<f32>,
) -> f32 {
    // `fall_dir` is velocity (down = −Y in UV). Advect field: uv + v*t.
    let d = normalize(fall_dir);
    let scrolled = uv + time * scroll * d;
    let grid_uv = scrolled * scale;
    let cell = floor(grid_uv);
    let frac_uv = fract(grid_uv);

    let rng = hash22(cell);
    if rng.x > density {
        return 0.0;
    }

    let phase = hash21(cell + 91.7) * 6.2831853;
    let sway = sin(time * (0.5 + rng.y * 0.7) + phase) * (0.02 + hash21(cell + 33.1) * 0.02);
    // In-cell drift tracks layer scroll (no extra speed on top of field motion).
    let fall_speed = scroll * (0.32 + hash21(cell + 52.4) * 0.28);
    let base_y = 0.15 + rng.y * 0.55;
    let local_y = fract(base_y - time * fall_speed);
    let center = vec2<f32>(0.12 + rng.x * 0.76 + sway, local_y);

    let fade = smoothstep(0.0, 0.2, local_y) * smoothstep(1.0, 0.78, local_y);
    let streak = streak_intensity(frac_uv, center, half_len, half_w, fall_dir);
    let shimmer = 0.82 + 0.18 * sin(time * (4.0 + rng.x * 3.0) + phase);
    return streak * fade * shimmer;
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);

    let a = hash21(i + vec2<f32>(0.0, 0.0));
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));

    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn mist_fbm(p: vec2<f32>) -> f32 {
    var val = 0.0;
    val += value_noise(p) * 0.55;
    val += value_noise(p * 2.03 + 4.1) * 0.30;
    val += value_noise(p * 4.07 - 1.8) * 0.15;
    return val;
}

fn mist_layer(uv: vec2<f32>, time: f32) -> f32 {
    let aspect = globals.screen.x / max(globals.screen.y, 1.0);
    // Low frequency + smooth FBM — avoids visible grid cells from raw hash tiles.
    let p = uv * vec2<f32>(1.15 * aspect, 0.95) + vec2<f32>(time * 0.018, time * 0.009);
    let n = mist_fbm(p);
    let soft = smoothstep(0.28, 0.72, n);
    let lower = smoothstep(0.62, 0.08, uv.y);
    return soft * lower * 0.10;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = globals.time;
    let aspect = globals.screen.x / max(globals.screen.y, 1.0);
    // UV: y=0 bottom, y=1 top. Velocity down-screen + wind lean right.
    let fall_dir = vec2<f32>(0.20 * aspect, -1.0);

    let r0 = rain_layer(uv, 22.0, 0.42, 0.24, 0.008, SCROLL_NEAR, t, fall_dir);
    let r1 = rain_layer(uv, 38.0, 0.48, 0.17, 0.006, SCROLL_MID, t, fall_dir);
    let r2 = rain_layer(uv, 58.0, 0.52, 0.11, 0.004, SCROLL_FAR, t, fall_dir);

    let col0 = vec3<f32>(0.82, 0.88, 0.96);
    let col1 = vec3<f32>(0.68, 0.76, 0.88);
    let col2 = vec3<f32>(0.55, 0.64, 0.76);

    var color = col0 * r0 * 0.55 + col1 * r1 * 0.38 + col2 * r2 * 0.28;
    color += vec3<f32>(0.62, 0.70, 0.82) * mist_layer(uv, t);

    let inv_g = 1.0 / max(globals.gamma, 0.01);
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(inv_g));

    return vec4<f32>(color, 0.0);
}
