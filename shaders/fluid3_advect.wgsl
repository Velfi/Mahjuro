// 3D semi-Lagrangian advection with linear filtered sampling.
// Also applies buoyancy (+Y force scaled by density) and dissipation.

struct FluidUniforms {
    grid_size:    vec4<f32>,
    grid_min:     vec4<f32>,
    grid_max:     vec4<f32>,
    inv_extent:   vec4<f32>,
    params:       vec4<f32>, // x=dt, y=density_dis, z=velocity_dis, w=buoyancy
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var src_vd: texture_3d<f32>;
@group(0) @binding(2) var src_samp: sampler;
@group(0) @binding(3) var dst_vd: texture_storage_3d<rgba16float, write>;

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec3<u32>(u32(fluid.grid_size.x), u32(fluid.grid_size.y), u32(fluid.grid_size.z));
    if (gid.x >= dims.x || gid.y >= dims.y || gid.z >= dims.z) {
        return;
    }
    let coord = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));

    // Cell-center velocity (used to backtrace).
    let cur = textureLoad(src_vd, coord, 0);
    let vel = cur.xyz;

    // Convert cell center to world, step backward by velocity*dt, convert back to uvw [0,1].
    let dt = fluid.params.x;
    let cell_size = (fluid.grid_max.xyz - fluid.grid_min.xyz) / fluid.grid_size.xyz;
    let world_pos = mix(
        fluid.grid_min.xyz,
        fluid.grid_max.xyz,
        (vec3<f32>(f32(gid.x), f32(gid.y), f32(gid.z)) + vec3<f32>(0.5)) / fluid.grid_size.xyz,
    );
    let back_world = world_pos - vel * dt;
    var uvw = (back_world - fluid.grid_min.xyz) * fluid.inv_extent.xyz;
    uvw = clamp(uvw, vec3<f32>(0.0), vec3<f32>(1.0));

    let sampled = textureSampleLevel(src_vd, src_samp, uvw, 0.0);

    let density_dis = fluid.params.y;
    let velocity_dis = fluid.params.z;
    let buoyancy = fluid.params.w;

    var new_vel = sampled.xyz * velocity_dis;
    let new_density = sampled.w * density_dis;
    // Buoyancy: smoke rises proportional to its density.
    new_vel.y = new_vel.y + buoyancy * new_density * dt;

    textureStore(dst_vd, coord, vec4<f32>(new_vel, new_density));
}
