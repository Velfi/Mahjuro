// Ray-marched volumetric smoke renderer + SDF candle flames.
//
// Fullscreen triangle. Each pixel reconstructs a world-space ray from the
// inverse view-projection, slab-clips against the smoke AABB, depth-clips
// against the scene depth buffer, and integrates density front-to-back.
//
// Lighting is **pre-baked** by `fluid3_lightbake.wgsl` into the
// `lit_density_tex` 3D texture: `(rgb = pre-lit smoke colour, a = density)`.
//
// Candle flames are rendered in a second pass after the smoke march: each
// candle gets a dedicated ray-sphere intersection + fine sub-march (~20
// steps through the flame envelope), giving pixel-resolution teardrop
// shapes that properly interleave with the depth buffer.

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
    grid_size:     vec4<f32>,
    params:        vec4<f32>,   // x=max_alpha, y=step_count (z/w consumed by lightbake)
    mode:          vec4<f32>,   // x=0 both, 1 smoke only, 2 flames only
};

const MAX_LIGHTS: u32 = 16u;

struct PointLight {
    pos:   vec4<f32>,   // xyz position, w radius
    color: vec4<f32>,   // rgb color, a intensity
};
struct PointLights {
    count:  vec4<u32>,  // x=total, y=candle_count
    extras: vec4<f32>,  // x=gamma, y=time, z=flame_height_world
    lights: array<PointLight, 16>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> cam: VolumeCamera;
@group(1) @binding(1) var lit_density_tex: texture_3d<f32>;
@group(1) @binding(2) var density_samp: sampler;
@group(1) @binding(3) var depth_tex: texture_depth_2d;
@group(1) @binding(4) var<uniform> lights: PointLights;
@group(1) @binding(5) var velocity_tex: texture_3d<f32>;

// Convert a world position to a voxel texel coordinate for textureLoad.
fn world_to_texel(world: vec3<f32>, grid_min: vec3<f32>, grid_max: vec3<f32>) -> vec3<i32> {
    let grid_size = cam.grid_size.xyz;
    let uvw = (world - grid_min) / (grid_max - grid_min);
    let cell = clamp(uvw * grid_size, vec3<f32>(0.0), grid_size - vec3<f32>(1.0));
    return vec3<i32>(i32(cell.x), i32(cell.y), i32(cell.z));
}

fn active_sample_uvw(uvw: vec3<f32>) -> vec3<f32> {
    let dims = textureDimensions(lit_density_tex);
    let full_dims = vec3<f32>(f32(dims.x), f32(dims.y), f32(dims.z));
    let active_dims = cam.grid_size.xyz;
    let min_uvw = 0.5 / full_dims;
    let max_uvw = (active_dims - 0.5) / full_dims;
    return mix(min_uvw, max_uvw, uvw);
}

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) ndc:  vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
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

// Slab intersection of a ray with the AABB [grid_min, grid_max].
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

fn world_from_ndc(ndc: vec2<f32>, depth: f32) -> vec3<f32> {
    let clip = vec4<f32>(ndc, depth, 1.0);
    let world = cam.inv_view_proj * clip;
    return world.xyz / world.w;
}

// ── Noise helpers ────────────────────────────────────────────────────────
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

fn fbm3(p: vec3<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var pp = p;
    for (var i = 0; i < 3; i = i + 1) {
        v = v + amp * vnoise3(pp);
        pp = pp * 2.03;
        amp = amp * 0.5;
    }
    return v;
}

