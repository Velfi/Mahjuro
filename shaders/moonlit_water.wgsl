// Procedural moon-over-water vignette for the victory screen.
//
// Fullscreen triangle — no vertex buffers. The upper half builds a soft
// nocturne sky around a bright moon while the lower half renders layered,
// horizontally advected ripples and a broken reflection path.

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

fn moon_mask(uv: vec2<f32>, center: vec2<f32>, aspect: f32) -> f32 {
    let delta = (uv - center) * vec2<f32>(aspect, 1.0);
    let body = smoothstep(0.085, 0.062, length(delta));
    let notch = smoothstep(0.078, 0.03, length((uv - center - vec2<f32>(0.028, -0.006)) * vec2<f32>(aspect, 1.0)));
    return max(body - notch * 0.92, 0.0);
}

fn halo_mask(uv: vec2<f32>, center: vec2<f32>, aspect: f32) -> f32 {
    let delta = (uv - center) * vec2<f32>(aspect, 1.0);
    let dist = length(delta);
    let broad = smoothstep(0.42, 0.0, dist);
    let grain = 0.75 + 0.25 * fbm2(delta * vec2<f32>(5.0, 3.0) + vec2<f32>(0.0, globals.time * 0.03));
    return broad * broad * grain;
}

fn ripple_height(x: f32, y: f32, time: f32) -> f32 {
    let band0 = sin(x * 38.0 + time * 1.3 + y * 24.0) * 0.55;
    let band1 = sin(x * 71.0 - time * 1.8 + y * 46.0) * 0.30;
    let band2 = sin(x * 122.0 + time * 2.5 + y * 80.0) * 0.15;
    let swirl = fbm2(vec2<f32>(x * 5.0, y * 12.0 + time * 0.10)) - 0.5;
    return band0 + band1 + band2 + swirl * 0.6;
}

fn horizon_haze(uv: vec2<f32>, aspect: f32, time: f32) -> f32 {
    let p = vec2<f32>((uv.x - 0.5) * aspect * 1.2, (uv.y - 0.57) * 9.0);
    let dist = length(p);
    let base = smoothstep(0.42, 0.0, dist);
    let breakup = 0.65 + 0.35 * fbm2(vec2<f32>(p.x * 3.0 + time * 0.05, p.y * 2.2));
    return base * breakup;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = globals.time;
    let aspect = globals.screen.x / globals.screen.y;
    let cursor_uv = globals.cursor_pos / globals.screen;
    let parallax = (cursor_uv - vec2<f32>(0.5)) * vec2<f32>(0.018, 0.010);

    let moon_center = vec2<f32>(0.5, 0.30) + parallax;
    let moon = moon_mask(uv, moon_center, aspect);
    let halo = halo_mask(uv, moon_center, aspect);

    let sky_top = vec3<f32>(0.01, 0.03, 0.09);
    let sky_mid = vec3<f32>(0.03, 0.08, 0.16);
    let horizon = vec3<f32>(0.10, 0.15, 0.24);
    let water_dark = vec3<f32>(0.01, 0.04, 0.08);
    let water_glow = vec3<f32>(0.14, 0.23, 0.34);
    let moon_tint = vec3<f32>(0.95, 0.97, 1.0);

    let sky_mix = smoothstep(0.60, 0.02, uv.y);
    var color = mix(sky_top, sky_mid, sky_mix);
    color = mix(color, horizon, smoothstep(0.68, 0.48, uv.y) * 0.65);

    let haze = horizon_haze(uv, aspect, t);
    color += vec3<f32>(0.04, 0.06, 0.09) * haze * 0.65;

    let water_start = 0.56;
    if (uv.y > water_start) {
        let water_y = (uv.y - water_start) / max(1.0 - water_start, 0.001);
        let drifted_x = uv.x + parallax.x * 0.4;
        let h = ripple_height(drifted_x, water_y, t);
        let h_dx = ripple_height(drifted_x + 0.0025, water_y, t) - h;
        let band = 0.5 + 0.5 * h;

        let depth_mix = smoothstep(1.0, 0.0, water_y);
        let water_base = mix(water_dark, water_glow, depth_mix * 0.5);
        color = water_base;

        let crest = smoothstep(0.50, 0.95, band) * (0.35 + 0.65 * (1.0 - water_y));
        color += vec3<f32>(0.08, 0.13, 0.18) * crest;

        let reflection_center = moon_center.x + h_dx * 1.8;
        let reflection_width = mix(0.16, 0.028, water_y);
        let reflection_dx = abs(drifted_x - reflection_center);
        let reflection_column = smoothstep(reflection_width, 0.0, reflection_dx);
        let breakup = smoothstep(0.15, 0.95, band) * (0.75 + 0.25 * fbm2(vec2<f32>(drifted_x * 8.0, water_y * 18.0 - t * 0.2)));
        let shimmer = 0.55 + 0.45 * sin((water_y * 120.0) - t * 3.6 + h * 3.5);
        let reflection = reflection_column * breakup * shimmer * (1.0 - water_y * 0.35);
        color += moon_tint * reflection * 0.85;

        let far_glow = smoothstep(0.08, 0.0, abs(uv.y - water_start));
        color += vec3<f32>(0.05, 0.08, 0.12) * far_glow;
    }

    color += moon_tint * moon;
    color += vec3<f32>(0.30, 0.36, 0.44) * halo * 0.42;

    let moon_column = smoothstep(0.22, 0.0, abs(uv.x - moon_center.x));
    let sky_sheen = moon_column * smoothstep(0.68, 0.20, uv.y) * smoothstep(0.56, 0.24, uv.y);
    color += vec3<f32>(0.05, 0.07, 0.10) * sky_sheen;

    let vignette_delta = (uv - vec2<f32>(0.5, 0.56)) * vec2<f32>(aspect * 0.86, 1.0);
    let vignette = smoothstep(1.08, 0.12, length(vignette_delta));
    color *= vignette;

    let inv_g = 1.0 / max(globals.gamma, 0.01);
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(inv_g));
    return vec4<f32>(color, 0.0);
}
