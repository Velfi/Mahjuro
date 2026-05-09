// Shared punctual attenuation for `shop_glb.wgsl` and `tile_3d.wgsl`.
// Prepended at shader creation in `wgpu_renderer/init.rs`.

fn khr_spot_angle_attenuation_scene(cos_a: f32, cos_inner: f32, cos_outer: f32) -> f32 {
    let den = max(cos_inner - cos_outer, 1e-3);
    let scale = 1.0 / den;
    let offset = -cos_outer * scale;
    let angular = clamp(cos_a * scale + offset, 0.0, 1.0);
    return angular * angular;
}

/// `KHR_lights_punctual` distance attenuation (inverse square × smooth range window).
fn punctual_attenuation_khr(distance: f32, range_max: f32) -> f32 {
    let d = max(distance, 1e-4);
    var att = 1.0 / (d * d);
    if (range_max > 1e-5) {
        let x = min(d / range_max, 1.0);
        let window = max(1.0 - pow(x, 4.0), 0.0);
        att = att * window;
    }
    return att;
}

/// Document-space scaling for room GLB: `inv_doc_scale = 1/world_scale` when non-zero.
fn punctual_attenuation_with_inv_doc_scale(
    dist_world: f32,
    range_world: f32,
    inv_doc_scale: f32,
) -> f32 {
    let d = select(dist_world, dist_world * inv_doc_scale, inv_doc_scale > 1e-8);
    let r = select(range_world, range_world * inv_doc_scale, inv_doc_scale > 1e-8);
    return punctual_attenuation_khr(d, r);
}

/// Gameplay / synthetic point lights (quadratic window by radius).
fn scene_smooth_point_atten(dist: f32, radius: f32) -> f32 {
    let t = clamp(1.0 - dist / max(radius, 1.0), 0.0, 1.0);
    return t * t;
}
