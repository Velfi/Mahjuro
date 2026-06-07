// Synodic lunar phase shading for the main-menu `MoonObject` mesh.
//
// `phase` is 0..1 (0 = new, 0.25 = first quarter, 0.5 = full, 0.75 = last quarter).
// Sun direction is derived from observer view so gibbous phases are not locked to 50% lit.

fn moon_phase_orbit_tangent(view_dir: vec3<f32>) -> vec3<f32> {
    let O = normalize(view_dir);
    // Z-up world; stable axis in the plane perpendicular to the view ray.
    var tangent = cross(vec3<f32>(0.0, 0.0, 1.0), O);
    if (dot(tangent, tangent) < 1e-8) {
        tangent = cross(vec3<f32>(1.0, 0.0, 0.0), O);
    }
    return normalize(tangent);
}

fn moon_phase_sun_dir_world(view_dir: vec3<f32>, phase: f32) -> vec3<f32> {
    // 0 = new (sun opposite viewer), 0.5 = full (sun ~ viewer direction).
    let phase_angle = phase * 6.2831853;
    let O = normalize(view_dir);
    let tangent = moon_phase_orbit_tangent(view_dir);
    return normalize(-O * cos(phase_angle) + tangent * sin(phase_angle));
}

fn moon_phase_ndotl(n_world: vec3<f32>, view_dir: vec3<f32>, phase: f32) -> f32 {
    return dot(n_world, moon_phase_sun_dir_world(view_dir, phase));
}

/// Terminator with a soft ink-blue falloff (albedo detail bleeds through shadow).
fn moon_phase_lit_mask(n_world: vec3<f32>, view_dir: vec3<f32>, phase: f32) -> f32 {
    let ndotl = moon_phase_ndotl(n_world, view_dir, phase);
    return smoothstep(-0.040, 0.060, ndotl);
}

/// Ink-blue shadow veil — same family as moonlit-water sea tones.
fn moon_phase_night_veil_rgb() -> vec3<f32> {
    return vec3<f32>(0.007, 0.016, 0.032);
}

/// Shadow-side veil strength (`unlit` = 1 on the dark hemisphere).
fn moon_phase_shadow_veil_alpha(unlit: f32) -> f32 {
    return mix(0.34, 0.10, unlit);
}

/// Overall moonlight on the scene (0 = new moon, 1 = full).
fn moon_phase_moonlight_strength(phase: f32) -> f32 {
    let f = moon_phase_visible_lit_fraction(phase);
    return f * (0.26 + 0.74 * f);
}

/// Lit-side bias for water / corona (brighter toward the sunlit limb).
fn moon_phase_lit_side_bias(
    offset_aspect: vec2<f32>,
    view_dir: vec3<f32>,
    phase: f32,
) -> f32 {
    let side = moon_phase_corona_screen_bias(offset_aspect, view_dir, phase);
    return mix(0.48, 1.0, side);
}

fn moon_hub_phase_emissive(
    albedo: vec3<f32>,
    n_world: vec3<f32>,
    view_dir: vec3<f32>,
    phase: f32,
) -> vec3<f32> {
    let lit_mask = moon_phase_lit_mask(n_world, view_dir, phase);
    let ndotl = moon_phase_ndotl(n_world, view_dir, phase);

    let mu = max(dot(n_world, normalize(view_dir)), 0.0);
    let limb_att = mix(0.82, 1.0, smoothstep(0.15, 0.65, mu));

    let moon_body = vec3<f32>(0.94, 0.95, 0.90);
    let lit = moon_body * albedo * 1.55 * lit_mask * limb_att;

    // Dark hemisphere: translucent ink-blue over albedo (not a flat black clip).
    let unlit = 1.0 - lit_mask;
    let veil = moon_phase_night_veil_rgb();
    let shadow = albedo * veil * moon_phase_shadow_veil_alpha(unlit) * unlit;

    let earth_tint = vec3<f32>(0.016, 0.028, 0.050);
    let earthshine = albedo * earth_tint * unlit * mix(0.55, 0.18, unlit);

    // Terminator bloom — mostly on the lit side of the boundary.
    let term_band = smoothstep(-0.10, 0.02, ndotl) - smoothstep(0.02, 0.16, ndotl);
    let term_glow = albedo * vec3<f32>(0.05, 0.08, 0.13) * term_band * mix(0.40, 1.0, lit_mask);

    return lit + shadow + earthshine + term_glow;
}

/// Fraction of the disc visible as sunlit from the current view (0 = new, 1 = full).
fn moon_phase_visible_lit_fraction(phase: f32) -> f32 {
    let phase_angle = phase * 6.2831853;
    return clamp((1.0 - cos(phase_angle)) * 0.5, 0.0, 1.0);
}

/// Bias a screen-space corona toward the lit limb (Z-up; victory / moonlit-water camera).
///
/// `offset_aspect` uses moonlit-water UV deltas (x = right, y = down). The sun
/// direction is projected onto the image plane — do not apply mesh rotation here;
/// phase shading already accounts for `MoonObject` orientation in world space.
fn moon_phase_corona_screen_bias(
    offset_aspect: vec2<f32>,
    view_dir: vec3<f32>,
    phase: f32,
) -> f32 {
    let r2 = dot(offset_aspect, offset_aspect);
    if (r2 < 1e-8) {
        return 1.0;
    }
    let V = normalize(view_dir);
    let sun_w = moon_phase_sun_dir_world(V, phase);
    // Image plane for camera on −Y with Z-up: x → screen right, −z → screen up (uv y down).
    let sun_plane = sun_w - V * dot(sun_w, V);
    let sun_len2 = dot(sun_plane, sun_plane);
    if (sun_len2 < 1e-8) {
        return moon_phase_visible_lit_fraction(phase);
    }
    let sun_n = sun_plane * inverseSqrt(sun_len2);
    let sun_screen = vec2<f32>(sun_n.x, -sun_n.z);
    let screen_len2 = dot(sun_screen, sun_screen);
    if (screen_len2 < 1e-8) {
        return moon_phase_visible_lit_fraction(phase);
    }
    let sun_screen_n = sun_screen * inverseSqrt(screen_len2);
    let dir = offset_aspect * inverseSqrt(r2);
    let align = max(dot(dir, sun_screen_n), 0.0);
    // Lit side carries most of the halo; shadow side keeps a faint atmospheric bleed.
    return mix(0.10, 1.0, pow(align, 0.72));
}

/// Procedural outer glow on the 3D moon mesh — lit-side rim + terminator bloom.
fn moon_hub_phase_outer_glow(
    n_world: vec3<f32>,
    view_dir: vec3<f32>,
    phase: f32,
) -> vec3<f32> {
    let V = normalize(view_dir);
    let lit_mask = moon_phase_lit_mask(n_world, V, phase);
    let ndotv = max(dot(n_world, V), 0.0);
    // View-facing rim on the sunlit hemisphere reads as volumetric scatter.
    let rim = pow(1.0 - ndotv, 2.6) * lit_mask;
    let ndotl = moon_phase_ndotl(n_world, V, phase);
    let terminator = smoothstep(-0.12, 0.03, ndotl) - smoothstep(0.03, 0.22, ndotl);
    let frac = moon_phase_visible_lit_fraction(phase);
    let strength = frac * (0.30 + 0.70 * frac);
    let tint = vec3<f32>(0.86, 0.91, 1.04);
    return tint * (rim * 0.55 + terminator * 0.22) * strength;
}
