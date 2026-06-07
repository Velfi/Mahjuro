// Procedural moon-over-water vignette for the victory screen.
//
// Moon rendering uses proper sphere-based crescent geometry:
//   - The lit crescent is the difference between two offset spherical caps
//     projected to screen, giving a physically correct crescent shape at any
//     phase rather than a texture mask or crude notch subtraction.
//   - Limb darkening follows a simplified Eddington model (1 - u + u*cos θ).
//   - Mare regions are approximated with low-frequency fBm darkening on the
//     lit face.
//   - The corona fades with a 1/r² envelope, biases toward the lit limb from
//     `moon_phase.wgsl`, and scales with visible illuminated fraction.
//   - The sea is ink-dark with smooth depth, noise-warped swells, and scattered
//     moon-glints on wind-ruffled facets (not horizontal scanline stripes).

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
    cursor_pos: vec2<f32>,
    transition_progress: f32,
    quality_level: f32,
    moon_phase: f32,
    /// `[0]` cascade quality; `[1]` main-menu pride rainbow; `[2]` procedural moon disc (0 = hidden).
    /// Scalars (not `vec3`) so the uniform matches Rust `Globals` (48 B, not 64 B).
    _globals_pad_x: f32,
    _globals_pad_y: f32,
    _globals_pad_z: f32,
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

// Integer lattice hash — stable on Metal and DX12/FXC (no sin/fract).
fn hash_u32(x: u32) -> u32 {
    var v = x;
    v = v ^ (v >> 16u);
    v = v * 0x7feb352du;
    v = v ^ (v >> 15u);
    v = v * 0x846ca68bu;
    v = v ^ (v >> 16u);
    return v;
}

fn hash21_i(i: vec2<i32>) -> f32 {
    let n = hash_u32(u32(i.x) * 0x9e3779b9u ^ u32(i.y) * 0x85ebca6bu);
    return f32(n >> 8u) * (1.0 / 16777215.0);
}

