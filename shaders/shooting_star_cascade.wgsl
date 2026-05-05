// Shooting-star cascade scene transition.
//
// Fullscreen triangle — no vertex buffers.  Driven entirely by
// `globals.transition_progress` (0 = inactive, 0→1 over the full
// transition).  Additive blend: the main loop fades the scene to black
// via apply_alpha while this shader paints bright particles on top.
//
// Timeline:
//   0.00–0.15  Lead shooting star enters upper-left
//   0.10–0.45  Cascade eruption: child particles burst from the trail
//   0.40–0.55  Peak density (scene switch happens at ~0.50)
//   0.55–0.75  Particles decelerate, drift down, fade
//   0.75–1.00  Golden dust linger, last sparkles die

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
    cursor_pos: vec2<f32>,
    transition_progress: f32,
    quality_level: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// ── Fullscreen triangle ──────────────────────────────────────────────
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

// ── Hash helpers ─────────────────────────────────────────────────────
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

fn hash31(p: vec2<f32>) -> vec3<f32> {
    let h = vec3<f32>(
        dot(p, vec2<f32>(127.1, 311.7)),
        dot(p, vec2<f32>(269.5, 183.3)),
        dot(p, vec2<f32>(419.2, 371.9)),
    );
    return fract(sin(h) * 43758.5453);
}

// ── SDF: distance to line segment ────────────────────────────────────
fn sdf_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// ── Lead shooting star ───────────────────────────────────────────────
// Streaks from upper-left to lower-right during progress 0.00–0.35.
fn lead_star(uv: vec2<f32>, aspect: f32, progress: f32) -> vec3<f32> {
    let star_start = vec2<f32>(0.05, 0.08);
    let star_end   = vec2<f32>(0.92, 0.88);

    // Star traversal (ease-in-out hermite)
    let star_t = clamp(progress / 0.35, 0.0, 1.0);
    let eased  = star_t * star_t * (3.0 - 2.0 * star_t);
    let head   = mix(star_start, star_end, vec2<f32>(eased));

    // Trail lags behind the head
    let trail_t = clamp((progress - 0.06) / 0.35, 0.0, 1.0);
    let eased_trail = trail_t * trail_t * (3.0 - 2.0 * trail_t);
    let tail = mix(star_start, star_end, vec2<f32>(eased_trail));

    // Aspect-corrected coords
    let p = uv * vec2<f32>(aspect, 1.0);
    let h = head * vec2<f32>(aspect, 1.0);
    let t = tail * vec2<f32>(aspect, 1.0);

    let d_head  = length(p - h);
    let d_trail = sdf_segment(p, t, h);

    // Bright core + softer trail
    let core  = exp(-d_head * 350.0) * 4.0;
    let trail = exp(-d_trail * 120.0) * 2.5;
    let glow  = exp(-d_trail * 40.0) * 0.6;

    // Fade out once the star has crossed
    let fade = smoothstep(0.38, 0.25, progress);

    let core_color  = vec3<f32>(1.0, 0.97, 0.88);
    let trail_color = vec3<f32>(1.0, 0.85, 0.40);
    let glow_color  = vec3<f32>(0.8, 0.65, 0.25);

    return (core_color * core + trail_color * trail + glow_color * glow) * fade;
}

// ── Trail path for spawn proximity ───────────────────────────────────
// Returns the distance from `uv` to the lead star's trail, used by
// cascade layers to spawn particles near the streak.
fn trail_distance(uv: vec2<f32>, aspect: f32, progress: f32) -> f32 {
    let star_start = vec2<f32>(0.05, 0.08);
    let star_end   = vec2<f32>(0.92, 0.88);
    let star_t = clamp(progress / 0.35, 0.0, 1.0);
    let eased  = star_t * star_t * (3.0 - 2.0 * star_t);
    let head   = mix(star_start, star_end, vec2<f32>(eased));

    let p = uv * vec2<f32>(aspect, 1.0);
    let a = star_start * vec2<f32>(aspect, 1.0);
    let h = head * vec2<f32>(aspect, 1.0);

    return sdf_segment(p, a, h);
}

