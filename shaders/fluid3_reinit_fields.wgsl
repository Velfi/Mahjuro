struct FluidUniforms {
    grid_size: vec4<f32>,
    grid_min: vec4<f32>,
    grid_max: vec4<f32>,
    inv_extent: vec4<f32>,
    params: vec4<f32>,
    force_params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var current_vd: texture_3d<f32>;
@group(0) @binding(2) var current_temp: texture_3d<f32>;
@group(0) @binding(3) var origin_vd: texture_storage_3d<rgba16float, write>;
@group(0) @binding(4) var origin_temp: texture_storage_3d<r32float, write>;
@group(0) @binding(5) var delta_vd_a: texture_storage_3d<rgba16float, write>;
@group(0) @binding(6) var delta_vd_b: texture_storage_3d<rgba16float, write>;
@group(0) @binding(7) var delta_temp_a: texture_storage_3d<r32float, write>;
@group(0) @binding(8) var delta_temp_b: texture_storage_3d<r32float, write>;

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec3<u32>(u32(fluid.grid_size.x), u32(fluid.grid_size.y), u32(fluid.grid_size.z));
    if (gid.x >= dims.x || gid.y >= dims.y || gid.z >= dims.z) {
        return;
    }
    let coord = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));
    textureStore(origin_vd, coord, textureLoad(current_vd, coord, 0));
    textureStore(origin_temp, coord, textureLoad(current_temp, coord, 0));
    textureStore(delta_vd_a, coord, vec4<f32>(0.0));
    textureStore(delta_vd_b, coord, vec4<f32>(0.0));
    textureStore(delta_temp_a, coord, vec4<f32>(0.0));
    textureStore(delta_temp_b, coord, vec4<f32>(0.0));
}
