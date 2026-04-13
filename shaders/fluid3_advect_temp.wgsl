// Pure semi-Lagrangian advection for the temperature scalar.
//
// We use manual trilinear reconstruction from an r32float texture so the
// temperature field can stay compact while remaining decoupled from the
// velocity+density transport.

struct FluidUniforms {
    grid_size:    vec4<f32>,
    grid_min:     vec4<f32>,
    grid_max:     vec4<f32>,
    inv_extent:   vec4<f32>,
    params:       vec4<f32>,
    force_params: vec4<f32>, // x=temp_dissipation
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var vel_tex: texture_3d<f32>;
@group(0) @binding(2) var src_temp: texture_3d<f32>;
@group(0) @binding(3) var dst_temp: texture_storage_3d<r32float, write>;

fn clamp_coord(c: vec3<i32>, dims: vec3<i32>) -> vec3<i32> {
    return vec3<i32>(
        clamp(c.x, 0, dims.x - 1),
        clamp(c.y, 0, dims.y - 1),
        clamp(c.z, 0, dims.z - 1),
    );
}

fn sample_temp_trilinear(pos_cells: vec3<f32>, dims: vec3<i32>) -> f32 {
    let base_f = clamp(pos_cells, vec3<f32>(0.0), vec3<f32>(vec3<f32>(fluid.grid_size.xyz) - vec3<f32>(1.0)));
    let base = vec3<i32>(floor(base_f));
    let frac = fract(base_f);
    let hi = clamp_coord(base + vec3<i32>(1, 1, 1), dims);

    let c000 = textureLoad(src_temp, base, 0).x;
    let c100 = textureLoad(src_temp, vec3<i32>(hi.x, base.y, base.z), 0).x;
    let c010 = textureLoad(src_temp, vec3<i32>(base.x, hi.y, base.z), 0).x;
    let c110 = textureLoad(src_temp, vec3<i32>(hi.x, hi.y, base.z), 0).x;
    let c001 = textureLoad(src_temp, vec3<i32>(base.x, base.y, hi.z), 0).x;
    let c101 = textureLoad(src_temp, vec3<i32>(hi.x, base.y, hi.z), 0).x;
    let c011 = textureLoad(src_temp, vec3<i32>(base.x, hi.y, hi.z), 0).x;
    let c111 = textureLoad(src_temp, hi, 0).x;

    let c00 = mix(c000, c100, frac.x);
    let c10 = mix(c010, c110, frac.x);
    let c01 = mix(c001, c101, frac.x);
    let c11 = mix(c011, c111, frac.x);
    let c0 = mix(c00, c10, frac.y);
    let c1 = mix(c01, c11, frac.y);
    return mix(c0, c1, frac.z);
}

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims_u = vec3<u32>(u32(fluid.grid_size.x), u32(fluid.grid_size.y), u32(fluid.grid_size.z));
    if (gid.x >= dims_u.x || gid.y >= dims_u.y || gid.z >= dims_u.z) {
        return;
    }
    let dims = vec3<i32>(i32(dims_u.x), i32(dims_u.y), i32(dims_u.z));
    let coord = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));

    let vel = textureLoad(vel_tex, coord, 0).xyz;
    let world_pos = mix(
        fluid.grid_min.xyz,
        fluid.grid_max.xyz,
        (vec3<f32>(f32(gid.x), f32(gid.y), f32(gid.z)) + vec3<f32>(0.5)) / fluid.grid_size.xyz,
    );
    let back_world = world_pos - vel * fluid.params.x;
    let pos_cells =
        ((back_world - fluid.grid_min.xyz) * fluid.inv_extent.xyz) * fluid.grid_size.xyz
        - vec3<f32>(0.5);

    let temp = sample_temp_trilinear(pos_cells, dims) * fluid.force_params.x;
    textureStore(dst_temp, coord, vec4<f32>(max(temp, 0.0), 0.0, 0.0, 0.0));
}