// ── Single cascade particle layer ────────────────────────────────────
// Each layer evaluates a grid of virtual particles.  Per-particle
// properties (position, velocity, color, lifetime) are deterministic
// from the cell hash, so no persistent state is needed.
fn cascade_layer(
    uv: vec2<f32>,
    aspect: f32,
    layer_f: f32,
    progress: f32,
    time: f32,
) -> vec3<f32> {
    // Per-layer tuning
    let scale       = 18.0 + layer_f * 10.0;   // 18 … 168
    let density     = 0.75 + layer_f * 0.015;   // fill ratio (very dense)
    let psize       = 0.042 - layer_f * 0.002;  // particle radius
    let speed_mult  = 1.3 - layer_f * 0.06;
    let spawn_delay = layer_f * 0.015;

    // Particle age relative to spawn moment
    let spawn_start = 0.07 + spawn_delay;
    let age = max(progress - spawn_start, 0.0);
    if age <= 0.0 { return vec3<f32>(0.0); }

    // Physics: initial burst outward, then gravity pulls down
    let gravity_t = age * 2.2;
    let drift_y   = -gravity_t * gravity_t * 0.18;
    let drift_x   = sin(time * 0.4 + layer_f * 1.7) * 0.015 * speed_mult;
    let drift      = vec2<f32>(drift_x, drift_y * speed_mult);

    let scrolled = uv + drift;
    let grid_uv  = scrolled * scale;
    let cell      = floor(grid_uv);
    let frac      = fract(grid_uv);

    // Per-cell random (seeded with layer to de-correlate layers)
    let seed = cell + vec2<f32>(layer_f * 137.0, layer_f * 59.0);
    let rng  = hash22(seed);
    if rng.x > density { return vec3<f32>(0.0); }

    // Particle centre within cell
    let center = vec2<f32>(0.15 + rng.x * 0.7, 0.15 + rng.y * 0.7);

    // Per-particle outward velocity from trail
    let vel_angle = hash21(seed + vec2<f32>(7.7, 0.0)) * 6.2832;
    let vel_mag   = 0.08 + hash21(seed + vec2<f32>(13.3, 0.0)) * 0.35;
    let vel       = vec2<f32>(cos(vel_angle), sin(vel_angle)) * vel_mag;
    let moved     = center + vel * age * speed_mult * 1.5;

    // Distance from fragment to particle centre
    let diff = frac - moved;
    let d    = length(diff);

    // Core + glow
    let core = smoothstep(psize, psize * 0.08, d);
    let glow = smoothstep(psize * 3.5, psize * 0.2, d) * 0.25;

    // Spawn proximity to lead star trail — particles near the streak
    // are brighter.  Further particles still appear but dimmer.
    let cell_world_uv = (cell + center) / scale - drift;
    let td = trail_distance(cell_world_uv, aspect, progress);
    let spawn_mask = smoothstep(0.55, 0.02, td);

    // Lifetime
    let lifetime  = 0.25 + hash21(seed + vec2<f32>(23.1, 0.0)) * 0.55;
    let life_frac = clamp(age / lifetime, 0.0, 1.0);
    let life_fade = smoothstep(1.0, 0.65, life_frac)
                  * smoothstep(0.0, 0.04, life_frac);

    // Twinkle
    let twinkle = 0.65 + 0.35 * sin(
        time * (2.5 + rng.y * 4.0) + rng.x * 6.2832
    );

    // Color temperature: hot white → gold → amber
    let temp = mix(
        vec3<f32>(1.0, 0.97, 0.90),
        vec3<f32>(0.95, 0.55, 0.12),
        life_frac * 0.85,
    );

    let brightness = (core + glow) * life_fade * twinkle * spawn_mask;
    return temp * brightness;
}

