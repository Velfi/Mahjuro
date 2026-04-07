/// Directional light beam with volumetric dust motes.
/// Rendered behind hinted tiles to show meld-completion candidates.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
) -> VsOut {
    let x = rect.x + corner.x * rect.z;
    let y = rect.y + corner.y * rect.w;
    let nx = (x / globals.screen.x) * 2.0 - 1.0;
    let ny = 1.0 - (y / globals.screen.y) * 2.0;
    var out: VsOut;
    out.clip_pos = vec4<f32>(nx, ny, 0.0, 1.0);
    out.uv = corner;
    out.color = color;
    return out;
}

/// Simple hash for procedural noise (returns 0..1).
fn hash(p: vec2<f32>) -> f32 {
    var h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453);
}

/// Smooth value noise.
fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f); // smoothstep interpolant

    let a = hash(i);
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));

    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

/// Layered noise for dust volume.
fn dust(p: vec2<f32>, t: f32) -> f32 {
    // Drift slowly along the light direction.
    let drift = vec2<f32>(0.7, -0.5) * t * 0.4;
    let q = p + drift;
    var n = value_noise(q * 6.0) * 0.5;
    n += value_noise(q * 12.0 + vec2<f32>(3.7, 1.2)) * 0.3;
    n += value_noise(q * 24.0 + vec2<f32>(7.1, 5.3)) * 0.2;
    return n;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = globals.time;

    // Light beam axis: from upper-right toward lower-left.
    let beam_dir = normalize(vec2<f32>(0.7, -0.5));
    let beam_perp = vec2<f32>(-beam_dir.y, beam_dir.x);

    // Centered UV.
    let centered = uv - vec2<f32>(0.5, 0.5);

    // Project onto beam axis and perpendicular.
    let along = dot(centered, beam_dir);
    let across = dot(centered, beam_perp);

    // Beam shape: bright in center, fading to edges.
    // Along axis: strongest at top-right, fading toward bottom-left.
    let along_fade = smoothstep(-0.6, 0.3, along);
    // Across axis: tight Gaussian falloff from beam center.
    let beam_width = 0.10 + along * 0.08; // narrow cone, slight spread
    let across_fade = exp(-across * across / (2.0 * beam_width * beam_width));

    let beam = along_fade * across_fade;

    // Volumetric dust motes.
    let dust_val = dust(uv, t);
    // Bright sparkles where noise peaks.
    let sparkle = smoothstep(0.55, 0.75, dust_val);
    // Softer volume where noise is moderate.
    let volume = smoothstep(0.25, 0.6, dust_val) * 0.5;

    let dust_contrib = (volume + sparkle * 0.6) * beam;

    // Soft pulse on the overall beam.
    let pulse = sin(t * 1.5) * 0.1 + 0.9;

    // Final alpha: beam glow + dust, modulated by instance intensity.
    let glow = beam * 0.35 + dust_contrib * 0.4;
    let alpha = glow * pulse * in.color.a;

    // Vignette at edges of the quad to avoid hard cutoffs.
    let edge = smoothstep(0.0, 0.08, uv.x) * smoothstep(1.0, 0.92, uv.x)
             * smoothstep(0.0, 0.08, uv.y) * smoothstep(1.0, 0.92, uv.y);

    return vec4<f32>(in.color.rgb * alpha * edge, alpha * edge);
}
