struct FluidUniforms {
    grid_size: vec4<f32>,
    grid_min: vec4<f32>,
    grid_max: vec4<f32>,
    inv_extent: vec4<f32>,
    params: vec4<f32>,
    force_params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var back_a: texture_storage_3d<rgba16float, write>;
@group(0) @binding(2) var back_b: texture_storage_3d<rgba16float, write>;
@group(0) @binding(3) var forward_a: texture_storage_3d<rgba16float, write>;
@group(0) @binding(4) var forward_b: texture_storage_3d<rgba16float, write>;

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec3<u32>(u32(fluid.grid_size.x), u32(fluid.grid_size.y), u32(fluid.grid_size.z));
    if (gid.x >= dims.x || gid.y >= dims.y || gid.z >= dims.z) {
        return;
    }
    let coord = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));
    let uvw =
        (vec3<f32>(f32(gid.x), f32(gid.y), f32(gid.z)) + vec3<f32>(0.5)) / fluid.grid_size.xyz;
    let id = vec4<f32>(uvw, 0.0);
    textureStore(back_a, coord, id);
    textureStore(back_b, coord, id);
    textureStore(forward_a, coord, id);
    textureStore(forward_b, coord, id);
}