// ── Micro-sparkle layer ──────────────────────────────────────────────
// Very high grid density, tiny particles, rapid twinkle. These fill the
// gaps between the main cascade particles with a fine glitter.
fn sparkle_layer(
    uv: vec2<f32>,
    aspect: f32,
    layer_f: f32,
    progress: f32,
    time: f32,
) -> vec3<f32> {
    let scale       = 80.0 + layer_f * 25.0;   // 80 … 255
    let density     = 0.60 + layer_f * 0.03;
    let psize       = 0.012 - layer_f * 0.001;
    let speed_mult  = 1.0 - layer_f * 0.08;
    let spawn_delay = 0.10 + layer_f * 0.02;

    let age = max(progress - spawn_delay, 0.0);
    if age <= 0.0 { return vec3<f32>(0.0); }

    let gravity_t = age * 2.5;
    let drift = vec2<f32>(
        cos(time * 0.3 + layer_f * 2.3) * 0.01,
        -gravity_t * gravity_t * 0.12 * speed_mult,
    );

    let grid_uv = (uv + drift) * scale;
    let cell    = floor(grid_uv);
    let frac    = fract(grid_uv);

    let seed = cell + vec2<f32>(layer_f * 293.0, layer_f * 197.0);
    let rng  = hash22(seed);
    if rng.x > density { return vec3<f32>(0.0); }

    let center = vec2<f32>(0.1 + rng.x * 0.8, 0.1 + rng.y * 0.8);
    let vel_angle = hash21(seed + vec2<f32>(5.5, 0.0)) * 6.2832;
    let vel_mag   = 0.05 + hash21(seed + vec2<f32>(11.1, 0.0)) * 0.25;
    let moved = center + vec2<f32>(cos(vel_angle), sin(vel_angle)) * vel_mag * age * speed_mult;

    let d = length(frac - moved);
    let core = smoothstep(psize, psize * 0.05, d);

    // Trail proximity
    let cell_world_uv = (cell + center) / scale - drift;
    let td = trail_distance(cell_world_uv, aspect, progress);
    let spawn_mask = smoothstep(0.50, 0.02, td);

    let lifetime  = 0.15 + hash21(seed + vec2<f32>(19.9, 0.0)) * 0.40;
    let life_frac = clamp(age / lifetime, 0.0, 1.0);
    let life_fade = smoothstep(1.0, 0.5, life_frac) * smoothstep(0.0, 0.03, life_frac);

    // Fast twinkle for sparkle effect
    let twinkle = 0.4 + 0.6 * sin(time * (5.0 + rng.y * 8.0) + rng.x * 6.2832);

    let temp = mix(
        vec3<f32>(1.0, 0.95, 0.85),
        vec3<f32>(1.0, 0.70, 0.25),
        life_frac * 0.7,
    );

    return temp * core * life_fade * twinkle * spawn_mask * 0.7;
}

