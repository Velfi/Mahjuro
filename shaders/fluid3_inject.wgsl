// 3D fluid injection — gaussian splat of velocity + density per impulse.
//
// Reads source vel/density via texture_3d, writes destination via storage texture.

struct FluidUniforms {
    grid_size:    vec4<f32>, // xyz = grid dims (cells), w = unused
    grid_min:     vec4<f32>, // xyz = world min,         w = unused
    grid_max:     vec4<f32>, // xyz = world max,         w = unused
    inv_extent:   vec4<f32>, // xyz = 1/(max-min),       w = unused
    params:       vec4<f32>, // x=dt, y=density_dis, z=velocity_dis, w=buoyancy
};

struct InjectionPoint {
    pos_radius: vec4<f32>,   // xyz=world pos,  w=radius (world units)
    vel_density: vec4<f32>,  // xyz=world vel,  w=density strength
};

// Must stay in sync with `MAX_INJECTIONS` in `src/render/fluid.rs`.
const MAX_INJECTIONS: u32 = 64u;
struct InjectionParams {
    points: array<InjectionPoint, 64>,
    active_count: vec4<u32>, // x=count
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var<uniform> injection: InjectionParams;
@group(0) @binding(2) var src_vd: texture_3d<f32>;
@group(0) @binding(3) var dst_vd: texture_storage_3d<rgba16float, write>;

fn cell_to_world(c: vec3<f32>) -> vec3<f32> {
    let uvw = (c + vec3<f32>(0.5)) / fluid.grid_size.xyz;
    return mix(fluid.grid_min.xyz, fluid.grid_max.xyz, uvw);
}

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec3<u32>(u32(fluid.grid_size.x), u32(fluid.grid_size.y), u32(fluid.grid_size.z));
    if (gid.x >= dims.x || gid.y >= dims.y || gid.z >= dims.z) {
        return;
    }
    let coord = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));
    var vd = textureLoad(src_vd, coord, 0);

    let world = cell_to_world(vec3<f32>(f32(gid.x), f32(gid.y), f32(gid.z)));

    let count = injection.active_count.x;
    for (var p: u32 = 0u; p < count; p = p + 1u) {
        let pt = injection.points[p];
        let center = pt.pos_radius.xyz;
        let radius = max(pt.pos_radius.w, 0.001);
        let diff = world - center;
        let dist2 = dot(diff, diff);
        let r2 = radius * radius;
        let gauss = exp(-dist2 / (2.0 * r2));
        if (gauss > 0.0001) {
            vd.x = vd.x + pt.vel_density.x * gauss;
            vd.y = vd.y + pt.vel_density.y * gauss;
            vd.z = vd.z + pt.vel_density.z * gauss;
            vd.w = vd.w + pt.vel_density.w * gauss;
        }
    }

    textureStore(dst_vd, coord, vd);
}
