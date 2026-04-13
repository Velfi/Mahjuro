// Procedural sun-over-water vignette for the defeat screen.
//
// Mirrors the fullscreen moon-water composition, but shifts to a warm dawn
// palette so loss reads clearly distinct from the moonlit victory scene.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
    cursor_pos: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
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
    out.clip_pos = vec4<f32>(p, 0.9999, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    return out;
}

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);

    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));

    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm2(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.55;
    var f = p;
    for (var i = 0; i < 4; i += 1) {
        v += value_noise(f) * amp;
        f = f * 2.03 + vec2<f32>(3.7, 5.1);
        amp *= 0.5;
    }
    return v;
}

fn sun_mask(uv: vec2<f32>, center: vec2<f32>, aspect: f32) -> f32 {
    let delta = (uv - center) * vec2<f32>(aspect, 1.0);
    return smoothstep(0.090, 0.060, length(delta));
}

fn halo_mask(uv: vec2<f32>, center: vec2<f32>, aspect: f32) -> f32 {
    let delta = (uv - center) * vec2<f32>(aspect, 1.0);
    let dist = length(delta);
    let broad = smoothstep(0.48, 0.0, dist);
    let grain = 0.78 + 0.22 * fbm2(delta * vec2<f32>(4.0, 2.5) + vec2<f32>(0.0, globals.time * 0.02));
    return broad * broad * grain;
}

fn water_height(p: vec2<f32>, time: f32) -> f32 {
    let far_mix = 1.0 - p.y;
    let x_scale = mix(16.0, 44.0, far_mix);
    let warp = (fbm2(vec2<f32>(p.x * 1.2 - time * 0.008, p.y * 2.8 + time * 0.006)) - 0.5) * 0.05;
    let long_x = p.x + warp;

    let swell0 = sin(long_x * x_scale + time * 0.40) * 0.32;
    let swell1 = sin(long_x * (x_scale * 2.1) - time * 0.52 + p.y * 1.1) * 0.12;
    let chop = (fbm2(vec2<f32>(
        p.x * mix(7.0, 16.0, far_mix) - time * 0.020,
        p.y * mix(3.5, 14.0, far_mix) + time * 0.010,
    )) - 0.5) * 0.18;
    let capillary = sin(long_x * mix(34.0, 86.0, far_mix) - time * 0.85) * 0.030;
    return swell0 + swell1 + chop + capillary;
}

