// Per-punctual shadow atlas sampling (gameplay candle tiles in the live depth map).
//
// Requires `ShadowGlobals` with `punctual_params` and `punctual_lights` fields,
// plus group-2 bindings `shadow_globals`, `shadow_map`, and `shadow_samp`.

fn sample_shadow_pcf_atlas(
    lvp: mat4x4<f32>,
    atlas_rect: vec4<f32>,
    tile_texel: f32,
    world_pos: vec3<f32>,
    bias: f32,
) -> f32 {
    let lp = lvp * vec4<f32>(world_pos, 1.0);
    let proj = lp.xyz / lp.w;
    if (proj.z < 0.0 || proj.z > 1.0) {
        return 1.0;
    }
    let local_uv = vec2<f32>(proj.x * 0.5 + 0.5, proj.y * -0.5 + 0.5);
    let uv = atlas_rect.xy + local_uv * atlas_rect.zw;
    if (uv.x < atlas_rect.x || uv.x > atlas_rect.x + atlas_rect.z
        || uv.y < atlas_rect.y || uv.y > atlas_rect.y + atlas_rect.w) {
        return 1.0;
    }
    let depth_ref = proj.z - bias;
    var sum = 0.0;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let off = vec2<f32>(f32(dx), f32(dy)) * tile_texel;
            sum = sum + textureSampleCompare(shadow_map, shadow_samp, uv + off, depth_ref);
        }
    }
    return sum / 9.0;
}

fn punctual_shadow_vis(light_idx: u32, world_pos: vec3<f32>) -> f32 {
    if (shadow_globals.punctual_params.z < 0.5) {
        return 1.0;
    }
    let count = u32(shadow_globals.punctual_params.x);
    if (light_idx >= count) {
        return 1.0;
    }
    let slot = shadow_globals.punctual_lights[light_idx];
    return mix(0.18, 1.0, sample_shadow_pcf_atlas(
        slot.light_view_proj,
        slot.atlas_rect,
        shadow_globals.punctual_params.y,
        world_pos,
        shadow_globals.params.y,
    ));
}
