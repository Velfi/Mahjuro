// Ray-marched volumetric smoke renderer.
//
// Fullscreen triangle. Each pixel reconstructs a world-space ray from the
// inverse view-projection, slab-clips against the smoke AABB, depth-clips
// against the scene depth buffer, and integrates density front-to-back.
//
// Lighting is **pre-baked** by `fluid3_lightbake.wgsl` into the
// `lit_density_tex` 3D texture: `(rgb = pre-lit smoke colour, a = density)`.
// The bake runs once per frame at voxel rate (~3.5M evals) instead of
// per-fragment-per-step (~450M evals), and the resulting bilinear-filtered
// colour matches the per-step lighting almost exactly because the candle
// radii are much larger than the voxel size.

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
    params:        vec4<f32>,   // x=max_alpha, y=step_count (z/w consumed by lightbake)
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> cam: VolumeCamera;
@group(1) @binding(1) var lit_density_tex: texture_3d<f32>;
@group(1) @binding(2) var density_samp: sampler;
@group(1) @binding(3) var depth_tex: texture_depth_2d;

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

    let span = t_far - t_near;
    let step = span / f32(nsteps);

    // Jitter start to break up banding.
    let jitter = fract(sin(dot(in.clip_pos.xy, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    var t = t_near + step * jitter;

    var color = vec3<f32>(0.0);
    var transmittance = 1.0;

    let inv_extent = 1.0 / (cam.grid_max.xyz - cam.grid_min.xyz);

    for (var i: i32 = 0; i < nsteps; i = i + 1) {
        if (transmittance < 0.01) { break; }
        let pos = origin + dir * t;
        let uvw = (pos - cam.grid_min.xyz) * inv_extent;
        if (any(uvw < vec3<f32>(0.0)) || any(uvw > vec3<f32>(1.0))) {
            t = t + step;
            continue;
        }
        // Pre-lit sample: rgb = lit smoke colour (already includes
        // ambient + per-light contributions, Reinhard tone-map, and
        // smoke albedo), a = density. Bilinear-filtered across the
        // 3D grid by `density_samp`.
        let sample = textureSampleLevel(lit_density_tex, density_samp, uvw, 0.0);
        let density = max(sample.a, 0.0);
        if (density > 0.001) {
            let absorb = 1.0 - exp(-density * step * 0.01);
            color = color + transmittance * sample.rgb * absorb;
            transmittance = transmittance * (1.0 - absorb);
        }
        t = t + step;
    }

    let alpha = (1.0 - transmittance) * max_alpha;
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    let rgb = pow(color * max_alpha, vec3<f32>(inv_g));
    return vec4<f32>(rgb, alpha);
}