fn hash22_i(i: vec2<i32>) -> vec2<f32> {
    return vec2<f32>(hash21_i(i), hash21_i(i + vec2<i32>(17, 59)));
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    // Quintic smoothstep — zero 1st derivative at cell edges reduces seam visibility.
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = hash21_i(i);
    let b = hash21_i(i + vec2<i32>(1, 0));
    let c = hash21_i(i + vec2<i32>(0, 1));
    let d = hash21_i(i + vec2<i32>(1, 1));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Break axis-aligned value-noise columns before fbm sampling.
fn warp_coords(p: vec2<f32>, strength: f32) -> vec2<f32> {
    let w = vec2<f32>(
        value_noise(p * 0.55 + vec2<f32>(0.0, 1.7)) - 0.5,
        value_noise(p * 0.55 + vec2<f32>(4.1, 0.0)) - 0.5,
    );
    return p + w * strength;
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
    let cell = vec2<i32>(floor(grid_uv));
    let frac_uv = fract(grid_uv);

    let rng = hash22_i(cell);
    if (rng.x > density) { return 0.0; }

    let star_pos = vec2<f32>(0.18 + rng.x * 0.64, 0.18 + rng.y * 0.64);
    let d = length(frac_uv - star_pos);
    let brightness = smoothstep(size, size * 0.10, d);

    let phase = hash21_i(cell + vec2<i32>(42, 17)) * 6.2831853;
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
    let chop_seed = vec2<f32>(
        p.x * mix(6.0, 13.0, far_mix) - time * 0.022,
        p.y * mix(5.0, 13.0, far_mix) + time * 0.010,
    );
    let chop_p = warp_coords(
        chop_seed + vec2<f32>(time * 0.013, -time * 0.007),
        mix(0.12, 0.06, far_mix),
    );
    let chop = (fbm2(chop_p) - 0.5) * 0.18;
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
    let phase = globals.moon_phase;
    let moonlight = moon_phase_moonlight_strength(phase);
    let corona_view = vec3<f32>(0.0, -1.0, 0.0);
    let ndotl = dot(normal, sun_dir);
    let lit_mask = smoothstep(-0.040, 0.060, ndotl) * in_disc;
    let direct_light = pow(max(ndotl, 0.0), 0.82);
    let terminator_rim = smoothstep(-0.12, 0.04, ndotl) - smoothstep(0.02, 0.18, ndotl);

    // Limb darkening on the lit face (u=0.6 is typical for the Moon).
    let ld      = limb_darkening(disc_r2, 0.60);

    // Real LRO WAC albedo — encodes mare/highland contrast and crater detail.
    let albedo  = sample_moon_albedo(disc_uv);

    // Moon colour: albedo drives the surface brightness; slightly warm ivory tint.
    let moon_body = vec3<f32>(0.93, 0.94, 0.88);
    let lit_albedo = moon_body * ld * albedo * 1.35;
    let lit_face = lit_albedo * (0.16 + direct_light * 1.55);
    let unlit = in_disc * (1.0 - lit_mask);
    let night_veil = moon_phase_night_veil_rgb();
    let shadow_detail = albedo * night_veil * moon_phase_shadow_veil_alpha(unlit) * unlit;
    let earthshine = vec3<f32>(0.016, 0.028, 0.050) * albedo * unlit * mix(0.55, 0.18, unlit)
        * (0.40 + 0.60 * ld);
    let terminator_glow = vec3<f32>(0.05, 0.08, 0.13) * albedo * terminator_rim * in_disc
        * mix(0.40, 1.0, lit_mask);
    var moon_col = lit_face * lit_mask + shadow_detail + earthshine + terminator_glow;

    // Atmospheric refraction glow right at the limb: thin bright ring.
    let limb_ring = smoothstep(1.08, 1.0, sqrt(disc_r2)) * smoothstep(0.90, 1.0, sqrt(disc_r2));
    moon_col     += vec3<f32>(1.8, 2.0, 2.3) * limb_ring * 0.22;

    // Corona: 1/r² halo with cloud-break noise; brighter on the lit limb, dim at new moon.
    let lit_frac = moon_phase_visible_lit_fraction(phase);
    let moon_disc_strength = globals._globals_pad_z;
    let corona_r    = dist / moon_r;  // 1.0 at limb, grows outward
    let corona_base = smoothstep(6.0, 1.0, corona_r) / max(corona_r * corona_r, 0.25);
    let corona_noise = 0.72 + 0.28 * fbm2(delta * vec2<f32>(3.5, 2.0) + vec2<f32>(0.0, t * 0.025));
    let corona_bias = moon_phase_corona_screen_bias(delta, corona_view, phase);
    let corona      = corona_base * corona_noise * smoothstep(6.5, 1.15, corona_r)
        * corona_bias * mix(0.06, 1.0, lit_frac);

    // ── Sky ───────────────────────────────────────────────────────────────────
    let sky_top   = vec3<f32>(0.002, 0.004, 0.010);
    let sky_mid   = vec3<f32>(0.006, 0.014, 0.032);
    let horizon_c = vec3<f32>(0.035, 0.050, 0.085);
    let moon_tint = vec3<f32>(0.82, 0.88, 1.0);   // cool blue-white

    let sky_mix = smoothstep(0.60, 0.02, uv.y);
    var color   = mix(sky_top, sky_mid, sky_mix);
    color       = mix(color, horizon_c, smoothstep(0.68, 0.48, uv.y) * 0.48);

    let haze = horizon_haze(uv, aspect, t);
    color   += vec3<f32>(0.04, 0.06, 0.09) * haze * 0.38;
    color   += aurora_haze(uv, aspect, t) * 0.55;

    let stars0 = star_layer(uv, 42.0, 0.34, 0.030, t);
    let stars1 = star_layer(uv, 85.0, 0.46, 0.020, t);
    let stars2 = star_layer(uv, 160.0, 0.54, 0.012, t);
    let water_start = 0.56;
    let star_mask = smoothstep(water_start + 0.02, 0.02, uv.y);
    let star_tint = vec3<f32>(0.85, 0.93, 1.18);
    color += star_tint * (stars0 * 1.9 + stars1 * 1.25 + stars2 * 0.7) * star_mask;

    // ── Water (ink-dark base + organic swells + noise-shimmer moon path) ───────
    let water_ink  = vec3<f32>(0.003, 0.008, 0.018);
    let water_deep = vec3<f32>(0.006, 0.014, 0.028);
    let water_mist = vec3<f32>(0.018, 0.028, 0.045);
    if (uv.y > water_start) {
        let water_y   = (uv.y - water_start) / max(1.0 - water_start, 0.001);
        let drifted_x = uv.x;
        let water_p   = vec2<f32>(drifted_x, water_y);
        let h_rip     = water_height(water_p, t);
        let h_dx      = water_height(water_p + vec2<f32>(0.0022, 0.0), t) - h_rip;
        let h_dy      = water_height(water_p + vec2<f32>(0.0, 0.0032), t) - h_rip;
        let swell     = 0.5 + 0.5 * h_rip;

        let wash_uv = vec2<f32>(drifted_x * 1.85 - t * 0.006, water_y * 0.95 + t * 0.004);
        let wash_lo = fbm2(wash_uv * 0.55 + vec2<f32>(1.7, 4.2));
        let wash_hi = fbm2(wash_uv * 1.35 + vec2<f32>(5.1, 0.8));
        let wash    = wash_lo * 0.62 + wash_hi * 0.38;

        // Phase + lit-side bias — crescent throws less light, mostly toward sunlit limb.
        let moon_rel = vec2<f32>((drifted_x - moon_center.x) * aspect, 0.20);
        let side_light = moon_phase_lit_side_bias(moon_rel, corona_view, phase);
        let sea_light = moonlight * side_light;

        // Smooth depth grade — no stepped horizontal bands.
        let depth_curve = water_y + (wash - 0.5) * 0.06;
        var water_col  = mix(water_ink, water_deep, smoothstep(0.0, 0.78, depth_curve));
        water_col      = mix(water_col, water_mist, smoothstep(0.82, 1.0, water_y) * 0.38 * moonlight);
        water_col     += vec3<f32>(0.008, 0.012, 0.020) * (wash - 0.5) * (0.45 - water_y * 0.32);
        water_col     *= mix(0.50, 1.0, sea_light);

        // Rolling swells — noise-warped phase so crests aren't parallel stripes.
        let swell_warp = swell + (wash_lo - 0.5) * 0.22 + (wash_hi - 0.5) * 0.10;
        let far_swell  = 0.20 + 0.80 * (1.0 - water_y);
        let crest_lift = smoothstep(0.52, 0.70, swell_warp)
            * (1.0 - smoothstep(0.70, 0.94, swell_warp));
        let trough_dip = smoothstep(0.48, 0.28, swell_warp)
            * smoothstep(0.06, 0.28, swell_warp);
        water_col += vec3<f32>(0.024, 0.038, 0.058) * crest_lift * far_swell * sea_light;
        water_col -= vec3<f32>(0.010, 0.015, 0.024) * trough_dip * (0.22 + 0.78 * water_y);

        let surface_n = normalize(vec3<f32>(-h_dx * 22.0, 1.0, -h_dy * 9.0));
        let view_dir  = normalize(vec3<f32>(0.0, 0.88, 0.48));
        let crest_rim = pow(1.0 - max(dot(surface_n, view_dir), 0.0), 4.6)
            * crest_lift * (0.16 + 0.84 * (1.0 - water_y));
        water_col += vec3<f32>(0.018, 0.028, 0.042) * crest_rim * sea_light;

        color = water_col;

        // Moon path — triangular column (wide at horizon) + glints on swells.
        let reflection_center = moon_center.x + h_dx * 0.54 + h_dy * 0.05;
        let path_width  = mix(0.24, 0.042, water_y);
        let path_soft   = path_width * 2.75;
        let path_wide   = path_width * 3.6;
        let reflection_dx = abs(drifted_x - reflection_center);
        let path_core   = smoothstep(path_width, 0.0, reflection_dx);
        let path_halo   = smoothstep(path_soft, 0.0, reflection_dx);
        let path_veil   = smoothstep(path_wide, 0.0, reflection_dx);

        let sparkle_uv = warp_coords(
            vec2<f32>(drifted_x * 11.0 - t * 0.038, water_y * 22.0 + t * 0.016),
            0.08,
        );
        let sparkle_coarse = fbm2(sparkle_uv);
        let sparkle_fine   = fbm2(sparkle_uv * 2.4 + vec2<f32>(3.3, 1.9));
        let sparkle        = sparkle_coarse * 0.58 + sparkle_fine * 0.42;
        let facet_mask = pow(
            max(1.0 - abs(h_dx) * 48.0 - abs(h_dy) * 16.0, 0.0),
            4.8,
        );
        let crest_glint = smoothstep(0.46, 0.82, swell_warp) * facet_mask;
        let glint = smoothstep(0.38, 0.82, sparkle) * crest_glint;
        let shimmer = 0.68 + 0.32 * sparkle * (0.55 + 0.45 * sin(t * 1.15 + sparkle * 9.0 + h_rip * 3.5));

        let path_fade = smoothstep(1.0, 0.05, water_y);
        // Bright triangular body — always present under the moon when lit.
        let tri_near = mix(1.0, 0.38, water_y);
        let tri_body = path_core * tri_near * path_fade;
        let tri_halo = path_halo * mix(0.72, 0.22, water_y) * path_fade;
        let tri_veil = path_veil * mix(0.38, 0.10, water_y) * path_fade;
        // Facet glints ride on top of the column.
        let tri_spark = path_core * (glint * shimmer * 0.85 + smoothstep(0.32, 0.78, sparkle) * 0.45)
            * path_fade;

        color += moon_tint * (tri_body * 0.62 + tri_halo * 0.28 + tri_veil * 0.12 + tri_spark * 0.48) * sea_light;

        let horizon_blend = smoothstep(0.06, 0.0, water_y);
        color = mix(color, horizon_c * 0.72 * moonlight, horizon_blend * 0.22);
    }

    // ── Composite moon over sky ───────────────────────────────────────────────
    // `_globals_pad_z` = 0 hides the procedural disc when a 3D `MoonObject` is
    // composited on top (victory screen); corona and sky sheen stay on.
    if (uv.y <= water_start) {
        // When the 3D moon replaces the procedural disc, keep a softer sky corona so
        // the mesh's phase-aware rim glow dominates without a symmetric double halo.
        let corona_scale = mix(1.0, 0.42, 1.0 - moon_disc_strength);
        color += moon_tint * corona * 1.45 * corona_scale;
        color = mix(color, moon_col, in_disc * moon_disc_strength);
    }

    // Vertical silver sheen in the sky directly above the moon.
    let moon_column = smoothstep(0.20, 0.0, abs(uv.x - moon_center.x) * aspect);
    let sky_sheen   = moon_column * smoothstep(0.68, 0.20, uv.y) * smoothstep(0.56, 0.24, uv.y);
    color += vec3<f32>(0.08, 0.11, 0.18) * sky_sheen * mix(0.10, 1.0, lit_frac);

    let vignette_delta = (uv - vec2<f32>(0.5, 0.56)) * vec2<f32>(aspect * 0.86, 1.0);
    let vignette       = smoothstep(1.02, 0.08, length(vignette_delta));
    color *= vignette;

    let inv_g = 1.0 / max(globals.gamma, 0.01);
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(inv_g));
    return vec4<f32>(color, 0.0);
}
