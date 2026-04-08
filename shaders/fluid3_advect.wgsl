// 3D semi-Lagrangian advection with linear filtered sampling.
// Also applies buoyancy (+Y force scaled by density) and dissipation.
//
// On top of the textbook semi-Lagrangian step we model three effects that
// turn the candle plumes from rocket exhaust into something that reads as
// actual smoke:
//
//   1. Cooling — buoyancy is attenuated as smoke rises. Rising smoke
//      mixes with cold air and loses temperature, so its lift drops
//      with altitude. We use the cell's height fraction in the grid
//      as a cheap "remaining heat" proxy.
//   2. Altitude drag — extra vertical velocity dissipation that grows
//      with height. Convection is fastest near the wick and slowest
//      far above it; this gives the column its characteristic taper.
//   3. Lateral curl — a deterministic, position-keyed swirl in the
//      XZ plane proportional to local density and altitude. The
//      pressure projection step turns this into divergence-free
//      rolling motion, which is what makes the smoke spread laterally
//      and form mushroom-style billows instead of a tight pillar.

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

// Cheap 2D value-noise hash → [0,1).
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

    // Height fraction in [0,1]: 0 at the table plane, 1 at the top of
    // the smoke grid. Drives all three "real smoke" effects below.
    let extent_y = max(fluid.grid_max.y - fluid.grid_min.y, 1e-3);
    let height_frac = clamp((world_pos.y - fluid.grid_min.y) / extent_y, 0.0, 1.0);

    // Normalized "smoke amount" in roughly [0,1]. The injection step
    // can drive raw density to ~12 at the source cell (0.1/frame
    // splat against a 0.992 dissipation halflife), so we use this
    // saturating value for the *curl* below — the curl was tuned
    // against density~1 and a 12× over-strength swirl is what was
    // collapsing the column into a ball at the wick.
    //
    // Buoyancy keeps using the *raw* density: buoyancy is physically
    // linear in mass and the pressure projection handles a smooth
    // dense column without any trouble. Clamping it would just rob
    // the column of the lift it needs to actually leave the wick.
    let smoke = tanh(new_density);

    // 1. Cooling: buoyancy fades from full strength at the wick to
    //    near-zero at the top of the grid. Stretched well past the
    //    wick so the smoke gets the full grid height to rise through.
    let heat = 1.0 - smoothstep(0.15, 1.05, height_frac);
    new_vel.y = new_vel.y + buoyancy * new_density * dt * heat;

    // 2. Altitude drag on the vertical component only — keeps lateral
    //    motion intact (so the spreading from #3 isn't immediately
    //    sucked back together) while bleeding off upward momentum as
    //    the smoke climbs. Very mild so the column actually reaches
    //    the upper grid.
    new_vel.y = new_vel.y * mix(1.0, 0.985, height_frac);

    // 3. Lateral curl. Build a stream-function-like potential from
    //    two octaves of value noise sampled in the XZ plane, then
    //    take its 2D curl (∂ψ/∂z, -∂ψ/∂x) to get a divergence-free
    //    horizontal velocity field. Because the noise is purely
    //    position-keyed there is no time term required, but each
    //    smoke parcel still sees a different swirl as it rises
    //    because the field varies with Y too.
    //
    //    The strength is gated by `new_density * height_frac` so:
    //      - empty cells aren't perturbed (no wasted projection work)
    //      - the wick base stays a clean point source
    //      - the spreading kicks in higher up where real plumes break
    let nscale = 0.012;
    let np = vec2<f32>(world_pos.x, world_pos.z) * nscale
           + vec2<f32>(world_pos.y * nscale * 0.4, 0.0);
    let eps = 4.0;
    let psi_xp = vnoise2(np + vec2<f32>(eps * nscale, 0.0));
    let psi_xm = vnoise2(np - vec2<f32>(eps * nscale, 0.0));
    let psi_zp = vnoise2(np + vec2<f32>(0.0, eps * nscale));
    let psi_zm = vnoise2(np - vec2<f32>(0.0, eps * nscale));
    let curl_x =  (psi_zp - psi_zm) / (2.0 * eps);
    let curl_z = -(psi_xp - psi_xm) / (2.0 * eps);
    // Curl strength uses the *normalized* `smoke` value, not raw
    // density. With raw density saturating around 12 at the source,
    // the previous `density * 380` produced a swirl ~12× the
    // intended magnitude — strong enough that lateral velocity
    // matched vertical velocity at the wick, smearing the column
    // into a small ball. Tanh-clamped `smoke` keeps the swirl in
    // the regime the constant 380 was originally tuned for, and we
    // also push the activation a little higher up so the wick base
    // stays a clean point source.
    let swirl_strength = smoke * smoothstep(0.10, 0.55, height_frac) * 380.0;
    new_vel.x = new_vel.x + curl_x * swirl_strength * dt;
    new_vel.z = new_vel.z + curl_z * swirl_strength * dt;

    textureStore(dst_vd, coord, vec4<f32>(new_vel, new_density));
}
