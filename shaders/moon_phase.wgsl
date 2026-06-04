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

/// Hard terminator with a hairline soften (avoids aliasing, not a foggy ramp).
fn moon_phase_lit_mask(n_world: vec3<f32>, view_dir: vec3<f32>, phase: f32) -> f32 {
    let ndotl = moon_phase_ndotl(n_world, view_dir, phase);
    return smoothstep(-0.008, 0.012, ndotl);
}

fn moon_hub_phase_emissive(
    albedo: vec3<f32>,
    n_world: vec3<f32>,
    view_dir: vec3<f32>,
    phase: f32,
) -> vec3<f32> {
    let lit_mask = moon_phase_lit_mask(n_world, view_dir, phase);

    let mu = max(dot(n_world, normalize(view_dir)), 0.0);
    let limb_att = mix(0.82, 1.0, smoothstep(0.15, 0.65, mu));

    let moon_body = vec3<f32>(0.94, 0.95, 0.90);
    let lit = moon_body * albedo * 1.55 * lit_mask * limb_att;

    // Dark hemisphere: nearly black; earthshine only deep in shadow (not at terminator).
    let shadow_depth = (1.0 - lit_mask) * (1.0 - lit_mask);
    let dark = albedo * vec3<f32>(0.006, 0.009, 0.016) * shadow_depth;
    let earthshine = vec3<f32>(0.006, 0.009, 0.016) * albedo * shadow_depth * 0.35;

    return lit + dark + earthshine;
}
