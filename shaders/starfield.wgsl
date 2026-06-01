// Procedural constellation starfield for the main menu background.
//
// Fullscreen triangle — no vertex buffers. Each pixel computes a
// multi-layer parallax star field with mahjong-themed constellations
// that glow when the cursor is nearby.
//
// Globals.cursor_pos carries the screen-space mouse position so the
// star layers shift with subtle parallax, and constellations brighten
// on proximity.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
    cursor_pos: vec2<f32>,
    transition_progress: f32,
    quality_level: f32,
    moon_phase: f32,
    /// User Effects tier during cascade wipes (Rust `Globals._globals_pad[0]`).
    cascade_quality_level: f32,
    /// x = main-menu June pride rainbow (Rust `Globals._globals_pad[1]`).
    _globals_pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

fn starfield_tint(uv: vec2<f32>, time: f32) -> vec3<f32> {
    if (globals._globals_pad.x > 0.5) {
        return rainbow_swirl_smooth_rgb(uv, time);
    }
    return vec3<f32>(1.0, 0.98, 0.95);
}

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
    out.clip_pos = vec4<f32>(p, 0.9999, 1.0); // near far plane
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

// ── Star layer ────────────────────────────────────────────────────────
// Grid-based procedural stars. Each cell may contain one star whose
// position, brightness, and twinkle phase are derived from hashes.
struct StarLayerSample {
    brightness: f32,
    tint: vec3<f32>,
}

fn star_layer(uv: vec2<f32>, scale: f32, density: f32, size: f32, time: f32) -> StarLayerSample {
    let grid_uv = uv * scale;
    let cell = floor(grid_uv);
    let frac_uv = fract(grid_uv);

    let rng = hash22(cell);
    if rng.x > density {
        return StarLayerSample(0.0, vec3<f32>(0.0));
    }

    // Star position within cell (inset from edges to avoid clipping)
    let star_pos = vec2<f32>(0.2 + rng.x * 0.6, 0.2 + rng.y * 0.6);
    let d = length(frac_uv - star_pos);

    // Soft point with smooth falloff
    let brightness = smoothstep(size, size * 0.15, d);

    // Per-star twinkle
    let phase = hash21(cell + 73.1) * 6.2831853;
    let spd = 1.5 + rng.y * 2.0;
    let twinkle = 0.7 + 0.3 * sin(time * spd + phase);

    let tint = starfield_tint(cell * 0.11 + star_pos * 1.7, time);
    return StarLayerSample(brightness * twinkle, tint);
}

// ── SDF line segment ──────────────────────────────────────────────────
fn sdf_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let t = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * t);
}

// ── Constellation data ────────────────────────────────────────────────
// 6 mahjong-themed constellations. Each has up to 8 star positions and
// up to 10 line segments (pairs of star indices). Sentinel = (-1, -1).
//
// Positions are in UV space [0, 1]. Constellations are spread across
// the sky to avoid overlap.

const C_COUNT: u32 = 6u;
const C_MAX_STARS: u32 = 8u;
const C_MAX_LINES: u32 = 10u;

// Flat storage: stars[constellation][star] = vec2
// Lines[constellation][line] = vec2(index_a, index_b)

// 0: Three Dots (San Pin) — triangle of dots
const C0_STARS = array<vec2<f32>, 8>(
    vec2<f32>(0.14, 0.23),
    vec2<f32>(0.17, 0.28),
    vec2<f32>(0.11, 0.28),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0)
);
const C0_LINES = array<vec2<f32>, 10>(
    vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 2.0), vec2<f32>(2.0, 0.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0)
);

// 1: Bamboo Pair — two parallel vertical bars
const C1_STARS = array<vec2<f32>, 8>(
    vec2<f32>(0.73, 0.17),
    vec2<f32>(0.73, 0.24),
    vec2<f32>(0.77, 0.17),
    vec2<f32>(0.77, 0.24),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0)
);
const C1_LINES = array<vec2<f32>, 10>(
    vec2<f32>(0.0, 1.0), vec2<f32>(2.0, 3.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0)
);

// 2: Wan Character (万) — cross-like brushstroke
const C2_STARS = array<vec2<f32>, 8>(
    vec2<f32>(0.46, 0.68),
    vec2<f32>(0.54, 0.68),
    vec2<f32>(0.50, 0.68),
    vec2<f32>(0.50, 0.73),
    vec2<f32>(0.47, 0.71),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0)
);
const C2_LINES = array<vec2<f32>, 10>(
    vec2<f32>(0.0, 1.0),  // horizontal top stroke
    vec2<f32>(2.0, 3.0),  // vertical center stroke
    vec2<f32>(4.0, 1.0),  // diagonal left-to-right
    vec2<f32>(0.0, 3.0),  // diagonal right-to-left
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0)
);

