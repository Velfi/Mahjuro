// Procedural moon-over-water vignette for the victory screen.
//
// Moon rendering uses proper sphere-based crescent geometry:
//   - The lit crescent is the difference between two offset spherical caps
//     projected to screen, giving a physically correct crescent shape at any
//     phase rather than a texture mask or crude notch subtraction.
//   - Limb darkening follows a simplified Eddington model (1 - u + u*cos θ).
//   - Mare regions are approximated with low-frequency fBm darkening on the
//     lit face.
//   - The corona fades with a 1/r² envelope and has gentle cloud-break noise.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
    cursor_pos: vec2<f32>,
    transition_progress: f32,
    quality_level: f32,
    moon_phase: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;

// LRO WAC equirectangular albedo map (real lunar surface data).
// textureLoad only — DX12/FXC treats a separate sampler binding as an SM 5.1+
// resource array and fails pipeline creation.
@group(0) @binding(1) var moon_albedo_tex: texture_2d<f32>;

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
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm2(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.55;
    var f = p;
    for (var i = 0; i < 4; i += 1) {
        v += value_noise(f) * amp;
        f = f * 2.03 + vec2<f32>(3.7, 5.1);
        amp *= 0.5;
    }
    return v;
}

fn star_layer(uv: vec2<f32>, scale: f32, density: f32, size: f32, time: f32) -> f32 {
    let grid_uv = uv * scale;
    let cell = floor(grid_uv);
    let frac_uv = fract(grid_uv);

    let rng = hash22(cell);
    if (rng.x > density) { return 0.0; }

    let star_pos = vec2<f32>(0.18 + rng.x * 0.64, 0.18 + rng.y * 0.64);
    let d = length(frac_uv - star_pos);
    let brightness = smoothstep(size, size * 0.10, d);

    let phase = hash21(cell + 41.7) * 6.2831853;
    let twinkle = 0.72 + 0.28 * sin(time * (1.4 + rng.y * 2.1) + phase);
    return brightness * twinkle;
}

fn aurora_haze(uv: vec2<f32>, aspect: f32, time: f32) -> vec3<f32> {
    let p = vec2<f32>((uv.x - 0.5) * aspect, uv.y);
    let veil = fbm2(vec2<f32>(p.x * 2.4 + time * 0.012, p.y * 5.5 - time * 0.02));
    let ribbon = smoothstep(0.68, 0.18, abs(p.x * 0.85 + (veil - 0.5) * 0.35))
        * smoothstep(0.58, 0.08, uv.y);
    let cyan = vec3<f32>(0.06, 0.16, 0.22);
    let indigo = vec3<f32>(0.04, 0.06, 0.16);
    return mix(indigo, cyan, veil) * ribbon * 0.28;
}

// ── Moon ──────────────────────────────────────────────────────────────────────

// Simplified Eddington limb darkening: I(μ) ∝ 1 - u + u·μ
// μ = cos(angle from disc centre) ≈ sqrt(1 - r²) for a unit disc.
fn limb_darkening(r2: f32, u: f32) -> f32 {
    let mu = sqrt(max(1.0 - r2, 0.0));
    return 1.0 - u + u * mu;
}

