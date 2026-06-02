// Xbox-style projected shadow sampling — one 2D depth layer per punctual caster.

struct ShadowGlobals {
    params: vec4<f32>,
    counts: vec4<f32>,
    point_view_proj: array<mat4x4<f32>, 16>,
    point_light_layer: array<vec4<f32>, 4>,
    contact_ao_view_proj: mat4x4<f32>,
};

fn punctual_shadow_layer(light_index: u32) -> i32 {
    let block = light_index / 4u;
    let slot = light_index % 4u;
    let v = shadow_globals.point_light_layer[block];
    switch slot {
        case 0u: { return i32(v.x); }
        case 1u: { return i32(v.y); }
        case 2u: { return i32(v.z); }
        default: { return i32(v.w); }
    }
}

@group(2) @binding(0) var<uniform> shadow_globals: ShadowGlobals;
@group(2) @binding(1) var point_shadows: texture_depth_2d_array;
@group(2) @binding(2) var spot_shadows: texture_depth_2d_array;
@group(2) @binding(3) var shadow_samp: sampler_comparison;
@group(2) @binding(4) var contact_ao_map: texture_2d<f32>;
@group(2) @binding(5) var contact_ao_samp: sampler;
@group(2) @binding(6) var contact_baked_depth: texture_2d<f32>;

fn sample_projected_depth(
    lvp: mat4x4<f32>,
    depth_tex: texture_depth_2d_array,
    layer: i32,
    world_pos: vec3<f32>,
    bias: f32,
) -> f32 {
    let lp = lvp * vec4<f32>(world_pos, 1.0);
    let proj = lp.xyz / lp.w;
    if (proj.z < 0.0 || proj.z > 1.0) {
        return 1.0;
    }
    let uv = vec2<f32>(proj.x * 0.5 + 0.5, proj.y * -0.5 + 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return 1.0;
    }
    let depth_ref = proj.z - bias;
    let texel = shadow_globals.params.z;
    var sum = 0.0;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let off = vec2<f32>(f32(dx), f32(dy)) * texel;
            sum = sum + textureSampleCompare(depth_tex, shadow_samp, uv + off, layer, depth_ref);
        }
    }
    return sum / 9.0;
}

fn sample_point_projected_shadow(light_index: u32, world_pos: vec3<f32>) -> f32 {
    let layer = punctual_shadow_layer(light_index);
    if (layer < 0) {
        return 1.0;
    }
    let caster_count = u32(shadow_globals.counts.x);
    if (u32(layer) >= caster_count) {
        return 1.0;
    }
    let bias = shadow_globals.params.y;
    return sample_projected_depth(
        shadow_globals.point_view_proj[u32(layer)],
        point_shadows,
        layer,
        world_pos,
        bias,
    );
}

fn sample_contact_ao(world_pos: vec3<f32>) -> f32 {
    if (shadow_globals.counts.z < 0.5) {
        return 1.0;
    }
    let wp = world_pos * shadow_globals.counts.w;
    let lp = shadow_globals.contact_ao_view_proj * vec4<f32>(wp, 1.0);
    let proj = lp.xyz / lp.w;
    if (proj.z < 0.0 || proj.z > 1.0) {
        return 1.0;
    }
    let uv = vec2<f32>(proj.x * 0.5 + 0.5, proj.y * -0.5 + 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return 1.0;
    }
    let depth_eps = shadow_globals.counts.y;
    if (depth_eps > 0.0) {
        let dims = textureDimensions(contact_baked_depth);
        let coord = vec2<i32>(
            i32(clamp(uv.x * f32(dims.x), 0.0, f32(dims.x) - 1.0)),
            i32(clamp(uv.y * f32(dims.y), 0.0, f32(dims.y) - 1.0)),
        );
        let baked_d = textureLoad(contact_baked_depth, coord, 0).r;
        if (abs(baked_d - proj.z) > depth_eps) {
            return 1.0;
        }
    }
    return textureSample(contact_ao_map, contact_ao_samp, uv).r;
}

fn punctual_shadow_vis(light_idx: u32, world_pos: vec3<f32>) -> f32 {
    if (shadow_globals.params.x < 0.5) {
        return 1.0;
    }
    return mix(0.08, 1.0, sample_point_projected_shadow(light_idx, world_pos));
}

fn combined_mesh_shadow_vis(world_pos: vec3<f32>) -> f32 {
    if (shadow_globals.params.x < 0.5) {
        return 1.0;
    }
    var vis = 1.0;
    let caster_count = u32(shadow_globals.counts.x);
    for (var layer = 0u; layer < caster_count; layer = layer + 1u) {
        let bias = shadow_globals.params.y;
        vis = min(
            vis,
            sample_projected_depth(
                shadow_globals.point_view_proj[layer],
                point_shadows,
                i32(layer),
                world_pos,
                bias,
            ),
        );
    }
    return vis;
}
