// Density-only MacCormack advection with a synthetic velocity field.
//
// This is NOT a fluid simulation — there's no projection, no pressure solve,
// no BiMocq. The velocity field is
//
//     vel = drift + curl_coarse + curl_fine + injected_vel
//
// where the two curl terms are analytic curl of scalar-noise potentials at
// different spatial/temporal frequencies — coarse large-scale swirl plus
// finer in-plume turbulence.
//
// Advection is a MacCormack correction on top of semi-Lagrangian: backtrace
// from the cell center, sample, then forward-trace that sample back and
// compare against the current density to estimate numerical diffusion error.
// The corrected value is clamped by the 2x2x2 neighborhood around the
// backtrace point so the correction can't overshoot into new minima/maxima
// (monotonic limiter — the standard fix for MacCormack's overshoot issue).
// Net effect: plumes keep sharper edges and finer wisps survive longer than
// under plain semi-Lagrangian.
//
// The `injected_vel` term comes from the xyz channels of the density
// texture — the inject shader stashes per-puff velocity there so tile drags
// and scripted wind gusts still nudge smoke.

struct FluidUniforms {
    grid_size:    vec4<f32>, // xyz = grid dims, w = sim_time (seconds, wrapped)
    grid_min:     vec4<f32>,
    grid_max:     vec4<f32>,
    inv_extent:   vec4<f32>,
    // x = dt, y = density_dissipation, z = drift_speed (world units/sec +Z),
    // w = curl_strength (world units/sec amplitude of the curl term).
    params:       vec4<f32>,
    // x = curl_spatial_scale, y = curl_time_scale, z = stored_vel_mix, w = unused.
    force_params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var src_density: texture_3d<f32>;
@group(0) @binding(2) var src_samp: sampler;
@group(0) @binding(3) var dst_density: texture_storage_3d<rgba16float, write>;

fn hash31(p: vec3<f32>) -> f32 {
    let h = dot(p, vec3<f32>(127.1, 311.7, 74.7));
    return fract(sin(h) * 43758.5453123);
}

fn vnoise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash31(i + vec3<f32>(0.0, 0.0, 0.0));
    let b = hash31(i + vec3<f32>(1.0, 0.0, 0.0));
    let c = hash31(i + vec3<f32>(0.0, 1.0, 0.0));
    let d = hash31(i + vec3<f32>(1.0, 1.0, 0.0));
    let e = hash31(i + vec3<f32>(0.0, 0.0, 1.0));
    let ff = hash31(i + vec3<f32>(1.0, 0.0, 1.0));
    let g = hash31(i + vec3<f32>(0.0, 1.0, 1.0));
    let h = hash31(i + vec3<f32>(1.0, 1.0, 1.0));
    let x0 = mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
    let x1 = mix(mix(e, ff, u.x), mix(g, h, u.x), u.y);
    return mix(x0, x1, u.z);
}

// Analytic 3D curl of three independent scalar-noise potentials. The curl of
// any gradient is divergence-free, so this gives a swirling, mass-preserving
// velocity field that looks organic without any simulation cost.
fn curl_noise(p: vec3<f32>) -> vec3<f32> {
    let eps = 0.6;
    let dx = vec3<f32>(eps, 0.0, 0.0);
    let dy = vec3<f32>(0.0, eps, 0.0);
    let dz = vec3<f32>(0.0, 0.0, eps);
    // Offset the three potentials so they're decorrelated.
    let p1 = p;
    let p2 = p + vec3<f32>(31.4, 0.0, 0.0);
    let p3 = p + vec3<f32>(0.0, 71.7, 0.0);
    let a_dy = vnoise3(p1 + dy) - vnoise3(p1 - dy);
    let a_dz = vnoise3(p1 + dz) - vnoise3(p1 - dz);
    let b_dx = vnoise3(p2 + dx) - vnoise3(p2 - dx);
    let b_dz = vnoise3(p2 + dz) - vnoise3(p2 - dz);
    let c_dx = vnoise3(p3 + dx) - vnoise3(p3 - dx);
    let c_dy = vnoise3(p3 + dy) - vnoise3(p3 - dy);
    let inv_2eps = 1.0 / (2.0 * eps);
    return vec3<f32>(
        (c_dy - b_dz) * inv_2eps,
        (a_dz - c_dx) * inv_2eps,
        (b_dx - a_dy) * inv_2eps,
    );
}

