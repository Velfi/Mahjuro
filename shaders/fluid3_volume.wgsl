// Ray-marched volumetric smoke renderer.
//
// Fullscreen triangle. Each pixel reconstructs a world-space ray from the
// inverse view-projection, slab-clips against the smoke AABB, depth-clips
// against the scene depth buffer, and integrates density front-to-back with
// per-step lighting from the candle point lights.

const MAX_LIGHTS: u32 = 8u;

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
};

struct VolumeCamera {
    inv_view_proj: mat4x4<f32>,
    view_proj:     mat4x4<f32>,
    cam_pos:       vec4<f32>,   // xyz = world camera origin
    grid_min:      vec4<f32>,
    grid_max:      vec4<f32>,
    params:        vec4<f32>,   // x=max_alpha, y=step_count, z=light_strength, w=ambient
};

struct PointLight {
    pos:   vec4<f32>,   // xyz position, w radius
    color: vec4<f32>,   // rgb color, a intensity
};
struct PointLights {
    count: vec4<u32>,
    // extras.x = display gamma exponent; rest reserved.
    extras: vec4<f32>,
    lights: array<PointLight, 16>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> cam: VolumeCamera;
@group(1) @binding(1) var density_tex: texture_3d<f32>;
@group(1) @binding(2) var density_samp: sampler;
@group(1) @binding(3) var depth_tex: texture_depth_2d;
@group(1) @binding(4) var<uniform> lights: PointLights;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) ndc:  vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    // Fullscreen triangle.
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 3.0,  1.0),
    );
    let p = pos[vid];
    var out: VsOut;
    out.clip_pos = vec4<f32>(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}

// Slab intersection of a ray (origin, dir) with the AABB [grid_min, grid_max].
// Returns (t_near, t_far). t_far < t_near means no hit.
fn ray_aabb(origin: vec3<f32>, dir: vec3<f32>) -> vec2<f32> {
    let inv_dir = vec3<f32>(
        select(1.0 / dir.x, 1e30, abs(dir.x) < 1e-8),
        select(1.0 / dir.y, 1e30, abs(dir.y) < 1e-8),
        select(1.0 / dir.z, 1e30, abs(dir.z) < 1e-8),
    );
    let t0 = (cam.grid_min.xyz - origin) * inv_dir;
    let t1 = (cam.grid_max.xyz - origin) * inv_dir;
    let tmin = min(t0, t1);
    let tmax = max(t0, t1);
    let t_near = max(max(tmin.x, tmin.y), tmin.z);
    let t_far  = min(min(tmax.x, tmax.y), tmax.z);
    return vec2<f32>(t_near, t_far);
}

