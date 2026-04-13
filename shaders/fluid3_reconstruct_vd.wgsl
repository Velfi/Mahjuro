struct FluidUniforms {
    grid_size: vec4<f32>,
    grid_min: vec4<f32>,
    grid_max: vec4<f32>,
    inv_extent: vec4<f32>,
    params: vec4<f32>,
    force_params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var back_map: texture_3d<f32>;
@group(0) @binding(2) var origin_vd: texture_3d<f32>;
@group(0) @binding(3) var delta_vd: texture_3d<f32>;
@group(0) @binding(4) var samp: sampler;
@group(0) @binding(5) var dst_vd: texture_storage_3d<rgba16float, write>;

fn sample_vd(tex: texture_3d<f32>, uvw: vec3<f32>) -> vec4<f32> {
    let dims = textureDimensions(tex);
    let full_dims = vec3<f32>(f32(dims.x), f32(dims.y), f32(dims.z));
    let active_dims = fluid.grid_size.xyz;
    let min_uvw = 0.5 / full_dims;
    let max_uvw = (active_dims - 0.5) / full_dims;
    return textureSampleLevel(tex, samp, mix(min_uvw, max_uvw, uvw), 0.0);
}

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec3<u32>(u32(fluid.grid_size.x), u32(fluid.grid_size.y), u32(fluid.grid_size.z));
    if (gid.x >= dims.x || gid.y >= dims.y || gid.z >= dims.z) {
        return;
    }

    let coord = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));
    let map_pos = textureLoad(back_map, coord, 0).xyz;
    let origin = sample_vd(origin_vd, map_pos);
    let delta = sample_vd(delta_vd, map_pos);
    let state = origin + delta;
    textureStore(
        dst_vd,
        coord,
        vec4<f32>(state.xyz * fluid.params.z, max(state.w * fluid.params.y, 0.0)),
    );
}