// ── Constellation flash ──────────────────────────────────────────────
// During settle phase (0.55–0.75), some particles briefly align into
// small geometric formations before dispersing.
fn constellation_flash(uv: vec2<f32>, aspect: f32, progress: f32) -> vec3<f32> {
    let envelope = smoothstep(0.53, 0.60, progress)
                 * smoothstep(0.78, 0.68, progress);
    if envelope < 0.001 { return vec3<f32>(0.0); }

    var result = vec3<f32>(0.0);
    let color  = vec3<f32>(1.0, 0.90, 0.50);

    // Four small geometric shapes at hash-seeded positions
    // Shape 0: triangle
    let c0 = hash22(vec2<f32>(71.3, 13.7));
    for (var j = 0u; j < 3u; j++) {
        let angle = f32(j) / 3.0 * 6.2832 - 0.5236;
        let dot_pos = c0 + vec2<f32>(cos(angle), sin(angle)) * 0.022;
        let d = length((uv - dot_pos) * vec2<f32>(aspect, 1.0));
        result += color * smoothstep(0.005, 0.001, d);
    }
    // Shape 1: diamond
    let c1 = hash22(vec2<f32>(142.6, 27.4));
    for (var j = 0u; j < 4u; j++) {
        let angle = f32(j) / 4.0 * 6.2832;
        let dot_pos = c1 + vec2<f32>(cos(angle), sin(angle)) * 0.018;
        let d = length((uv - dot_pos) * vec2<f32>(aspect, 1.0));
        result += color * smoothstep(0.005, 0.001, d);
    }
    // Shape 2: pentagon
    let c2 = hash22(vec2<f32>(213.9, 41.1));
    for (var j = 0u; j < 5u; j++) {
        let angle = f32(j) / 5.0 * 6.2832 + 0.3;
        let dot_pos = c2 + vec2<f32>(cos(angle), sin(angle)) * 0.020;
        let d = length((uv - dot_pos) * vec2<f32>(aspect, 1.0));
        result += color * smoothstep(0.004, 0.001, d);
    }
    // Shape 3: small triangle
    let c3 = hash22(vec2<f32>(284.2, 55.8));
    for (var j = 0u; j < 3u; j++) {
        let angle = f32(j) / 3.0 * 6.2832 + 1.0472;
        let dot_pos = c3 + vec2<f32>(cos(angle), sin(angle)) * 0.015;
        let d = length((uv - dot_pos) * vec2<f32>(aspect, 1.0));
        result += color * smoothstep(0.004, 0.001, d);
    }

    // Connecting lines within each shape (thin glowing segments)
    // Shape 0 lines
    for (var j = 0u; j < 3u; j++) {
        let a0 = f32(j) / 3.0 * 6.2832 - 0.5236;
        let a1 = f32((j + 1u) % 3u) / 3.0 * 6.2832 - 0.5236;
        let pa = (c0 + vec2<f32>(cos(a0), sin(a0)) * 0.022) * vec2<f32>(aspect, 1.0);
        let pb = (c0 + vec2<f32>(cos(a1), sin(a1)) * 0.022) * vec2<f32>(aspect, 1.0);
        let sd = sdf_segment(uv * vec2<f32>(aspect, 1.0), pa, pb);
        result += color * 0.4 * smoothstep(0.003, 0.0005, sd);
    }
    // Shape 1 lines
    for (var j = 0u; j < 4u; j++) {
        let a0 = f32(j) / 4.0 * 6.2832;
        let a1 = f32((j + 1u) % 4u) / 4.0 * 6.2832;
        let pa = (c1 + vec2<f32>(cos(a0), sin(a0)) * 0.018) * vec2<f32>(aspect, 1.0);
        let pb = (c1 + vec2<f32>(cos(a1), sin(a1)) * 0.018) * vec2<f32>(aspect, 1.0);
        let sd = sdf_segment(uv * vec2<f32>(aspect, 1.0), pa, pb);
        result += color * 0.3 * smoothstep(0.003, 0.0005, sd);
    }

    return result * envelope * 0.55;
}

// ── Golden dust linger ───────────────────────────────────────────────
// Two layers of slowly drifting golden motes for the tail end of the
// transition, giving the new scene a brief ethereal quality.
fn dust_layer(uv: vec2<f32>, scale: f32, density: f32, size: f32, time: f32) -> f32 {
    let drift = vec2<f32>(
        sin(time * 0.15) * 0.02,
        -time * 0.008,
    );
    let scrolled = uv + drift;
    let grid_uv  = scrolled * scale;
    let cell      = floor(grid_uv);
    let frac      = fract(grid_uv);

    let rng = hash22(cell);
    if rng.x > density { return 0.0; }

    let center = vec2<f32>(0.1 + rng.x * 0.8, 0.1 + rng.y * 0.8);
    // Gentle wander
    let wander = vec2<f32>(
        sin(time * 0.3 + rng.x * 6.28) * 0.08,
        cos(time * 0.25 + rng.y * 6.28) * 0.06,
    );
    let moved = center + wander;
    let d = length(frac - moved);

    let twinkle = 0.6 + 0.4 * sin(time * (1.5 + rng.y * 2.0) + rng.x * 6.28);
    return smoothstep(size, size * 0.1, d) * twinkle;
}