// Reconstruct world position from ndc + non-linear depth.
fn world_from_ndc(ndc: vec2<f32>, depth: f32) -> vec3<f32> {
    let clip = vec4<f32>(ndc, depth, 1.0);
    let world = cam.inv_view_proj * clip;
    return world.xyz / world.w;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let ndc = in.ndc;
    // Reconstruct ray.
    let near_world = world_from_ndc(ndc, 0.0);
    let far_world  = world_from_ndc(ndc, 1.0);
    let origin = cam.cam_pos.xyz;
    let dir = normalize(far_world - origin);

    let hit = ray_aabb(origin, dir);
    let t_near = max(hit.x, 0.0);
    var t_far = hit.y;
    if (t_far <= t_near) {
        return vec4<f32>(0.0);
    }

    // Sample scene depth and convert to a world-space distance along this ray.
    let pix = vec2<i32>(
        i32((ndc.x * 0.5 + 0.5) * globals.screen.x),
        i32((1.0 - (ndc.y * 0.5 + 0.5)) * globals.screen.y),
    );
    let dims = textureDimensions(depth_tex);
    let clamped = clamp(pix, vec2<i32>(0, 0), vec2<i32>(i32(dims.x) - 1, i32(dims.y) - 1));
    let depth_sample = textureLoad(depth_tex, clamped, 0);
    if (depth_sample < 0.999999) {
        let scene_world = world_from_ndc(ndc, depth_sample);
        let scene_t = length(scene_world - origin);
        t_far = min(t_far, scene_t);
    }
    if (t_far <= t_near) {
        return vec4<f32>(0.0);
    }

    let max_alpha = cam.params.x;
    let nsteps = max(i32(cam.params.y), 4);
    let light_strength = cam.params.z;
    let ambient = cam.params.w;

    let span = t_far - t_near;
    let step = span / f32(nsteps);

    // Jitter start to break up banding.
    let jitter = fract(sin(dot(in.clip_pos.xy, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    var t = t_near + step * jitter;

    var color = vec3<f32>(0.0);
    var transmittance = 1.0;

    let inv_extent = 1.0 / (cam.grid_max.xyz - cam.grid_min.xyz);
    let lcount = lights.count.x;

    for (var i: i32 = 0; i < nsteps; i = i + 1) {
        if (transmittance < 0.01) { break; }
        let pos = origin + dir * t;
        let uvw = (pos - cam.grid_min.xyz) * inv_extent;
        if (any(uvw < vec3<f32>(0.0)) || any(uvw > vec3<f32>(1.0))) {
            t = t + step;
            continue;
        }
        let sample = textureSampleLevel(density_tex, density_samp, uvw, 0.0);
        let density = max(sample.w, 0.0);
        if (density > 0.001) {
            // Per-step absorption.
            let absorb = 1.0 - exp(-density * step * 0.01);

            // Lighting: ambient + sum of point-light contributions.
            //
            // Two important deviations from a naïve `(1 - dist/radius)²`
            // attenuation, both required to stop the smoke columns above
            // each candle from blowing out into bright shafts:
            //
            //   1. Minimum distance clamp. The smoke plume from a wick
            //      sits *right next to* its own light source. Without a
            //      floor on the effective distance, every voxel in the
            //      column near the candle sees `falloff ≈ 1` and gets
            //      the full unattenuated `light_strength` dumped into
            //      it — visually, a vertical stripe of pure white above
            //      every wick. Clamping the effective distance to
            //      ~25% of the light radius gives the column a sane
            //      maximum brightness.
            //
            //   2. Smoke albedo. Real smoke scatters only a fraction
            //      of incoming light forward; treating it as ~white
            //      `(0.85, 0.82, 0.78)` is what makes lit columns
            //      read as glowing rather than illuminated. The
            //      darker grey here (with a faint warm bias from the
            //      candle palette) keeps even brightly-lit cells in
            //      mid-grey territory.
            var lit = vec3<f32>(ambient);
            for (var li: u32 = 0u; li < lcount; li = li + 1u) {
                let l = lights.lights[li];
                let to_light = l.pos.xyz - pos;
                let dist = sqrt(max(dot(to_light, to_light), 1.0));
                let radius = max(l.pos.w, 1.0);
                // Floor the effective distance at a fraction of the
                // light radius. This is what prevents the in-column
                // brightness spike — voxels closer than `min_dist`
                // all share the same falloff value rather than
                // sweeping up to 1.0.
                let min_dist = radius * 0.28;
                let eff_dist = max(dist, min_dist);
                let falloff = clamp(1.0 - eff_dist / radius, 0.0, 1.0);
                lit = lit + l.color.rgb * l.color.a * falloff * falloff * light_strength;
            }
            // Soft-knee the accumulated lighting so a voxel sitting
            // inside several overlapping candle radii can't push past
            // the smoke's own albedo. Reinhard preserves the warm
            // chroma instead of clipping a channel and going pale.
            lit = lit / (vec3<f32>(1.0) + lit * 0.6);

            // Darker grey base — real smoke is closer to charcoal
            // than printer paper. The previous (0.85,0.82,0.78) value
            // turned every lit voxel into a near-white streak.
            let smoke_color = vec3<f32>(0.42, 0.40, 0.37) * lit;
            color = color + transmittance * smoke_color * absorb;
            transmittance = transmittance * (1.0 - absorb);
        }
        t = t + step;
    }

    let alpha = (1.0 - transmittance) * max_alpha;
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    let rgb = pow(color * max_alpha, vec3<f32>(inv_g));
    return vec4<f32>(rgb, alpha);
}
