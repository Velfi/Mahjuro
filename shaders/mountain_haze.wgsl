// Procedural mountain-haze atmosphere.
//
// Fullscreen triangle with FBM-based scrolling fog, layered over the
// background additively. Designed as a low-cost alternative to the
// volumetric fluid sim for scenes that want a "foggy mountain night" wash
// without depth-accurate scattering — items still read cleanly because the
// haze is a screen-space effect, not a 3D volume.
//
// Art-direction knobs (density / colour / horizon position / drift speed)
// arrive through the `HazeParams` uniform at group 1, bound from
// `VolumetricTuning` every frame — live-editable from the Volumetric
// debug overlay.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
    cursor_pos: vec2<f32>,
};

struct HazeParams {
    // xyz = colour, w = density multiplier
    color_density: vec4<f32>,
    // x = horizon y (0..1), y = drift-speed multiplier,
    // z = vertical-wall center x (0..1), w = wall half-width in UV (0 = full-width)
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> haze: HazeParams;

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
    // uv.y = 0 at top of screen, 1 at bottom.
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    return out;
}

// ── Hash / noise primitives ──────────────────────────────────────────
// Integer-bit hash. The classic `fract(sin(dot(...)) * 43758.5453)`
// trick relies on `sin` of a very large argument, which is
// implementation-defined: Metal gives a nice spread, but DX12/Vulkan
// shader compilers on Windows often round adjacent inputs to the same
// value, so neighbouring lattice cells collapse to identical hashes
// and the FBM lattice becomes visibly blocky.
fn hash21(p: vec2<f32>) -> f32 {
    var q = vec2<u32>(bitcast<u32>(p.x), bitcast<u32>(p.y));
    q = q * vec2<u32>(1597334673u, 3812015801u);
    let n = (q.x ^ q.y) * 1597334673u;
    return f32(n) * (1.0 / 4294967296.0);
}

// Value noise: bilinear-interpolated hash grid. Cheap, smooth enough for
// fog at screen-resolution scales.
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f); // smoothstep interpolant
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Fractal Brownian motion: four octaves of value noise.
fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var pp = p;
    for (var i: i32 = 0; i < 4; i = i + 1) {
        v = v + amp * vnoise(pp);
        pp = pp * 2.03 + vec2<f32>(13.1, -7.7);
        amp = amp * 0.5;
    }
    return v;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let density_mul = haze.color_density.w;
    // Zero density: skip the whole effect. Lets the overlay turn the haze
    // completely off without removing the render op from the frame list.
    if (density_mul <= 0.0) {
        return vec4<f32>(0.0);
    }

    let uv = in.uv;
    let drift_speed = haze.params.y;
    let t = globals.time * drift_speed;
    let aspect = globals.screen.x / globals.screen.y;
    var p = vec2<f32>(uv.x * aspect, uv.y);

    // ── Scroll & churn ───────────────────────────────────────────────
    let drift_a = vec2<f32>(t * 0.012, t * -0.004);
    let drift_b = vec2<f32>(t * 0.020, t * 0.006);
    let layer_a = fbm((p + drift_a) * 1.2);
    let layer_b = fbm((p + drift_b) * 2.4 + vec2<f32>(11.3, 5.1));
    var fog = layer_a * 0.70 + layer_b * 0.30;

    // ── Vertical density profile ─────────────────────────────────────
    let horizon_y = haze.params.x;
    let band = 1.0 - smoothstep(0.0, 0.45, abs(uv.y - horizon_y));
    let vertical_profile = 0.45 + band * 0.90;

    // ── Optional vertical fog slab (gameplay fog wall) ───────────────
    let wall_cx = haze.params.z;
    let wall_hw = haze.params.w;
    var horizontal_profile = 1.0;
    if (wall_hw > 1e-4) {
        let dx = abs(uv.x - wall_cx);
        horizontal_profile = 1.0 - smoothstep(wall_hw * 0.55, wall_hw * 1.15, dx);
    }

    // ── Breathing ────────────────────────────────────────────────────
    let breathe = 0.92 + 0.08 * sin(globals.time * 0.28);

    let density = clamp(
        pow(fog, 1.3) * vertical_profile * horizontal_profile * breathe * density_mul,
        0.0,
        1.0,
    );

    // ── Colour grading ───────────────────────────────────────────────
    // Base colour from the uniform; horizon gets a faintly warmer lift so
    // the band reads as condensation catching ambient moonlight, not a
    // flat rectangle of solid fog.
    let base = haze.color_density.rgb;
    let horizon_tint = base * vec3<f32>(1.15, 1.05, 0.95);
    let tint = mix(base, horizon_tint, band);

    // Edge vignette so corners don't fight HUD chrome.
    let d_corner = length((uv - vec2<f32>(0.5, 0.5)) * vec2<f32>(aspect * 0.9, 1.0));
    let vignette = 1.0 - smoothstep(0.55, 0.95, d_corner) * 0.4;

    var color = tint * density * vignette;

    let inv_g = 1.0 / max(globals.gamma, 0.01);
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(inv_g));

    return vec4<f32>(color, 0.0);
}
