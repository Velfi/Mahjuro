// Vorticity confinement for the 3D smoke field.
//
// Reads the divergence-free projected velocity field and injects a small
// force toward stronger curl magnitude. This is a standard way to recover
// visually important rolling turbulence on coarse Eulerian grids without
// the instability of inverse-advection schemes.

struct FluidUniforms {
    grid_size:    vec4<f32>,
    grid_min:     vec4<f32>,
    grid_max:     vec4<f32>,
    inv_extent:   vec4<f32>,
    params:       vec4<f32>, // x=dt, y=density_dis, z=velocity_dis, w=buoyancy
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var src_vd: texture_3d<f32>;
@group(0) @binding(2) var dst_vd: texture_storage_3d<rgba16float, write>;

fn clamp_coord(c: vec3<i32>, dims: vec3<i32>) -> vec3<i32> {
    return vec3<i32>(
        clamp(c.x, 0, dims.x - 1),
        clamp(c.y, 0, dims.y - 1),
        clamp(c.z, 0, dims.z - 1),
    );
}

fn load_vd(c: vec3<i32>, dims: vec3<i32>) -> vec4<f32> {
    return textureLoad(src_vd, clamp_coord(c, dims), 0);
}

fn curl_at(c: vec3<i32>, dims: vec3<i32>) -> vec3<f32> {
    let vl = load_vd(c + vec3<i32>(-1, 0, 0), dims).xyz;
    let vr = load_vd(c + vec3<i32>( 1, 0, 0), dims).xyz;
    let vb = load_vd(c + vec3<i32>(0, -1, 0), dims).xyz;
    let vt = load_vd(c + vec3<i32>(0,  1, 0), dims).xyz;
    let vn = load_vd(c + vec3<i32>(0, 0, -1), dims).xyz;
    let vf = load_vd(c + vec3<i32>(0, 0,  1), dims).xyz;

    return vec3<f32>(
        0.5 * ((vf.y - vn.y) - (vt.z - vb.z)),
        0.5 * ((vr.z - vl.z) - (vf.x - vn.x)),
        0.5 * ((vt.x - vb.x) - (vr.y - vl.y)),
    );
}

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims_u = vec3<u32>(u32(fluid.grid_size.x), u32(fluid.grid_size.y), u32(fluid.grid_size.z));
    if (gid.x >= dims_u.x || gid.y >= dims_u.y || gid.z >= dims_u.z) {
        return;
    }

    let dims = vec3<i32>(i32(dims_u.x), i32(dims_u.y), i32(dims_u.z));
    let c = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));
    var vd = load_vd(c, dims);

    let curl_c = curl_at(c, dims);
    let mag_xp = length(curl_at(c + vec3<i32>(1, 0, 0), dims));
    let mag_xm = length(curl_at(c + vec3<i32>(-1, 0, 0), dims));
    let mag_yp = length(curl_at(c + vec3<i32>(0, 1, 0), dims));
    let mag_ym = length(curl_at(c + vec3<i32>(0, -1, 0), dims));
    let mag_zp = length(curl_at(c + vec3<i32>(0, 0, 1), dims));
    let mag_zm = length(curl_at(c + vec3<i32>(0, 0, -1), dims));

    let grad = 0.5 * vec3<f32>(
        mag_xp - mag_xm,
        mag_yp - mag_ym,
        mag_zp - mag_zm,
    );
    let grad_len = length(grad);
    let curl_len = length(curl_c);

    if (grad_len > 1e-5 && curl_len > 1e-5) {
        let n = grad / grad_len;
        // Scale with buoyancy so the existing smoke intensity presets also
        // control how energetic the turbulence enhancement feels.
        let confinement = fluid.params.w * 0.12;
        let smoke = clamp(vd.w, 0.0, 1.0);
        let force = cross(n, curl_c) * confinement * smoke;
        let new_vel = vd.xyz + force * fluid.params.x;
        vd = vec4<f32>(new_vel, vd.w);
    }

    // Preserve the solid table floor behavior after confinement too.
    if (c.y == 0) {
        vd.y = max(vd.y, 0.0);
    }

    textureStore(dst_vd, c, vd);
}