// ── SDF flame evaluation ─────────────────────────────────────────────────
// Physically-proportioned candle flame: a real votive flame is roughly
// 30 mm tall × 8-10 mm at its widest, with the widest point about 30%
// from the base. The luminous zone is a teardrop with a narrow neck at
// the wick, a dark inner cone (unburnt wax vapour), and a sharp tip.
//
// `wind` is the fluid velocity sampled at the wick tip (world units/s).
// The flame leans in the wind direction: the base stays fixed at the
// wick while the tip bends proportionally to height².
//
// Returns (emission_rgb, density) for compositing.
fn eval_flame(pos: vec3<f32>, wick: vec3<f32>, flame_h: f32, time: f32, idx: f32, wind: vec3<f32>) -> vec4<f32> {
    let to_raw = pos - wick;

    // ── Wind bend ───────────────────────────────────────────────────
    // The flame base is anchored at the wick. Higher portions lean
    // in the wind direction, with a quadratic falloff (the tip bends
    // most). We work in wick-local space and shift the evaluation
    // point *against* the wind so the envelope effectively leans.
    let norm_z_raw = clamp(to_raw.z / flame_h, 0.0, 1.5);
    let bend_strength = 0.12;
    let bend = vec2<f32>(wind.x, wind.y) * norm_z_raw * norm_z_raw * bend_strength;
    let to = vec3<f32>(to_raw.x - bend.x, to_raw.y - bend.y, to_raw.z);

    let height = to.z;
    let radial = length(vec2<f32>(to.x, to.y));

    if (height < -flame_h * 0.05 || height > flame_h) {
        return vec4<f32>(0.0);
    }
    let norm_h = clamp(height / flame_h, 0.0, 1.0);

    // Wind magnitude drives extra flicker and envelope smearing.
    let wind_mag = clamp(length(vec2<f32>(wind.x, wind.y)) * 0.02, 0.0, 1.5);

    // ── Teardrop profile ────────────────────────────────────────────
    // Widest at ~30% up from the base (real votive flame proportions).
    // Uses a skewed bulge: `sin(π·h^0.7)` peaks earlier than `sin(π·h)`.
    // Max half-width ≈ flame_h × 0.16  (≈5 mm on a 30 mm flame).
    let skewed_h = pow(norm_h, 0.7);
    let bulge = sin(skewed_h * 3.14159);
    let max_r = flame_h * 0.16;
    // Narrow waist at the base (wick cup) and sharp taper at the tip.
    let base_pinch = smoothstep(0.0, 0.12, norm_h);
    // Wind stretches the tip — strong gusts make the flame trail.
    let tip_taper  = 1.0 - norm_h * norm_h * (0.45 - wind_mag * 0.15);
    let envelope = max_r * bulge * base_pinch * tip_taper;

    // ── Noise displacement ──────────────────────────────────────────
    // Advected upward over time so wisps "lick" off the tip.
    // Wind shears the noise field laterally, making the flame streak
    // in the wind direction rather than just translating.
    let phase = idx * 6.2831853;
    let n_coord = vec3<f32>(
        to.x * 0.12 + sin(time * 1.7 + phase) * 0.15 - wind.x * norm_h * 0.04,
        to.y * 0.12 + cos(time * 2.1 + phase) * 0.15 - wind.y * norm_h * 0.04,
        norm_h * 3.0 - time * 3.0 + phase,
    );
    let noise = fbm3(n_coord) - 0.5;
    // Noise amplitude grows with height (base is stable, tip dances).
    // Wind pumps up the wobble amplitude.
    let wobble_amp = (0.4 + wind_mag * 0.3) * norm_h;
    let disp = envelope + noise * max_r * wobble_amp;

    if (radial >= disp || disp <= 0.0) {
        return vec4<f32>(0.0);
    }

    // Flame intensity: 1 at core axis, 0 at envelope edge.
    let flame_t = clamp(1.0 - radial / max(disp, 0.001), 0.0, 1.0);

    // ── Inner dark cone ─────────────────────────────────────────────
    // Real candle flames have a dark inner cone of unburnt wax vapour
    // surrounded by the luminous reaction zone. The dark zone is a
    // narrow cone in the lower 40% of the flame.
    let inner_cone_r = max_r * 0.35 * (1.0 - norm_h * 2.5) * base_pinch;
    let in_dark_cone = select(0.0, 1.0, radial < inner_cone_r && norm_h < 0.40);

    // Hotness: hottest in the luminous zone (between inner cone and
    // outer edge), cooling toward tip and outer rim.
    let hot = flame_t * (1.0 - norm_h * 0.4) * (1.0 - in_dark_cone * 0.6);

    // ── Blackbody palette ───────────────────────────────────────────
    let rim    = vec3<f32>(0.85, 0.18, 0.04);
    let orange = vec3<f32>(1.00, 0.50, 0.10);
    let yellow = vec3<f32>(1.00, 0.86, 0.32);
    let white  = vec3<f32>(1.00, 0.98, 0.80);
    let blue   = vec3<f32>(0.30, 0.45, 0.90);

    var col = mix(rim, orange, smoothstep(0.0, 0.30, hot));
    col = mix(col, yellow, smoothstep(0.30, 0.55, hot));
    col = mix(col, white, smoothstep(0.55, 0.85, hot));
    // Blue tint near the wick base (combustion zone).
    let blue_zone = (1.0 - smoothstep(0.0, 0.15, norm_h)) * flame_t;
    col = mix(col, blue, blue_zone * 0.25);
    // Dark-cup dimming right at the wick.
    let cup = 1.0 - smoothstep(0.0, 0.06, norm_h);
    col = col * (1.0 - cup * 0.65);

    // ── Opacity / emission envelope ─────────────────────────────────
    let edge_fade = smoothstep(0.0, 0.25, flame_t);
    let tip_fade  = 1.0 - smoothstep(0.75, 1.0, norm_h);
    let base_fade = smoothstep(0.0, 0.06, norm_h);

    // Per-flame flicker (two frequencies so it feels organic).
    // Wind pumps the flicker harder — gusts make the flame visibly
    // pulse brighter and dimmer.
    let flicker = (0.88 + 0.12 * sin(time * 11.0 + phase)
                + 0.06 * sin(time * 19.0 + phase * 1.3))
                * (1.0 + wind_mag * 0.3);

    let alpha = edge_fade * tip_fade * base_fade * flicker
              * (1.0 - in_dark_cone * 0.5);

    return vec4<f32>(col * alpha, alpha);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let ndc = in.ndc;
    let near_world = world_from_ndc(ndc, 0.0);
    let far_world  = world_from_ndc(ndc, 1.0);
    let origin = cam.cam_pos.xyz;
    let dir = normalize(far_world - origin);

    let hit = ray_aabb(origin, dir);
    let t_near = max(hit.x, 0.0);
    let t_far_aabb = hit.y;             // unclamped AABB far
    var t_far = t_far_aabb;
    if (t_far <= t_near) {
        return vec4<f32>(0.0);
    }

    // Depth-clip against the scene for smoke (flames use t_far_aabb
    // separately — see pass 2 below).
    let pix = vec2<i32>(
        i32((ndc.x * 0.5 + 0.5) * globals.screen.x),
        i32((1.0 - (ndc.y * 0.5 + 0.5)) * globals.screen.y),
    );
    let dims = textureDimensions(depth_tex);
    let clamped = clamp(pix, vec2<i32>(0, 0), vec2<i32>(i32(dims.x) - 1, i32(dims.y) - 1));
    let depth_sample = textureLoad(depth_tex, clamped, 0);
    var scene_t = t_far_aabb;
    if (depth_sample < 0.999999) {
        let scene_world = world_from_ndc(ndc, depth_sample);
        scene_t = length(scene_world - origin);
        t_far = min(t_far, scene_t);
    }
    if (t_far <= t_near) {
        // Even if smoke is fully occluded, flames above the occluding
        // geometry might still be visible — fall through to pass 2.
    }

    let max_alpha = cam.params.x;
    let nsteps = max(i32(cam.params.y), 4);
    let span = t_far - t_near;
    let step = span / f32(nsteps);

    // Jitter to break up banding.
    let jitter = fract(sin(dot(in.clip_pos.xy + vec2<f32>(globals.time * 17.0, globals.time * 31.0), vec2<f32>(12.9898, 78.233))) * 43758.5453);
    var t = t_near + step * jitter;

    var color = vec3<f32>(0.0);
    var transmittance = 1.0;

    let inv_extent = 1.0 / (cam.grid_max.xyz - cam.grid_min.xyz);

    // ── Pass 1: smoke density march ─────────────────────────────────
    // Skipped if the depth buffer fully occludes the smoke AABB.
    let smoke_valid = t_far > t_near;
    for (var i: i32 = 0; i < nsteps; i = i + 1) {
        if (!smoke_valid) { break; }
        if (transmittance < 0.01) { break; }
        let pos = origin + dir * t;
        let uvw = (pos - cam.grid_min.xyz) * inv_extent;
        if (all(uvw >= vec3<f32>(0.0)) && all(uvw <= vec3<f32>(1.0))) {
            let sample = textureSampleLevel(lit_density_tex, density_samp, active_sample_uvw(uvw), 0.0);
            let density = max(sample.a, 0.0);
            if (density > 0.001) {
                // Linear-in-density extinction with a realistic coefficient:
                // wispy edges stay translucent via low density, dense cores
                // actually occlude. Previous pow(density, 0.72) flattened
                // the high end and coefficient 0.016 made dense smoke look
                // like fog — silhouettes never read as solid.
                let sigma_t = clamp(density, 0.0, 1.0) * 0.065;
                let absorb = 1.0 - exp(-sigma_t * step);
                color = color + transmittance * sample.rgb * absorb;
                transmittance = transmittance * (1.0 - absorb);
            }
        }
        t = t + step;
    }

    // ── Pass 2: per-candle SDF flame sub-march ──────────────────────
    // Each candle gets its own fine-resolution march through a tight
    // bounding sphere around the flame zone, ensuring the teardrop is
    // properly resolved regardless of the coarse smoke step size.
    // Skipped in smoke-only profiling mode (mode.x == 1).
    if (cam.mode.x > 0.5 && cam.mode.x < 1.5) {
        let alpha = (1.0 - transmittance) * max_alpha;
        let inv_g = 1.0 / max(globals.gamma, 0.01);
        let rgb = pow(color * max_alpha, vec3<f32>(inv_g));
        return vec4<f32>(rgb, alpha);
    }
    let candle_count = lights.count.y;
    let flame_h = max(lights.extras.z, 4.0);
    let flame_time = lights.extras.y;

    for (var ci: u32 = 0u; ci < candle_count; ci = ci + 1u) {
        if (transmittance < 0.01) { break; }

        let l = lights.lights[ci];
        let wick = l.pos.xyz;

        // Bounding sphere centred on the flame midpoint — expanded a
        // bit to accommodate wind-bent flames.
        let flame_center = wick + vec3<f32>(0.0, flame_h * 0.45, 0.0);
        let flame_radius = flame_h * 0.75;

        // Ray-sphere intersection. Use scene_t (not depth-clipped
        // t_far) so flames above the wick aren't occluded by the
        // candle's own mesh or nearby geometry below the flame.
        let oc = origin - flame_center;
        let b = dot(oc, dir);
        let c = dot(oc, oc) - flame_radius * flame_radius;
        let disc = b * b - c;
        if (disc < 0.0) { continue; }
        let sq = sqrt(disc);
        let ft_near = max(-b - sq, t_near);
        let ft_far  = min(-b + sq, scene_t);
        if (ft_far <= ft_near) { continue; }

        // Sample fluid velocity at several heights near the candle and
        // take the strongest horizontal component. Wind gusts are
        // injected near the table plane; the wick tip is far above,
        // so we probe at 3 heights to catch the gust wherever it is.
        var best_wind = vec3<f32>(0.0);
        var best_wind_sq: f32 = 0.0;
        for (var si: i32 = 0; si < 3; si = si + 1) {
            let frac = f32(si) * 0.25;   // 0.0, 0.25, 0.50 of wick height
            let sample_z = mix(cam.grid_min.z + 2.0, wick.z, frac);
            let sp = vec3<f32>(wick.x, wick.y, sample_z);
            let tx = world_to_texel(sp, cam.grid_min.xyz, cam.grid_max.xyz);
            let vs = textureLoad(velocity_tex, tx, 0);
            let hz = vs.x * vs.x + vs.y * vs.y;
            if (hz > best_wind_sq) {
                best_wind_sq = hz;
                best_wind = vec3<f32>(vs.x, vs.y, 0.0);
            }
        }
        // DEBUG: gentle idle sway so the flame always looks alive,
        // plus any sampled fluid velocity on top.
        let idle_sway = vec3<f32>(
            sin(flame_time * 1.1 + f32(ci) * 2.1) * 40.0,
            cos(flame_time * 0.9 + f32(ci) * 3.7) * 30.0,
            0.0,
        );
        let wind = idle_sway + vec3<f32>(
            clamp(best_wind.x, -120.0, 120.0),
            clamp(best_wind.y, -120.0, 120.0),
            0.0,
        );

        // Fine sub-march through the flame zone.
        let flame_steps = 20;
        let fstep = (ft_far - ft_near) / f32(flame_steps);
        var ft = ft_near + fstep * jitter;

        for (var fi: i32 = 0; fi < flame_steps; fi = fi + 1) {
            let fpos = origin + dir * ft;
            let flame = eval_flame(fpos, wick, flame_h, flame_time, f32(ci), wind);
            if (flame.a > 0.001) {
                // Flame emission: additive, self-luminous, weighted by
                // current transmittance so smoke in front dims it.
                let flame_absorb = 1.0 - exp(-flame.a * fstep * 0.05);
                color = color + transmittance * flame.rgb * flame_absorb * 6.0;
                // Flames are partially opaque — hot gas does occlude.
                transmittance = transmittance * (1.0 - flame_absorb * 0.35);
            }
            ft = ft + fstep;
        }
    }

    let alpha = (1.0 - transmittance) * max_alpha;
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    let rgb = pow(color * max_alpha, vec3<f32>(inv_g));
    return vec4<f32>(rgb, alpha);
}