fn cell_to_world(c: vec3<f32>) -> vec3<f32> {
    let uvw = (c + vec3<f32>(0.5)) / fluid.grid_size.xyz;
    return mix(fluid.grid_min.xyz, fluid.grid_max.xyz, uvw);
}

// The 3D density textures are allocated at the max grid size (128×128×80)
// but only the first `grid_size` cells are live — the rest are untouched
// zeros. Plain `textureSampleLevel` with uvw in [0,1] interprets that range
// over the *physical* texture, so any uvw beyond `active/full` samples dead
// space and reads zero. Scale logical uvw [0,1] (where 0..1 spans the
// `active` cells) into the physical [0, active/full] range so the sampler
// lands on the correct physical cell — logical cell i's center maps to
// physical cell i's center.
fn active_sample_uvw(uvw: vec3<f32>) -> vec3<f32> {
    let dims = textureDimensions(src_density);
    let full_dims = vec3<f32>(f32(dims.x), f32(dims.y), f32(dims.z));
    let active_dims = fluid.grid_size.xyz;
    return uvw * active_dims / full_dims;
}

// Synthetic velocity sampled at a world position. Used at both the cell
// center (forward) and the backtrace point (for MacCormack's back-advect).
// The injected-velocity contribution comes from the nearest cell's xyz on
// the source texture — good enough for the MacCormack correction; exact
// bilinear isn't worth the extra sampleLevel.
fn synthetic_vel(world: vec3<f32>) -> vec3<f32> {
    let extent_z = max(fluid.grid_max.z - fluid.grid_min.z, 1e-3);
    let height_frac = clamp((world.z - fluid.grid_min.z) / extent_z, 0.0, 1.0);
    let ceiling_fade = 1.0 - smoothstep(0.75, 1.0, height_frac);
    let drift = vec3<f32>(0.0, 0.0, fluid.params.z * ceiling_fade);

    // Coarse octave — large-scale swirls roughly the width of the grid.
    let coarse_space = fluid.force_params.x;
    let coarse_time = fluid.force_params.y;
    let pc = world * coarse_space
           + vec3<f32>(fluid.grid_size.w * coarse_time,
                       fluid.grid_size.w * coarse_time * 0.7,
                       fluid.grid_size.w * coarse_time * -0.4);
    let curl_coarse = curl_noise(pc) * fluid.params.w;

    // Fine octave — ~4× spatial frequency, 2× time frequency, 55% amplitude.
    // Gives in-plume wisp detail without costing another shader pass.
    let fine_space = coarse_space * 4.3;
    let fine_time = coarse_time * 2.0;
    let pf = world * fine_space
           + vec3<f32>(fluid.grid_size.w * fine_time * 1.3,
                       fluid.grid_size.w * fine_time * -0.9,
                       fluid.grid_size.w * fine_time * 0.6)
           + vec3<f32>(17.3, -11.1, 5.7); // de-phase from the coarse octave
    let curl_fine = curl_noise(pf) * (fluid.params.w * 0.55);

    // Third octave — ~12× spatial, 3.5× time, 22% amplitude. Adds the
    // hair-thin wisp detail that reads as real turbulence rather than
    // large-blob swirl.
    let ultra_space = coarse_space * 11.7;
    let ultra_time = coarse_time * 3.5;
    let pu = world * ultra_space
           + vec3<f32>(fluid.grid_size.w * ultra_time * -1.1,
                       fluid.grid_size.w * ultra_time * 0.8,
                       fluid.grid_size.w * ultra_time * 1.4)
           + vec3<f32>(-29.1, 53.2, -7.8);
    let curl_ultra = curl_noise(pu) * (fluid.params.w * 0.22);

    return drift + curl_coarse + curl_fine + curl_ultra;
}

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec3<u32>(u32(fluid.grid_size.x), u32(fluid.grid_size.y), u32(fluid.grid_size.z));
    if (gid.x >= dims.x || gid.y >= dims.y || gid.z >= dims.z) {
        return;
    }
    let coord = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));
    let world = cell_to_world(vec3<f32>(f32(gid.x), f32(gid.y), f32(gid.z)));
    let dt = fluid.params.x;

    // Velocity at this cell center. Add injected-velocity stashed in xyz
    // from the previous inject pass so tile drags and gusts still push.
    let here = textureLoad(src_density, coord, 0);
    let injected_vel = here.xyz * fluid.force_params.z;
    let vel = synthetic_vel(world) + injected_vel;

    // ── Step 1: standard semi-Lagrangian backtrace ─────────────────────
    let back = world - vel * dt;
    var uvw_back = (back - fluid.grid_min.xyz) * fluid.inv_extent.xyz;
    uvw_back = clamp(uvw_back, vec3<f32>(0.0), vec3<f32>(1.0));
    let phi_hat = textureSampleLevel(src_density, src_samp, active_sample_uvw(uvw_back), 0.0);

    // ── Step 2: forward-trace from the backtrace sample point ──────────
    // If the scheme were lossless, forward-tracing phi_hat by +vel*dt
    // should reproduce the current cell's density. The difference is the
    // numerical-diffusion error; half of it gets added back as the
    // MacCormack correction.
    let vel_back = synthetic_vel(back); // velocity at the backtrace point
    let fwd = back + vel_back * dt;
    var uvw_fwd = (fwd - fluid.grid_min.xyz) * fluid.inv_extent.xyz;
    uvw_fwd = clamp(uvw_fwd, vec3<f32>(0.0), vec3<f32>(1.0));
    let phi_fwd = textureSampleLevel(src_density, src_samp, active_sample_uvw(uvw_fwd), 0.0);

    // phi_current: density *at this cell* on the source frame.
    let phi_current = here;
    let correction = 0.5 * (phi_current - phi_fwd);
    var phi_corrected = phi_hat + correction;

    // ── Step 3: monotonic limiter ──────────────────────────────────────
    // Clamp the corrected density to the 2×2×2 neighborhood around the
    // backtrace point, so the correction can't manufacture densities
    // outside the range that existed last frame near that sample point.
    // Without this, MacCormack creates ringing artifacts and neg/over-peak
    // spikes.
    let back_cell = uvw_back * fluid.grid_size.xyz - vec3<f32>(0.5);
    let c0 = vec3<i32>(
        clamp(i32(floor(back_cell.x)), 0, i32(dims.x) - 1),
        clamp(i32(floor(back_cell.y)), 0, i32(dims.y) - 1),
        clamp(i32(floor(back_cell.z)), 0, i32(dims.z) - 1),
    );
    let c1 = vec3<i32>(
        min(c0.x + 1, i32(dims.x) - 1),
        min(c0.y + 1, i32(dims.y) - 1),
        min(c0.z + 1, i32(dims.z) - 1),
    );
    let n000 = textureLoad(src_density, vec3<i32>(c0.x, c0.y, c0.z), 0);
    let n100 = textureLoad(src_density, vec3<i32>(c1.x, c0.y, c0.z), 0);
    let n010 = textureLoad(src_density, vec3<i32>(c0.x, c1.y, c0.z), 0);
    let n110 = textureLoad(src_density, vec3<i32>(c1.x, c1.y, c0.z), 0);
    let n001 = textureLoad(src_density, vec3<i32>(c0.x, c0.y, c1.z), 0);
    let n101 = textureLoad(src_density, vec3<i32>(c1.x, c0.y, c1.z), 0);
    let n011 = textureLoad(src_density, vec3<i32>(c0.x, c1.y, c1.z), 0);
    let n111 = textureLoad(src_density, vec3<i32>(c1.x, c1.y, c1.z), 0);
    let lo = min(min(min(n000, n100), min(n010, n110)),
                 min(min(n001, n101), min(n011, n111)));
    let hi = max(max(max(n000, n100), max(n010, n110)),
                 max(max(n001, n101), max(n011, n111)));
    phi_corrected = clamp(phi_corrected, lo, hi);

    let new_density = max(phi_corrected.w * fluid.params.y, 0.0);
    // Keep the injected velocity around but decay it so the "push" fades.
    // Without decay, a tile drag would keep propelling smoke forever.
    let new_vel = phi_corrected.xyz * 0.88;

    textureStore(dst_density, coord, vec4<f32>(new_vel, new_density));
}