// Sample the real LRO WAC albedo map.
// disc_uv is in [-1,1]×[-1,1] with (0,0) at disc centre.
// We project it onto a sphere and then unproject to equirectangular UV.
fn sample_moon_albedo(disc_uv: vec2<f32>) -> f32 {
    // Disc coords: x right, y up.  z = sqrt(1 - x² - y²) on the near hemisphere.
    let r2 = dot(disc_uv, disc_uv);
    if (r2 >= 1.0) { return 0.5; }
    let z  = sqrt(1.0 - r2);
    // Spherical angles from the near-face normal.
    // We show the near side (Mare Imbrium, Tranquillitatis, etc.) centred.
    // Slight axial tilt for a more natural orientation.
    let lon = atan2(disc_uv.x, z) + 3.14159265 * 0.06;   // ~11° east offset
    let lat = asin(clamp(-disc_uv.y * 0.96, -1.0, 1.0));  // slight tilt

    // Equirectangular: lon [-π,π] → u [0,1], lat [-π/2,π/2] → v [0,1]
    let u = lon / (2.0 * 3.14159265) + 0.5;
    let v = lat / 3.14159265 + 0.5;

    let dims = textureDimensions(moon_albedo_tex);
    let max_coord = vec2<i32>(dims) - vec2<i32>(1, 1);
    let px = vec2<f32>(u, v) * vec2<f32>(dims) - 0.5;
    let base = clamp(vec2<i32>(floor(px)), vec2<i32>(0, 0), max_coord);
    let f = fract(px);
    let c00 = textureLoad(moon_albedo_tex, base, 0);
    let c10 = textureLoad(moon_albedo_tex, min(base + vec2<i32>(1, 0), max_coord), 0);
    let c01 = textureLoad(moon_albedo_tex, min(base + vec2<i32>(0, 1), max_coord), 0);
    let c11 = textureLoad(moon_albedo_tex, min(base + vec2<i32>(1, 1), max_coord), 0);
    let col = mix(mix(c00, c10, f.x), mix(c01, c11, f.x), f.y);
    // Texture is RGB; luminance gives the albedo scalar.
    return dot(col.rgb, vec3<f32>(0.299, 0.587, 0.114));
}

// ── Water / atmosphere ────────────────────────────────────────────────────────

fn water_height(p: vec2<f32>, time: f32) -> f32 {
    // Perspective: the horizon compresses into finer detail, while the
    // foreground carries broader, slower swells.
    let far_mix = 1.0 - p.y;
    let x_scale = mix(16.0, 44.0, far_mix);
    let warp = (fbm2(vec2<f32>(p.x * 1.2 - time * 0.008, p.y * 2.8 + time * 0.006)) - 0.5) * 0.05;
    let long_x = p.x + warp;

    let swell0 = sin(long_x * x_scale + time * 0.42) * 0.32;
    let swell1 = sin(long_x * (x_scale * 2.1) - time * 0.56 + p.y * 1.2) * 0.12;
    let chop = (fbm2(vec2<f32>(
        p.x * mix(7.0, 16.0, far_mix) - time * 0.022,
        p.y * mix(3.5, 14.0, far_mix) + time * 0.010,
    )) - 0.5) * 0.18;
    let capillary = sin(long_x * mix(34.0, 86.0, far_mix) - time * 0.9) * 0.030;
    return swell0 + swell1 + chop + capillary;
}

fn horizon_haze(uv: vec2<f32>, aspect: f32, time: f32) -> f32 {
    let p = vec2<f32>((uv.x - 0.5) * aspect * 1.2, (uv.y - 0.57) * 9.0);
    let dist = length(p);
    let base = smoothstep(0.46, 0.0, dist);
    let breakup = 0.65 + 0.35 * fbm2(vec2<f32>(p.x * 3.0 + time * 0.05, p.y * 2.2));
    return base * breakup;
}