// 3: East Wind — diamond with center star
const C3_STARS = array<vec2<f32>, 8>(
    vec2<f32>(0.85, 0.51),  // top
    vec2<f32>(0.89, 0.55),  // right
    vec2<f32>(0.85, 0.59),  // bottom
    vec2<f32>(0.81, 0.55),  // left
    vec2<f32>(0.85, 0.55),  // center
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0)
);
const C3_LINES = array<vec2<f32>, 10>(
    vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 2.0),
    vec2<f32>(2.0, 3.0), vec2<f32>(3.0, 0.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0)
);

// 4: Red Dragon (中) — rectangular frame
const C4_STARS = array<vec2<f32>, 8>(
    vec2<f32>(0.22, 0.62),
    vec2<f32>(0.28, 0.62),
    vec2<f32>(0.28, 0.69),
    vec2<f32>(0.22, 0.69),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0)
);
const C4_LINES = array<vec2<f32>, 10>(
    vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 2.0),
    vec2<f32>(2.0, 3.0), vec2<f32>(3.0, 0.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0)
);

// 5: Flower Cluster — hexagonal blossom, upper-left corner
const C5_STARS = array<vec2<f32>, 8>(
    vec2<f32>(0.12, 0.10),  // center
    vec2<f32>(0.12, 0.06),  // top
    vec2<f32>(0.155, 0.08), // upper-right
    vec2<f32>(0.155, 0.12), // lower-right
    vec2<f32>(0.12, 0.14),  // bottom
    vec2<f32>(0.085, 0.12), // lower-left
    vec2<f32>(0.085, 0.08), // upper-left
    vec2<f32>(-1.0, -1.0)
);
const C5_LINES = array<vec2<f32>, 10>(
    vec2<f32>(0.0, 1.0), vec2<f32>(0.0, 2.0),
    vec2<f32>(0.0, 3.0), vec2<f32>(0.0, 4.0),
    vec2<f32>(0.0, 5.0), vec2<f32>(0.0, 6.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, -1.0)
);

// Accessor helpers — WGSL doesn't allow runtime indexing of const
// arrays-of-arrays, so we select per-constellation with a switch.
fn get_star(c: u32, s: u32) -> vec2<f32> {
    switch c {
        case 0u: { return C0_STARS[s]; }
        case 1u: { return C1_STARS[s]; }
        case 2u: { return C2_STARS[s]; }
        case 3u: { return C3_STARS[s]; }
        case 4u: { return C4_STARS[s]; }
        case 5u: { return C5_STARS[s]; }
        default: { return vec2<f32>(-1.0); }
    }
}

fn get_line(c: u32, l: u32) -> vec2<f32> {
    switch c {
        case 0u: { return C0_LINES[l]; }
        case 1u: { return C1_LINES[l]; }
        case 2u: { return C2_LINES[l]; }
        case 3u: { return C3_LINES[l]; }
        case 4u: { return C4_LINES[l]; }
        case 5u: { return C5_LINES[l]; }
        default: { return vec2<f32>(-1.0); }
    }
}

// Constellation centroid (precomputed for proximity test).
fn constellation_center(c: u32) -> vec2<f32> {
    switch c {
        case 0u: { return vec2<f32>(0.14, 0.263); }
        case 1u: { return vec2<f32>(0.75, 0.205); }
        case 2u: { return vec2<f32>(0.50, 0.70); }
        case 3u: { return vec2<f32>(0.85, 0.55); }
        case 4u: { return vec2<f32>(0.25, 0.655); }
        case 5u: { return vec2<f32>(0.12, 0.10); }
        default: { return vec2<f32>(0.5); }
    }
}

// ── Constellation rendering ───────────────────────────────────────────
fn constellations(uv: vec2<f32>, cursor_uv: vec2<f32>, aspect: f32, time: f32) -> vec3<f32> {
    var glow = vec3<f32>(0.0);

    for (var c = 0u; c < C_COUNT; c++) {
        let center = constellation_center(c);

        // Aspect-corrected distance for proximity
        let diff = (cursor_uv - center) * vec2<f32>(aspect, 1.0);
        let cursor_dist = length(diff);

        // Base visibility + proximity boost
        let proximity = smoothstep(0.30, 0.05, cursor_dist);
        let alpha = 0.06 + proximity * 0.40;

        // Gentle breathing animation per constellation
        let breathe = 0.9 + 0.1 * sin(time * 0.8 + f32(c) * 1.7);

        // Draw line segments
        var line_brightness = 0.0;
        for (var l = 0u; l < C_MAX_LINES; l++) {
            let seg = get_line(c, l);
            if seg.x < 0.0 { break; }
            let a = get_star(c, u32(seg.x));
            let b = get_star(c, u32(seg.y));
            // Aspect-correct the SDF
            let p_a = uv * vec2<f32>(aspect, 1.0);
            let s_a = a * vec2<f32>(aspect, 1.0);
            let s_b = b * vec2<f32>(aspect, 1.0);
            let d = sdf_segment(p_a, s_a, s_b);
            let line_w = 0.0025;
            line_brightness += smoothstep(line_w, line_w * 0.25, d);
        }

        // Draw constellation stars (brighter than field stars)
        var star_brightness = 0.0;
        for (var s = 0u; s < C_MAX_STARS; s++) {
            let sp = get_star(c, s);
            if sp.x < 0.0 { break; }
            let d_a = length((uv - sp) * vec2<f32>(aspect, 1.0));
            star_brightness += smoothstep(0.008, 0.001, d_a);
        }

        let tint = starfield_tint(center * 2.2 + vec2<f32>(f32(c) * 0.37, 0.0), time);
        glow += tint * (line_brightness * 0.25 + star_brightness * 0.8) * alpha * breathe;
    }

    return glow;
}

