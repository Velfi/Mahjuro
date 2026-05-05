// Procedural rising-ember vignette effect.
//
// Fullscreen triangle — no vertex buffers. Tiny orange-gold ember
// particles drift upward with horizontal sway. A vignette mask keeps
// embers concentrated at screen edges so they frame without obscuring
// the central UI.

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

// ── Single ember layer ────────────────────────────────────────────────
// Grid-based: each cell may spawn one ember that drifts upward over
// time. The cell's hash determines position, lifetime phase, sway
// frequency, and brightness.
fn ember_layer(uv: vec2<f32>, scale: f32, density: f32, size: f32,
               speed: f32, time: f32) -> f32 {
    // Scroll UV upward so embers rise
    let scrolled = vec2<f32>(uv.x, uv.y + time * speed);
    let grid_uv = scrolled * scale;
    let cell = floor(grid_uv);
    let frac_uv = fract(grid_uv);

    let rng = hash22(cell);
    if rng.x > density { return 0.0; }

    // Ember position within cell (inset from edges)
    let base_x = 0.15 + rng.x * 0.7;
    // Horizontal sway using time + per-ember phase
    let phase = hash21(cell + 41.3) * 6.2831853;
    let sway_freq = 0.8 + rng.y * 1.2;
    let sway_amp = 0.08 + hash21(cell + 17.7) * 0.06;
    let sway = sin(time * sway_freq + phase) * sway_amp;
    // Per-ember upward drift within the cell so each ember visibly rises
    // rather than appearing as a static scrolling texture.
    let rise_speed = 0.03 + hash21(cell + 63.2) * 0.04;
    let base_y = 0.2 + rng.y * 0.4;
    let local_y = fract(base_y + time * rise_speed);
    let ember_pos = vec2<f32>(base_x + sway, local_y);

    let d = length(frac_uv - ember_pos);

    // Bright core with soft exponential-ish glow halo
    let core = smoothstep(size, size * 0.1, d);
    let halo = smoothstep(size * 4.0, size * 0.5, d) * 0.3;

    // Lifecycle fade: born at bottom of cell (local_y ≈ 0), dies at top
    // (local_y ≈ 1). Smoothstep ramps give a gentle birth/death.
    let fade = smoothstep(0.0, 0.25, local_y) * smoothstep(1.0, 0.75, local_y);

    // Per-ember flicker
    let flicker = 0.75 + 0.25 * sin(time * (3.0 + rng.x * 4.0) + phase);

    return (core + halo) * fade * flicker;
}

// ── Fragment main ─────────────────────────────────────────────────────
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = globals.time;
    let aspect = globals.screen.x / globals.screen.y;
    // ── Ember layers ─────────────────────────────────────────────────
    // Layer 0: foreground — larger, sparse, fast
    let e0 = ember_layer(uv, 18.0, 0.30, 0.030, 0.04, t);

    // Layer 1: mid — medium density and speed
    let e1 = ember_layer(uv, 35.0, 0.35, 0.020, 0.025, t);

    // Layer 2: background — tiny, dense, slow
    let e2 = ember_layer(uv, 60.0, 0.40, 0.012, 0.015, t);

    // ── Colour grading ──────────────────────────────────────────────
    // Warm orange-amber with brightness variation per layer
    let col0 = vec3<f32>(1.0, 0.55, 0.12);  // bright amber
    let col1 = vec3<f32>(1.0, 0.40, 0.08);  // deeper orange
    let col2 = vec3<f32>(0.9, 0.30, 0.06);  // dim ember red

    var color = col0 * e0 * 0.8
              + col1 * e1 * 0.5
              + col2 * e2 * 0.3;

    // ── Vignette mask: visible at edges, transparent in centre ──────
    // Inverse of celestial — embers frame the screen edges.
    let vig_center = vec2<f32>(0.5, 0.45);
    let vig_d = length((uv - vig_center) * vec2<f32>(aspect * 0.8, 1.0));
    let vignette = smoothstep(0.20, 0.60, vig_d);
    color *= vignette;

    // ── Gamma correction ────────────────────────────────────────────
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(inv_g));

    return vec4<f32>(color, 0.0);
}
