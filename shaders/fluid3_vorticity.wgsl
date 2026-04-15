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

fn hash31(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(127.1, 311.7, 74.7))) * 43758.5453);
}

fn vnoise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let n000 = hash31(i + vec3<f32>(0.0, 0.0, 0.0));
    let n100 = hash31(i + vec3<f32>(1.0, 0.0, 0.0));
    let n010 = hash31(i + vec3<f32>(0.0, 1.0, 0.0));
    let n110 = hash31(i + vec3<f32>(1.0, 1.0, 0.0));
    let n001 = hash31(i + vec3<f32>(0.0, 0.0, 1.0));
    let n101 = hash31(i + vec3<f32>(1.0, 0.0, 1.0));
    let n011 = hash31(i + vec3<f32>(0.0, 1.0, 1.0));
    let n111 = hash31(i + vec3<f32>(1.0, 1.0, 1.0));
    let a = mix(n000, n100, u.x);
    let b = mix(n010, n110, u.x);
    let c = mix(n001, n101, u.x);
    let d = mix(n011, n111, u.x);
    let e = mix(a, b, u.y);
    let g = mix(c, d, u.y);
    return mix(e, g, u.z);
}

// Divergence-free curl-noise: take finite-difference curl of a vector
// potential made from three offset 3D value-noise scalar fields. Result is
// analytically div-free (up to the finite-difference stencil) so it injects
// rotational motion without adding pressure work for the solver to undo.
fn curl_noise3(p: vec3<f32>) -> vec3<f32> {
    let eps = 1.0;
    let o1 = vec3<f32>(0.0, 0.0, 0.0);
    let o2 = vec3<f32>(57.3, 113.7, 29.1);
    let o3 = vec3<f32>(-41.9, 17.5, 83.2);

    let p1yp = vnoise3(p + vec3<f32>(0.0, eps, 0.0) + o1);
    let p1ym = vnoise3(p - vec3<f32>(0.0, eps, 0.0) + o1);
    let p1zp = vnoise3(p + vec3<f32>(0.0, 0.0, eps) + o1);
    let p1zm = vnoise3(p - vec3<f32>(0.0, 0.0, eps) + o1);

    let p2xp = vnoise3(p + vec3<f32>(eps, 0.0, 0.0) + o2);
    let p2xm = vnoise3(p - vec3<f32>(eps, 0.0, 0.0) + o2);
    let p2zp = vnoise3(p + vec3<f32>(0.0, 0.0, eps) + o2);
    let p2zm = vnoise3(p - vec3<f32>(0.0, 0.0, eps) + o2);

    let p3xp = vnoise3(p + vec3<f32>(eps, 0.0, 0.0) + o3);
    let p3xm = vnoise3(p - vec3<f32>(eps, 0.0, 0.0) + o3);
    let p3yp = vnoise3(p + vec3<f32>(0.0, eps, 0.0) + o3);
    let p3ym = vnoise3(p - vec3<f32>(0.0, eps, 0.0) + o3);

    let inv2e = 1.0 / (2.0 * eps);
    return vec3<f32>(
        ((p3yp - p3ym) - (p2zp - p2zm)) * inv2e,
        ((p1zp - p1zm) - (p3xp - p3xm)) * inv2e,
        ((p2xp - p2xm) - (p1yp - p1ym)) * inv2e,
    );
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
    let extent_z = max(fluid.grid_max.z - fluid.grid_min.z, 1e-3);
    let height_frac = clamp((world_pos.z - fluid.grid_min.z) / extent_z, 0.0, 1.0);

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

    // Vorticity confinement preserves existing curl, but a symmetric hot
    // plume can still remain too laminar on a coarse grid. Seed a small,
    // analytically divergence-free 3D curl-noise field that also advances in
    // time so the plume has something physical-looking to roll into — the
    // previous 2D quasi-static stream function forced the same spatial
    // pattern every frame, which BiMocq faithfully preserved into visible
    // striping. True 3D curl-noise gives wispy, non-repeating shape.
    let nscale = 0.013;
    let t = fluid.grid_size.w;
    let np = vec3<f32>(world_pos.x, world_pos.y, world_pos.z) * nscale
           + vec3<f32>(t * 0.23, t * -0.17, t * 0.11);
    let noise_curl = curl_noise3(np);
    let hot_noise_strength = fluid.force_params.y * 145.0 * thermal * height_gate;
    let loft_noise_strength = fluid.force_params.y * 48.0 * smoke * loft_gate;
    new_vel = new_vel + noise_curl * (hot_noise_strength + loft_noise_strength) * fluid.params.x;

    // Very weak high-altitude drift so the tops of the plumes separate and
    // stop looking phase-locked.
    let drift_angle = world_pos.z * 0.018 + world_pos.x * 0.006 + world_pos.y * 0.004;
    let drift = vec3<f32>(sin(drift_angle), cos(drift_angle), 0.0) * loft_gate * smoke * 6.0;
    new_vel = new_vel + drift * fluid.params.x;

    vd = vec4<f32>(new_vel, vd.w);

    // Preserve the solid table floor behavior after confinement too.
    if (c.z == 0) {
        vd.z = max(vd.z, 0.0);
    }

    textureStore(dst_vd, c, vd);
    textureStore(dst_temp, c, vec4<f32>(temp, 0.0, 0.0, 0.0));
}
