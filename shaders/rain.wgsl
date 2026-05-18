// Procedural rainfall vignette for the main-menu exterior.
//
// Fullscreen triangle — no vertex buffers. Mostly vertical streaks with a
// slight random lean per drop; faint lower mist softens the waterfront mood.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
    cursor_pos: vec2<f32>,
};

struct RainParams {
    layer0: vec4<f32>,
    layer1: vec4<f32>,
    layer2: vec4<f32>,
    scroll: vec4<f32>,
    // xyz = per-layer brightness; w = global speed multiplier on all rain motion.
    bright: vec4<f32>,
    col0: vec4<f32>,
    col1: vec4<f32>,
    col2: vec4<f32>,
    mist_rgb_strength: vec4<f32>,
    mist_scale_scroll: vec4<f32>,
    mist_soft_lower: vec4<f32>,
    drop_a: vec4<f32>,
    drop_b: vec4<f32>,
    drop_c: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> rain: RainParams;

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

// Streak aligned to `fall_dir` (mostly down, slight random lean).
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

fn drop_fall_dir(cell: vec2<f32>, aspect: f32, lean: f32) -> vec2<f32> {
    let h = hash21(cell + 59.2);
    let tilt = (h - 0.5) * lean * aspect;
    return normalize(vec2<f32>(tilt, -1.0));
}

fn rain_layer(
    uv: vec2<f32>,
    layer: vec4<f32>,
    scroll: f32,
    time: f32,
    aspect: f32,
) -> f32 {
    let scale = layer.x;
    let density = layer.y;
    let half_len = layer.z;
    let half_w = layer.w;

    // Diagonal field drift + square-ish cells avoid vertical "curtain" banding from a
    // purely Y-scrolled axis-aligned grid of thin streaks.
    let lean = rain.scroll.w;
    let drift = vec2<f32>(scroll * lean * 3.0, -scroll);
    let scrolled = uv + time * drift;
    let grid_base = scrolled * vec2<f32>(scale * aspect, scale);
    let row = floor(grid_base.y);
    let stagger = fract(row * 0.5);
    let grid_uv = vec2<f32>(grid_base.x + stagger * 0.5, grid_base.y);
    let cell = floor(grid_uv);
    let frac_uv = fract(grid_uv);

    let rng = hash22(cell);
    if rng.x > density {
        return 0.0;
    }

    let drop_dir = drop_fall_dir(cell, aspect, lean);
    let phase = hash21(cell + 91.7) * 6.2831853;
    let sway_amp = rain.drop_a.x + hash21(cell + 33.1) * rain.drop_a.y;
    let sway = sin(time * (0.5 + rng.y * 0.7) + phase) * sway_amp;
    // Per-drop fall rate is global (`drop_a` + `bright.w` via `time`); `scroll` is only
    // the parallax field drift above so Speed × affects near/mid/far streaks equally.
    let fall_speed = rain.drop_a.z + hash21(cell + 52.4) * rain.drop_a.w;
    let base_y = rain.drop_b.x + rng.y * rain.drop_b.y;
    let local_y = fract(base_y - time * fall_speed);
    let pos_rng = hash22(cell + vec2<f32>(19.4, 7.2));
    let center = vec2<f32>(
        rain.drop_b.z + pos_rng.x * rain.drop_b.w + sway,
        local_y,
    );
    let half_w_adj = half_w * sqrt(max(aspect, 1.0));

    let fade = smoothstep(0.0, rain.drop_c.x, local_y)
        * smoothstep(1.0, rain.drop_c.y, local_y);
    let streak = streak_intensity(frac_uv, center, half_len, half_w_adj, drop_dir);
    let shimmer = rain.drop_c.z + rain.drop_c.w * sin(time * (4.0 + rng.x * 3.0) + phase);
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
    let p = uv * vec2<f32>(
            rain.mist_scale_scroll.x * aspect,
            rain.mist_scale_scroll.y,
        )
        + vec2<f32>(
            time * rain.mist_scale_scroll.z,
            time * rain.mist_scale_scroll.w,
        );
    let n = mist_fbm(p);
    let soft = smoothstep(rain.mist_soft_lower.x, rain.mist_soft_lower.y, n);
    let lower = smoothstep(rain.mist_soft_lower.z, rain.mist_soft_lower.w, uv.y);
    return soft * lower * rain.mist_rgb_strength.w;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = globals.time * max(rain.bright.w, 0.0);
    let aspect = globals.screen.x / max(globals.screen.y, 1.0);

    let r0 = rain_layer(uv, rain.layer0, rain.scroll.x, t, aspect);
    let r1 = rain_layer(uv, rain.layer1, rain.scroll.y, t, aspect);
    let r2 = rain_layer(uv, rain.layer2, rain.scroll.z, t, aspect);

    var color = rain.col0.rgb * r0 * rain.bright.x
        + rain.col1.rgb * r1 * rain.bright.y
        + rain.col2.rgb * r2 * rain.bright.z;
    color += rain.mist_rgb_strength.rgb * mist_layer(uv, t);

    let inv_g = 1.0 / max(globals.gamma, 0.01);
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(inv_g));

    return vec4<f32>(color, 0.0);
}
