// Synodic lunar phase shading for the main-menu `MoonObject` mesh.
//
// `phase` is 0..1 from CPU calendar data (0 = new, 0.25 = first quarter,
// 0.5 = full, 0.75 = last quarter). Uses the mesh base-color albedo texture.

fn moon_hub_phase_emissive(
    albedo: vec3<f32>,
    normal: vec3<f32>,
    view_dir: vec3<f32>,
    phase: f32,
) -> vec3<f32> {
    let phase_angle = phase * 6.2831853;
    let sun_dir = normalize(vec3<f32>(sin(phase_angle), 0.06, -cos(phase_angle)));
    let n = normalize(normal);
    let ndotl = dot(n, sun_dir);
    let lit_mask = smoothstep(-0.03, 0.02, ndotl);
    let direct_light = pow(max(ndotl, 0.0), 0.82);
    let terminator_rim = smoothstep(-0.10, 0.06, ndotl) - smoothstep(0.00, 0.16, ndotl);

    // View-dependent limb darkening (UV sphere mesh; u ≈ 0.6 for the Moon).
    let mu = max(dot(n, normalize(view_dir)), 0.0);
    let ld = 1.0 - 0.6 + 0.6 * mu;

    let moon_body = vec3<f32>(0.93, 0.94, 0.88);
    let lit_albedo = moon_body * ld * albedo * 1.35;
    let lit_face = lit_albedo * (0.16 + direct_light * 1.55);
    let dark_face = 1.0 - lit_mask;
    let earthshine = vec3<f32>(0.018, 0.026, 0.042) * dark_face * (0.40 + 0.60 * ld) * albedo;
    let terminator_glow = vec3<f32>(0.18, 0.20, 0.24) * terminator_rim;
    return lit_face * lit_mask + earthshine + terminator_glow;
}