fn golden_linger(uv: vec2<f32>, progress: f32, time: f32) -> vec3<f32> {
    let linger_t = smoothstep(0.68, 0.78, progress)
                 * smoothstep(1.0, 0.88, progress);
    if linger_t < 0.001 { return vec3<f32>(0.0); }

    let d0 = dust_layer(uv, 28.0, 0.22, 0.025, time);
    let d1 = dust_layer(uv + vec2<f32>(0.37, 0.73), 52.0, 0.28, 0.015, time);

    let color = vec3<f32>(1.0, 0.88, 0.50);
    return color * (d0 * 0.5 + d1 * 0.35) * linger_t;
}

// ── Procedural shooting star layer ───────────────────────────────────
// Grid-based: each cell spawns one shooting star.  All stars travel in
// roughly the same direction — a great arc from upper-left to lower-
// right, matching the lead star.  Per-star jitter keeps them from
// looking uniform.  Spawn timing is staggered by position along the
// arc so the 1024 stars sweep across the screen as a wave.
fn star_streak_layer(
    uv: vec2<f32>,
    aspect: f32,
    progress: f32,
    grid_scale: f32,
    layer_seed: f32,
    brightness: f32,
) -> vec3<f32> {
    let asp = vec2<f32>(aspect, 1.0);
    let p   = uv * asp;
    var result = vec3<f32>(0.0);

    let grid_uv = uv * grid_scale;
    let base_cell = floor(grid_uv);

    // Base arc direction: upper-left → lower-right (same as lead star)
    // Angle ≈ 0.74 rad (≈ 42°), pointing down-right in UV space.
    let base_angle = 0.74;

    for (var dy = -2i; dy <= 2i; dy++) {
        for (var dx = -2i; dx <= 2i; dx++) {
            let cell = base_cell + vec2<f32>(f32(dx), f32(dy));
            let seed = cell + vec2<f32>(layer_seed * 347.0, layer_seed * 113.0);

            let r0 = hash31(seed);
            let r1 = hash31(seed + vec2<f32>(77.7, 33.3));

            // Density gate — ~80% of cells spawn a star
            if r0.z > 0.80 { continue; }

            // Origin: within cell, mapped to UV space
            let origin = (cell + vec2<f32>(0.1 + r0.x * 0.8, 0.1 + r0.y * 0.8)) / grid_scale;

            // Direction: base arc ± small jitter (±0.35 rad ≈ ±20°)
            let jitter = (r1.x - 0.5) * 0.70;
            let angle = base_angle + jitter;
            let streak_len = (0.10 + r1.y * 0.20) / grid_scale * 32.0;
            let dir = vec2<f32>(cos(angle), sin(angle));
            let end_pt = origin + dir * streak_len;

            // Timing: stagger by position along the arc's perpendicular.
            // Stars near upper-left fire first; stars near lower-right
            // fire later, creating a sweeping wave.
            let arc_pos = dot(origin, vec2<f32>(0.6, 0.5));  // 0..~1
            let t0  = 0.02 + arc_pos * 0.30 + r0.z * 0.08;
            let dur = 0.12 + r1.z * 0.20;

            let st = clamp((progress - t0) / dur, 0.0, 1.0);
            if st <= 0.0 { continue; }
            let se = st * st * (3.0 - 2.0 * st);
            let head = mix(origin, end_pt, vec2<f32>(se));

            let trail_lag = 0.015 + r1.y * 0.02;
            let tail_t = clamp((progress - t0 - trail_lag) / dur, 0.0, 1.0);
            let tail_e = tail_t * tail_t * (3.0 - 2.0 * tail_t);
            let tail = mix(origin, end_pt, vec2<f32>(tail_e));

            let h = head * asp;
            let t = tail * asp;
            let d_seg  = sdf_segment(p, t, h);
            let d_head = length(p - h);

            let fade = smoothstep(t0 + dur + 0.12, t0 + dur * 0.5, progress);

            let core  = exp(-d_head * 500.0) * 1.8;
            let trail = exp(-d_seg * 220.0) * 0.9;

            let temp = mix(
                vec3<f32>(1.0, 0.95, 0.75),
                vec3<f32>(1.0, 0.80, 0.35),
                r0.x,
            );

            result += temp * (core + trail) * fade * brightness;
        }
    }

    return result;
}

