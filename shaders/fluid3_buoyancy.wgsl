// Apply cooling, temperature-driven buoyancy, and near-floor damping.

struct FluidUniforms {
    grid_size:    vec4<f32>,
    grid_min:     vec4<f32>,
    grid_max:     vec4<f32>,
    inv_extent:   vec4<f32>,
    params:       vec4<f32>, // x=dt, y=density_dis, z=velocity_dis, w=buoyancy
    force_params: vec4<f32>, // x=temp_dissipation, y=turbulence, z=floor_damping, w=density_mix
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var src_vd: texture_3d<f32>;
@group(0) @binding(2) var src_temp: texture_3d<f32>;
@group(0) @binding(3) var dst_vd: texture_storage_3d<rgba16float, write>;
@group(0) @binding(4) var dst_temp: texture_storage_3d<r32float, write>;

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec3<u32>(u32(fluid.grid_size.x), u32(fluid.grid_size.y), u32(fluid.grid_size.z));
    if (gid.x >= dims.x || gid.y >= dims.y || gid.z >= dims.z) {
        return;
    }
    let coord = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));
    let world_pos = mix(
        fluid.grid_min.xyz,
        fluid.grid_max.xyz,
        (vec3<f32>(f32(gid.x), f32(gid.y), f32(gid.z)) + vec3<f32>(0.5)) / fluid.grid_size.xyz,
    );

    var vd = textureLoad(src_vd, coord, 0);
    var temp = max(textureLoad(src_temp, coord, 0).x, 0.0);

    let extent_y = max(fluid.grid_max.y - fluid.grid_min.y, 1e-3);
    let height_frac = clamp((world_pos.y - fluid.grid_min.y) / extent_y, 0.0, 1.0);
    let cooling = 1.0 - smoothstep(0.10, 0.92, height_frac);
    let smoke = clamp(vd.w, 0.0, 1.0);
    let hot_lift = temp * fluid.params.w * fluid.params.x * cooling;
    let density_drag = smoke * fluid.force_params.w * fluid.params.x * 1.4;
    vd.y = vd.y + hot_lift - density_drag;

    // Let hot plumes spread laterally as they climb so the column opens up
    // rather than remaining a rigid vertical jet.
    let spread_band = smoothstep(0.35, 0.90, height_frac);
    vd.x = vd.x * mix(1.0, 0.988, spread_band);
    vd.z = vd.z * mix(1.0, 0.988, spread_band);

    // Near the table, favor shearing/sliding over noisy recirculation.
    let floor_band = 1.0 - smoothstep(0.02, 0.16, height_frac);
    let floor_damp = mix(1.0, 1.0 - fluid.force_params.z, floor_band);
    vd.x = vd.x * floor_damp;
    vd.z = vd.z * floor_damp;
    vd.y = vd.y * mix(1.0, 0.80, floor_band);

    // Feather the plume as it approaches the top of the volume.
    let ceiling_band = smoothstep(0.72, 0.98, height_frac);
    vd.x = vd.x * mix(1.0, 1.035, ceiling_band);
    vd.z = vd.z * mix(1.0, 1.035, ceiling_band);
    vd.y = vd.y * mix(1.0, 0.975, ceiling_band);

    temp = temp * mix(0.995, 0.90, height_frac);

    if (coord.y == 0) {
        vd.y = max(vd.y, 0.0);
        vd.w = max(vd.w, 0.0);
    }

    textureStore(dst_vd, coord, vd);
    textureStore(dst_temp, coord, vec4<f32>(temp, 0.0, 0.0, 0.0));
}
