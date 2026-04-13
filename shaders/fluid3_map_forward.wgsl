struct FluidUniforms {
    grid_size: vec4<f32>,
    grid_min: vec4<f32>,
    grid_max: vec4<f32>,
    inv_extent: vec4<f32>,
    params: vec4<f32>,
    force_params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var vel_tex: texture_3d<f32>;
@group(0) @binding(2) var src_map: texture_3d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var dst_map: texture_storage_3d<rgba16float, write>;

fn sample_uvw(tex: texture_3d<f32>, uvw: vec3<f32>) -> vec3<f32> {
    let dims = textureDimensions(tex);
    let full_dims = vec3<f32>(f32(dims.x), f32(dims.y), f32(dims.z));
    let active_dims = fluid.grid_size.xyz;
    let min_uvw = 0.5 / full_dims;
    let max_uvw = (active_dims - 0.5) / full_dims;
    return textureSampleLevel(tex, samp, mix(min_uvw, max_uvw, uvw), 0.0).xyz;
}

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec3<u32>(u32(fluid.grid_size.x), u32(fluid.grid_size.y), u32(fluid.grid_size.z));
    if (gid.x >= dims.x || gid.y >= dims.y || gid.z >= dims.z) {
        return;
    }

    let coord = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));
    let cell_uvw =
        (vec3<f32>(f32(gid.x), f32(gid.y), f32(gid.z)) + vec3<f32>(0.5)) / fluid.grid_size.xyz;
    let prev_map = sample_uvw(src_map, cell_uvw);
    let vel = sample_uvw(vel_tex, prev_map);
    let next_map = clamp(prev_map + vel * fluid.params.x * fluid.inv_extent.xyz, vec3<f32>(0.0), vec3<f32>(1.0));
    textureStore(dst_map, coord, vec4<f32>(next_map, 0.0));
}