// ── Fragment main ────────────────────────────────────────────────────
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv       = in.uv;
    let t        = globals.time;
    let progress = globals.transition_progress;
    let aspect   = globals.screen.x / globals.screen.y;

    // Early out when transition is inactive
    if progress <= 0.0 || progress >= 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    var color = vec3<f32>(0.0);

    // ── 1. Lead shooting star (always) ─────────────────────────────────
    color += lead_star(uv, aspect, progress);

    // ── 2. Procedural shooting stars ────────────────────────────────
    // Low: 2 layers, Medium: +1, High: +2 = 5 total
    color += star_streak_layer(uv, aspect, progress, 16.0, 1.0, 1.0);
    color += star_streak_layer(uv, aspect, progress, 16.0, 2.0, 0.85);
    if globals.quality_level >= 1.0 {
        color += star_streak_layer(uv, aspect, progress, 16.0, 3.0, 0.70);
    }
    if globals.quality_level >= 2.0 {
        color += star_streak_layer(uv, aspect, progress, 16.0, 4.0, 0.90);
        color += star_streak_layer(uv, aspect, progress, 16.0, 5.0, 0.75);
    }

    // ── 3. Cascade particle layers ──────────────────────────────────
    // Low: 4 layers, Medium: +4 = 8, High: +8 = 16
    color += cascade_layer(uv, aspect,  0.0, progress, t);
    color += cascade_layer(uv, aspect,  1.0, progress, t);
    color += cascade_layer(uv, aspect,  2.0, progress, t);
    color += cascade_layer(uv, aspect,  3.0, progress, t);
    if globals.quality_level >= 1.0 {
        color += cascade_layer(uv, aspect,  4.0, progress, t);
        color += cascade_layer(uv, aspect,  5.0, progress, t);
        color += cascade_layer(uv, aspect,  6.0, progress, t);
        color += cascade_layer(uv, aspect,  7.0, progress, t);
    }
    if globals.quality_level >= 2.0 {
        color += cascade_layer(uv, aspect,  8.0, progress, t);
        color += cascade_layer(uv, aspect,  9.0, progress, t);
        color += cascade_layer(uv, aspect, 10.0, progress, t);
        color += cascade_layer(uv, aspect, 11.0, progress, t);
        color += cascade_layer(uv, aspect, 12.0, progress, t);
        color += cascade_layer(uv, aspect, 13.0, progress, t);
        color += cascade_layer(uv, aspect, 14.0, progress, t);
        color += cascade_layer(uv, aspect, 15.0, progress, t);
    }

    // ── 4. Micro-sparkle layers ─────────────────────────────────────
    // Low: 0, Medium: 4, High: 8
    if globals.quality_level >= 1.0 {
        color += sparkle_layer(uv, aspect, 0.0, progress, t);
        color += sparkle_layer(uv, aspect, 1.0, progress, t);
        color += sparkle_layer(uv, aspect, 2.0, progress, t);
        color += sparkle_layer(uv, aspect, 3.0, progress, t);
    }
    if globals.quality_level >= 2.0 {
        color += sparkle_layer(uv, aspect, 4.0, progress, t);
        color += sparkle_layer(uv, aspect, 5.0, progress, t);
        color += sparkle_layer(uv, aspect, 6.0, progress, t);
        color += sparkle_layer(uv, aspect, 7.0, progress, t);
    }

    // ── 5. Constellation flashes (Medium+) ──────────────────────────
    if globals.quality_level >= 1.0 {
        color += constellation_flash(uv, aspect, progress);
    }

    // ── 6. Golden dust linger (always) ──────────────────────────────
    color += golden_linger(uv, progress, t);

    // ── Overall envelope ─────────────────────────────────────────────
    // Gentle fade at the very start and very end of the transition so
    // the effect doesn't pop in/out.
    let envelope = smoothstep(0.0, 0.04, progress)
                 * smoothstep(1.0, 0.93, progress);
    color *= envelope;

    // Gamma correction
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(inv_g));

    return vec4<f32>(color, 0.0);
}