// ── Main ──────────────────────────────────────────────────────────────────────

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv     = in.uv;
    let t      = globals.time;
    let aspect = globals.screen.x / globals.screen.y;
    let moon_center = vec2<f32>(0.5, 0.28);

    // Moon disc radius in UV space (corrected for aspect ratio).
    let moon_r   = 0.072;
    let delta_raw = uv - moon_center;
    let delta     = delta_raw * vec2<f32>(aspect, 1.0);
    let dist      = length(delta);
    // Normalised disc coords (1.0 at the limb).
    let disc_uv   = delta / moon_r;
    let disc_r2   = dot(disc_uv, disc_uv);
    let in_disc   = smoothstep(1.02, 0.88, sqrt(disc_r2));
    let near_z    = sqrt(max(1.0 - disc_r2, 0.0));
    let normal    = normalize(vec3<f32>(disc_uv, near_z));

    // Approximate real lunar phase from CPU-side calendar data:
    // 0.0 = new, 0.25 = first quarter, 0.5 = full, 0.75 = last quarter.
    let phase_angle = globals.moon_phase * 6.2831853;
    let sun_dir = normalize(vec3<f32>(sin(phase_angle), 0.06, -cos(phase_angle)));
    let ndotl = dot(normal, sun_dir);
    let lit_mask = smoothstep(-0.03, 0.02, ndotl) * in_disc;
    let direct_light = pow(max(ndotl, 0.0), 0.82);
    let terminator_rim = smoothstep(-0.10, 0.06, ndotl) - smoothstep(0.00, 0.16, ndotl);

    // Limb darkening on the lit face (u=0.6 is typical for the Moon).
    let ld      = limb_darkening(disc_r2, 0.60);

    // Real LRO WAC albedo — encodes mare/highland contrast and crater detail.
    let albedo  = sample_moon_albedo(disc_uv);

    // Moon colour: albedo drives the surface brightness; slightly warm ivory tint.
    let moon_body = vec3<f32>(0.93, 0.94, 0.88);
    let lit_albedo = moon_body * ld * albedo * 1.35;
    let lit_face = lit_albedo * (0.16 + direct_light * 1.55);
    let dark_face = in_disc * (1.0 - lit_mask);
    let earthshine = vec3<f32>(0.018, 0.026, 0.042) * dark_face * (0.40 + 0.60 * ld);
    let terminator_glow = vec3<f32>(0.18, 0.20, 0.24) * terminator_rim * in_disc;
    var moon_col = lit_face * lit_mask + earthshine + terminator_glow;

    // Atmospheric refraction glow right at the limb: thin bright ring.
    let limb_ring = smoothstep(1.08, 1.0, sqrt(disc_r2)) * smoothstep(0.90, 1.0, sqrt(disc_r2));
    moon_col     += vec3<f32>(1.8, 2.0, 2.3) * limb_ring * 0.22;

    // Corona: 1/r² halo with cloud-break noise.
    let corona_r    = dist / moon_r;  // 1.0 at limb, grows outward
    let corona_base = smoothstep(6.0, 1.0, corona_r) / max(corona_r * corona_r, 0.25);
    let corona_noise = 0.72 + 0.28 * fbm2(delta * vec2<f32>(3.5, 2.0) + vec2<f32>(0.0, t * 0.025));
    let corona      = corona_base * corona_noise * smoothstep(6.5, 1.15, corona_r);

    // ── Sky ───────────────────────────────────────────────────────────────────
    let sky_top   = vec3<f32>(0.01, 0.03, 0.09);
    let sky_mid   = vec3<f32>(0.03, 0.08, 0.16);
    let horizon_c = vec3<f32>(0.10, 0.15, 0.24);
    let moon_tint = vec3<f32>(0.82, 0.88, 1.0);   // cool blue-white

    let sky_mix = smoothstep(0.60, 0.02, uv.y);
    var color   = mix(sky_top, sky_mid, sky_mix);
    color       = mix(color, horizon_c, smoothstep(0.68, 0.48, uv.y) * 0.65);

    let haze = horizon_haze(uv, aspect, t);
    color   += vec3<f32>(0.04, 0.06, 0.09) * haze * 0.65;
    color   += aurora_haze(uv, aspect, t);

    let stars0 = star_layer(uv, 42.0, 0.34, 0.030, t);
    let stars1 = star_layer(uv, 85.0, 0.46, 0.020, t);
    let stars2 = star_layer(uv, 160.0, 0.54, 0.012, t);
    let water_start = 0.56;
    let star_mask = smoothstep(water_start + 0.02, 0.02, uv.y);
    let star_tint = vec3<f32>(0.85, 0.93, 1.18);
    color += star_tint * (stars0 * 1.9 + stars1 * 1.25 + stars2 * 0.7) * star_mask;

    // ── Water ─────────────────────────────────────────────────────────────────
    let water_dark = vec3<f32>(0.01, 0.04, 0.08);
    let water_glow = vec3<f32>(0.14, 0.23, 0.34);
    if (uv.y > water_start) {
        let water_y   = (uv.y - water_start) / max(1.0 - water_start, 0.001);
        let drifted_x = uv.x;
        let water_p   = vec2<f32>(drifted_x, water_y);
        let h_rip     = water_height(water_p, t);
        let h_dx      = water_height(water_p + vec2<f32>(0.0022, 0.0), t) - h_rip;
        let h_dy      = water_height(water_p + vec2<f32>(0.0, 0.0032), t) - h_rip;
        let surface_n = normalize(vec3<f32>(-h_dx * 24.0, 1.0, -h_dy * 10.0));
        let view_dir  = normalize(vec3<f32>(0.0, 0.88, 0.48));
        let fresnel   = pow(1.0 - max(dot(surface_n, view_dir), 0.0), 3.6);
        let band      = 0.5 + 0.5 * h_rip;

        let depth_mix  = smoothstep(1.0, 0.0, water_y);
        let water_base = mix(water_dark, water_glow, depth_mix * 0.50);
        color = water_base;
        color += vec3<f32>(0.015, 0.025, 0.040) * smoothstep(0.24, 0.0, water_y);

        let crest = smoothstep(0.58, 0.96, band) * (0.20 + 0.80 * (1.0 - water_y));
        let trough = smoothstep(0.42, 0.08, band) * (0.35 + 0.65 * water_y);
        color += vec3<f32>(0.04, 0.07, 0.11) * crest;
        color -= vec3<f32>(0.02, 0.03, 0.05) * trough;
        color += vec3<f32>(0.05, 0.08, 0.11) * fresnel * (0.30 + 0.70 * depth_mix);

        let reflection_center = moon_center.x + h_dx * 0.60 + h_dy * 0.06;
        let reflection_width  = mix(0.18, 0.036, water_y);
        let reflection_dx     = abs(drifted_x - reflection_center);
        let reflection_column = smoothstep(reflection_width, 0.0, reflection_dx);
        let glint_mask = pow(max(1.0 - abs(h_dx) * 78.0 - abs(h_dy) * 26.0, 0.0), 6.2);
        let sparkle_noise = fbm2(vec2<f32>(drifted_x * 16.0 - t * 0.04, water_y * 86.0 + t * 0.025));
        let streaks = 0.5 + 0.5 * sin(water_y * 300.0 - t * 1.7 + sparkle_noise * 8.5);
        let breakup = smoothstep(0.62, 0.92, streaks) * smoothstep(0.46, 0.88, sparkle_noise)
            * (0.26 + 0.74 * glint_mask);
        let shimmer = 0.84 + 0.16 * sin(water_y * 185.0 - t * 1.5 + h_rip * 2.8);
        let wide_glow = smoothstep(reflection_width * 1.85, 0.0, reflection_dx)
            * (0.30 + 0.70 * smoothstep(0.30, 0.85, sparkle_noise))
            * mix(0.90, 0.36, water_y);
        let reflection = reflection_column * breakup * shimmer * mix(0.96, 0.34, water_y);
        color += moon_tint * reflection * 0.50;
        color += moon_tint * wide_glow * 0.12;

        let horizon_blend = smoothstep(0.075, 0.0, water_y);
        color = mix(color, horizon_c * 1.02, horizon_blend * 0.35);

        let far_glow = smoothstep(0.032, 0.0, abs(uv.y - water_start));
        color += vec3<f32>(0.06, 0.09, 0.12) * far_glow * 0.45;
    }

    // ── Composite moon over sky ───────────────────────────────────────────────
    // Only draw the moon above the water line.
    if (uv.y <= water_start) {
        // Corona glow added to sky first.
        color += moon_tint * corona * 1.45;
        // Moon disc composited on top with a dim, visible shadowed hemisphere.
        color = mix(color, moon_col, in_disc);
    }

    // Vertical silver sheen in the sky directly above the moon.
    let moon_column = smoothstep(0.20, 0.0, abs(uv.x - moon_center.x) * aspect);
    let sky_sheen   = moon_column * smoothstep(0.68, 0.20, uv.y) * smoothstep(0.56, 0.24, uv.y);
    color += vec3<f32>(0.14, 0.19, 0.28) * sky_sheen;

    let vignette_delta = (uv - vec2<f32>(0.5, 0.56)) * vec2<f32>(aspect * 0.86, 1.0);
    let vignette       = smoothstep(1.08, 0.12, length(vignette_delta));
    color *= vignette;

    let inv_g = 1.0 / max(globals.gamma, 0.01);
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(inv_g));
    return vec4<f32>(color, 0.0);
}