fn horizon_haze(uv: vec2<f32>, aspect: f32, time: f32) -> f32 {
    let p = vec2<f32>((uv.x - 0.5) * aspect * 1.2, (uv.y - 0.57) * 8.8);
    let dist = length(p);
    let base = smoothstep(0.50, 0.0, dist);
    let breakup = 0.7 + 0.3 * fbm2(vec2<f32>(p.x * 2.8 + time * 0.04, p.y * 2.0));
    return base * breakup;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = globals.time;
    let aspect = globals.screen.x / globals.screen.y;
    let sun_center = vec2<f32>(0.5, 0.29);
    let sun = sun_mask(uv, sun_center, aspect);
    let halo = halo_mask(uv, sun_center, aspect);

    // Sunset palette: deep indigo-violet sky, orange horizon, dark purple water
    // with warm orange reflections — matching reference photo (Bigstock 460494471).
    let sky_top    = vec3<f32>(0.06, 0.06, 0.18);   // deep indigo-navy zenith
    let sky_mid    = vec3<f32>(0.28, 0.13, 0.32);   // violet band
    let horizon    = vec3<f32>(0.92, 0.48, 0.18);   // hot orange at the horizon line
    let water_dark = vec3<f32>(0.08, 0.06, 0.18);   // deep blue-violet trough
    let water_glow = vec3<f32>(0.22, 0.12, 0.28);   // purple mid-water
    let sun_tint   = vec3<f32>(1.0,  0.72, 0.28);   // warm amber-gold sun

    let sky_mix = smoothstep(0.62, 0.04, uv.y);
    var color = mix(sky_top, sky_mid, sky_mix);
    color = mix(color, horizon, smoothstep(0.72, 0.46, uv.y) * 0.88);

    // Warm orange haze band around the horizon.
    let haze = horizon_haze(uv, aspect, t);
    color += vec3<f32>(0.38, 0.18, 0.04) * haze * 0.80;

    // Soft pink-violet cloud blush above the haze.
    let cloud_blush = smoothstep(0.55, 0.28, uv.y) * smoothstep(0.18, 0.44, uv.y);
    color += vec3<f32>(0.28, 0.08, 0.22) * cloud_blush * 0.35;

    let water_start = 0.56;
    if (uv.y > water_start) {
        let water_y = (uv.y - water_start) / max(1.0 - water_start, 0.001);
        let drifted_x = uv.x;
        let water_p = vec2<f32>(drifted_x, water_y);
        let h = water_height(water_p, t);
        let h_dx = water_height(water_p + vec2<f32>(0.0022, 0.0), t) - h;
        let h_dy = water_height(water_p + vec2<f32>(0.0, 0.0032), t) - h;
        let surface_n = normalize(vec3<f32>(-h_dx * 24.0, 1.0, -h_dy * 10.0));
        let view_dir = normalize(vec3<f32>(0.0, 0.88, 0.48));
        let fresnel = pow(1.0 - max(dot(surface_n, view_dir), 0.0), 3.6);
        let band = 0.5 + 0.5 * h;

        let depth_mix = smoothstep(1.0, 0.0, water_y);
        let water_base = mix(water_dark, water_glow, depth_mix * 0.50);
        color = water_base;
        color += vec3<f32>(0.030, 0.014, 0.028) * smoothstep(0.24, 0.0, water_y);

        // Wave crests pick up a warm violet-rose scatter.
        let crest = smoothstep(0.58, 0.96, band) * (0.20 + 0.80 * (1.0 - water_y));
        let trough = smoothstep(0.42, 0.08, band) * (0.35 + 0.65 * water_y);
        color += vec3<f32>(0.18, 0.06, 0.12) * crest;
        color -= vec3<f32>(0.03, 0.02, 0.05) * trough;
        color += vec3<f32>(0.09, 0.04, 0.03) * fresnel * (0.30 + 0.70 * depth_mix);

        // Orange sun reflection column.
        let reflection_center = sun_center.x + h_dx * 0.60 + h_dy * 0.06;
        let reflection_width = mix(0.19, 0.040, water_y);
        let reflection_dx = abs(drifted_x - reflection_center);
        let reflection_column = smoothstep(reflection_width, 0.0, reflection_dx);
        let glint_mask = pow(max(1.0 - abs(h_dx) * 78.0 - abs(h_dy) * 26.0, 0.0), 6.2);
        let sparkle_noise = fbm2(vec2<f32>(drifted_x * 16.0 - t * 0.04, water_y * 86.0 + t * 0.025));
        let streaks = 0.5 + 0.5 * sin(water_y * 300.0 - t * 1.55 + sparkle_noise * 8.5);
        let breakup = smoothstep(0.62, 0.92, streaks) * smoothstep(0.46, 0.88, sparkle_noise)
            * (0.26 + 0.74 * glint_mask);
        let shimmer = 0.84 + 0.16 * sin(water_y * 185.0 - t * 1.35 + h * 2.8);
        let wide_glow = smoothstep(reflection_width * 1.85, 0.0, reflection_dx)
            * (0.30 + 0.70 * smoothstep(0.30, 0.85, sparkle_noise))
            * mix(0.90, 0.36, water_y);
        let reflection = reflection_column * breakup * shimmer * mix(0.98, 0.34, water_y);
        color += sun_tint * reflection * 0.82;
        color += sun_tint * wide_glow * 0.16;

        let horizon_blend = smoothstep(0.075, 0.0, water_y);
        color = mix(color, horizon * 1.02, horizon_blend * 0.34);

        // Horizon-kiss glow where sky meets water.
        let far_glow = smoothstep(0.032, 0.0, abs(uv.y - water_start));
        color += vec3<f32>(0.30, 0.14, 0.06) * far_glow * 0.45;
    }

    color += sun_tint * sun;
    color += vec3<f32>(0.55, 0.28, 0.08) * halo * 0.52;   // wider warm corona

    // Vertical orange pillar rising from sun into sky.
    let sun_column = smoothstep(0.24, 0.0, abs(uv.x - sun_center.x));
    let sky_sheen = sun_column * smoothstep(0.70, 0.18, uv.y) * smoothstep(0.58, 0.22, uv.y);
    color += vec3<f32>(0.28, 0.12, 0.04) * sky_sheen;

    let vignette_delta = (uv - vec2<f32>(0.5, 0.56)) * vec2<f32>(aspect * 0.86, 1.0);
    let vignette = smoothstep(1.08, 0.12, length(vignette_delta));
    color *= vignette;

    let inv_g = 1.0 / max(globals.gamma, 0.01);
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(inv_g));
    return vec4<f32>(color, 0.0);
}
