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
    force_params: vec4<f32>, // x=temp_dissipation, y=turbulence, z=floor_damping, w=density_mix
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var src_vd: texture_3d<f32>;
@group(0) @binding(2) var src_temp: texture_3d<f32>;
@group(0) @binding(3) var dst_vd: texture_storage_3d<rgba16float, write>;
@group(0) @binding(4) var dst_temp: texture_storage_3d<r32float, write>;

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

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn vnoise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i + vec2<f32>(0.0, 0.0));
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
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
    let temp = max(textureLoad(src_temp, c, 0).x, 0.0);
    let world_pos = mix(
        fluid.grid_min.xyz,
        fluid.grid_max.xyz,
        (vec3<f32>(f32(gid.x), f32(gid.y), f32(gid.z)) + vec3<f32>(0.5)) / fluid.grid_size.xyz,
    );
    let extent_y = max(fluid.grid_max.y - fluid.grid_min.y, 1e-3);
    let height_frac = clamp((world_pos.y - fluid.grid_min.y) / extent_y, 0.0, 1.0);

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
    let smoke = clamp(vd.w, 0.0, 1.0);
    let thermal = clamp(temp, 0.0, 1.0);
    let activity = max(smoke * 0.45, thermal);
    let height_gate = smoothstep(0.06, 0.42, height_frac);
    let loft_gate = smoothstep(0.38, 0.92, height_frac);

    var new_vel = vd.xyz;

    if (grad_len > 1e-5 && curl_len > 1e-5) {
        let n = grad / grad_len;
        let confinement = fluid.force_params.y * height_gate * activity;
        let force = cross(n, curl_c) * confinement;
        new_vel = new_vel + force * fluid.params.x;
    }

    // Vorticity confinement preserves existing curl, but a very symmetric hot
    // plume can still remain too laminar on a coarse grid. Seed a small,
    // divergence-free horizontal curl field here so the plume has something
    // physical-looking to roll into without hiding the force inside advection.
    let nscale = 0.013;
    let np = vec2<f32>(world_pos.x, world_pos.z) * nscale
           + vec2<f32>(world_pos.y * nscale * 0.35, 0.0);
    let eps = 4.0;
    let psi_xp = vnoise2(np + vec2<f32>(eps * nscale, 0.0));
    let psi_xm = vnoise2(np - vec2<f32>(eps * nscale, 0.0));
    let psi_zp = vnoise2(np + vec2<f32>(0.0, eps * nscale));
    let psi_zm = vnoise2(np - vec2<f32>(0.0, eps * nscale));
    let noise_curl = vec3<f32>(
        (psi_zp - psi_zm) / (2.0 * eps),
        0.0,
        -(psi_xp - psi_xm) / (2.0 * eps),
    );
    let hot_noise_strength = fluid.force_params.y * 145.0 * thermal * height_gate;
    let loft_noise_strength = fluid.force_params.y * 48.0 * smoke * loft_gate;
    new_vel = new_vel + noise_curl * (hot_noise_strength + loft_noise_strength) * fluid.params.x;

    // Very weak high-altitude drift so the tops of the plumes separate and
    // stop looking phase-locked.
    let drift_angle = world_pos.y * 0.018 + world_pos.x * 0.006 + world_pos.z * 0.004;
    let drift = vec3<f32>(sin(drift_angle), 0.0, cos(drift_angle)) * loft_gate * smoke * 6.0;
    new_vel = new_vel + drift * fluid.params.x;

    vd = vec4<f32>(new_vel, vd.w);

    // Preserve the solid table floor behavior after confinement too.
    if (c.y == 0) {
        vd.y = max(vd.y, 0.0);
    }

    textureStore(dst_vd, c, vd);
    textureStore(dst_temp, c, vec4<f32>(temp, 0.0, 0.0, 0.0));
}