// ── Shooting star ─────────────────────────────────────────────────────
fn shooting_star(uv: vec2<f32>, aspect: f32, time: f32) -> f32 {
    let period = 12.0;
    let cycle = floor(time / period);
    let t_local = fract(time / period);

    // Visible during the first ~15% of each cycle (~1.8 seconds)
    if t_local > 0.15 { return 0.0; }
    let t_anim = t_local / 0.15;

    // Random start position + direction seeded by cycle
    let seed = vec2<f32>(cycle * 31.7, cycle * 47.3);
    let start = vec2<f32>(0.2 + hash21(seed) * 0.6, hash21(seed + 1.0) * 0.4);
    let angle = -0.3 - hash21(seed + 2.0) * 0.5;
    let dir = vec2<f32>(cos(angle), sin(angle));
    let trail_len = 0.06 + hash21(seed + 3.0) * 0.05;

    // Head position — travels across the screen
    let head = start + dir * t_anim * 0.5;
    let tail = head - dir * trail_len * (1.0 - t_anim * 0.3);

    // Aspect-corrected SDF
    let p_a = uv * vec2<f32>(aspect, 1.0);
    let h_a = head * vec2<f32>(aspect, 1.0);
    let t_a = tail * vec2<f32>(aspect, 1.0);

    let d = sdf_segment(p_a, t_a, h_a);
    let head_d = length(p_a - h_a);

    let head_glow = smoothstep(0.005, 0.0005, head_d);
    let trail_glow = smoothstep(0.003, 0.0003, d) * smoothstep(0.0, trail_len * aspect, length(p_a - t_a));

    // Fade in and out
    let fade = smoothstep(0.0, 0.15, t_anim) * smoothstep(1.0, 0.7, t_anim);

    return (head_glow * 2.0 + trail_glow) * fade;
}

// ── Fragment main ─────────────────────────────────────────────────────
//
// Additive blend: the output RGB is *added* to whatever the table scene
// already rendered. Stars glow through the dark wood regions and vanish
// naturally where candlelight is already bright. No opaque sky fill —
// just emitted light from stars, constellations, and shooting stars.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = globals.time;
    let aspect = globals.screen.x / globals.screen.y;
    let cursor_uv = globals.cursor_pos / globals.screen;

    // Additive stars only — keep interstitial space black.
    var color = vec3<f32>(0.0);

    // ── Star layers with parallax ───────────────────────────────────
    let cursor_ndc = (cursor_uv - 0.5) * 2.0;

    // Layer 0: foreground — big, sparse, strong parallax
    let uv0 = uv - cursor_ndc * 0.015;
    let s0 = star_layer(uv0, 35.0, 0.35, 0.028, t);

    // Layer 1: mid — medium density and parallax
    let uv1 = uv - cursor_ndc * 0.008;
    let s1 = star_layer(uv1, 70.0, 0.45, 0.018, t);

    // Layer 2: background — tiny, dense, minimal parallax
    let uv2 = uv - cursor_ndc * 0.002;
    let s2 = star_layer(uv2, 140.0, 0.55, 0.010, t);

    color += s0.tint * s0.brightness * 0.7;
    color += s1.tint * s1.brightness * 0.5;
    color += s2.tint * s2.brightness * 0.3;

    // ── Constellations (mid-layer parallax) ─────────────────────────
    let c_uv = uv - cursor_ndc * 0.008;
    color += constellations(c_uv, cursor_uv, aspect, t);

    // ── Shooting star ───────────────────────────────────────────────
    let shoot = shooting_star(uv, aspect, t);
    color += starfield_tint(uv * 4.0, t) * shoot;

    // ── Vignette mask: fade to zero in the centre so the effect ────
    // frames the menu without competing with the UI.
    let vig_center = vec2<f32>(0.5, 0.45);
    let vig_d = length((uv - vig_center) * vec2<f32>(aspect * 0.8, 1.0));
    let vignette = smoothstep(0.25, 0.65, vig_d);
    color *= vignette;

    // ── Gamma correction ────────────────────────────────────────────
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(inv_g));

    return vec4<f32>(color, 0.0);
}
