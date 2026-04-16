// Density injection — gaussian splats from a list of impulse points.
//
// No temperature, no pressure. Per-puff velocity is stashed in the xyz
// channels of the density texture; the advect shader reads it back the
// next frame so tile drags and scripted wind gusts still nudge smoke.

struct FluidUniforms {
    grid_size:    vec4<f32>, // xyz = grid dims, w = sim_time
    grid_min:     vec4<f32>,
    grid_max:     vec4<f32>,
    inv_extent:   vec4<f32>,
    params:       vec4<f32>,
    force_params: vec4<f32>,
};

struct InjectionPoint {
    pos_radius:  vec4<f32>,       // xyz = world pos, w = radius (world units)
    vel_density: vec4<f32>,       // xyz = world vel, w = density strength
    temperature_phase: vec4<f32>, // unused; kept so the Rust side can leave its layout alone
};

// Must stay in sync with `MAX_INJECTIONS` in `src/render/fluid.rs`.
const MAX_INJECTIONS: u32 = 64u;
struct InjectionParams {
    points: array<InjectionPoint, 64>,
    active_count: vec4<u32>, // x = count
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var<uniform> injection: InjectionParams;
@group(0) @binding(2) var src_density: texture_3d<f32>;
@group(0) @binding(3) var dst_density: texture_storage_3d<rgba16float, write>;

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
    var out = textureLoad(src_density, coord, 0);

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
            // Velocity: accumulate per-puff velocity into xyz so the advect
            // pass can transport nearby density accordingly. Density: clamp
            // at zero so scripted "remove smoke" impulses can dig wells
            // without going negative (which would soak up later positive
            // injections invisibly and present as "the cursor stopped
            // emitting smoke").
            out.x = out.x + pt.vel_density.x * gauss;
            out.y = out.y + pt.vel_density.y * gauss;
            out.z = out.z + pt.vel_density.z * gauss;
            out.w = max(out.w + pt.vel_density.w * gauss, 0.0);
        }
    }

    textureStore(dst_density, coord, out);
}
