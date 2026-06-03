// Synodic lunar phase shading for the main-menu `MoonObject` mesh.
//
// `phase` is 0..1 (0 = new, 0.25 = first quarter, 0.5 = full, 0.75 = last quarter).
// Uses per-pixel `n_world` so the terminator stays smooth on the low-poly sphere.

fn moon_phase_sun_dir_world(model: mat4x4<f32>, phase: f32) -> vec3<f32> {
    // +180° so phase 0 (new) faces the camera with the dark hemisphere.
    let phase_angle = (phase + 0.5) * 6.2831853;
    // Synodic light sweeps the mesh equator (pole = +Z in `MoonObject` mesh space).
    let sun_mesh = vec3(cos(phase_angle), sin(phase_angle), 0.0);
    return normalize((model * vec4<f32>(sun_mesh, 0.0)).xyz);
}

fn moon_phase_ndotl(n_world: vec3<f32>, model: mat4x4<f32>, phase: f32) -> f32 {
    return dot(n_world, moon_phase_sun_dir_world(model, phase));
}

/// Hard terminator with a hairline soften (avoids aliasing, not a foggy ramp).
fn moon_phase_lit_mask(n_world: vec3<f32>, model: mat4x4<f32>, phase: f32) -> f32 {
    let ndotl = moon_phase_ndotl(n_world, model, phase);
    return smoothstep(-0.008, 0.012, ndotl);
}

fn moon_hub_phase_emissive(
    albedo: vec3<f32>,
    n_world: vec3<f32>,
    view_dir: vec3<f32>,
    phase: f32,
    model: mat4x4<f32>,
) -> vec3<f32> {
    let ndotl = moon_phase_ndotl(n_world, model, phase);
    let lit_mask = moon_phase_lit_mask(n_world, model, phase);

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
