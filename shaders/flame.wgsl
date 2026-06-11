// Volumetric candle flame — digital-garden plume sim + palette layers.
// Mesh meta: position = (cos_theta, y01, sin_theta); Y-up plume → Mahjuro Z-up world.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct FlameShaderTuning {
    flame_height_mul: f32,
    wind_strength: f32,
    turbulence: f32,
    emission_gain: f32,
    flame_width_mul: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
};

struct FlameView {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
    tuning: FlameShaderTuning,
    _pad: vec4<f32>,
};
@group(1) @binding(0) var<uniform> view: FlameView;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) flame: vec2<f32>,
    @location(1) @interpolate(flat) brightness: f32,
};

const FLAME_BASE: f32 = 0.004;
const FLAME_HEIGHT_EXP: f32 = 0.82;
const FLAME_HEIGHT_UNIT: f32 = 1.0;
/// digital-garden `flameHeight` the envelope widths were authored for.
const DESIGN_FLAME_HEIGHT: f32 = 0.34;
/// Multiplier on all plume FBM time terms (turbulence scroll, sway, eddies).
const PLUME_ANIM_SPEED: f32 = 2.0;
const FBM_OCTAVES: u32 = 4u;
const FBM_LACUNARITY: f32 = 1.9;
/// Noise input scale — lower = broader, smoother deformation (was ~14–31 on radial coords).
const PLUME_TURB_SCALE: f32 = 7.5;
const PLUME_TURB_HEIGHT: f32 = 9.5;
const PLUME_TURB_FINE_SCALE: f32 = 14.0;
const PLUME_TURB_FINE_HEIGHT: f32 = 12.0;
const PLUME_EDDY_Y: f32 = 2.6;
const PLUME_EDDY_Y01: f32 = 1.5;
const PLUME_WIND_EDDY_Y: f32 = 4.5;

fn hash3(x: f32, y: f32, z: f32) -> f32 {
    var p = vec3<f32>(x, y, z);
    p = fract(p * 0.3183099 + vec3<f32>(0.1, 0.2, 0.3));
    p = p + dot(p, p.yzx + 19.19);
    return fract((p.x + p.y) * p.z * 127.1);
}

fn fade3(t: vec3<f32>) -> vec3<f32> {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

/// Ken Perlin improved 3D gradient — smoother than value noise at cell corners.
fn grad_dot(h: f32, x: f32, y: f32, z: f32) -> f32 {
    let u = select(y, x, h < 0.5);
    let v = select(z, x, h < 0.25 || h >= 0.75);
    let su = select(-1.0, 1.0, fract(h * 767.0) < 0.5);
    let sv = select(-1.0, 1.0, fract(h * 313.0) < 0.5);
    return su * u + sv * v;
}

fn noise3(x: f32, y: f32, z: f32) -> f32 {
    let pi = floor(vec3<f32>(x, y, z));
    let pf = fract(vec3<f32>(x, y, z));
    let w = fade3(pf);

    let n000 = grad_dot(hash3(pi.x, pi.y, pi.z), pf.x, pf.y, pf.z);
    let n100 = grad_dot(hash3(pi.x + 1.0, pi.y, pi.z), pf.x - 1.0, pf.y, pf.z);
    let n010 = grad_dot(hash3(pi.x, pi.y + 1.0, pi.z), pf.x, pf.y - 1.0, pf.z);
    let n110 = grad_dot(hash3(pi.x + 1.0, pi.y + 1.0, pi.z), pf.x - 1.0, pf.y - 1.0, pf.z);
    let n001 = grad_dot(hash3(pi.x, pi.y, pi.z + 1.0), pf.x, pf.y, pf.z - 1.0);
    let n101 = grad_dot(hash3(pi.x + 1.0, pi.y, pi.z + 1.0), pf.x - 1.0, pf.y, pf.z - 1.0);
    let n011 = grad_dot(hash3(pi.x, pi.y + 1.0, pi.z + 1.0), pf.x, pf.y - 1.0, pf.z - 1.0);
    let n111 = grad_dot(
        hash3(pi.x + 1.0, pi.y + 1.0, pi.z + 1.0),
        pf.x - 1.0,
        pf.y - 1.0,
        pf.z - 1.0,
    );

    let nx00 = mix(mix(n000, n100, w.x), mix(n010, n110, w.x), w.y);
    let nx10 = mix(mix(n001, n101, w.x), mix(n011, n111, w.x), w.y);
    return mix(nx00, nx10, w.z);
}

fn fbm3(x: f32, y: f32, z: f32) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var norm = 0.0;
    var px = x;
    var py = y;
    var pz = z;
    for (var i = 0u; i < FBM_OCTAVES; i = i + 1u) {
        v = v + a * noise3(px, py, pz);
        norm = norm + a;
        let nx = 0.8 * px - 0.6 * py + 0.2 * pz;
        let ny = 0.6 * px + 0.8 * py + 0.1 * pz;
        let nz = -0.2 * px + 0.1 * py + 0.9 * pz;
        px = nx * FBM_LACUNARITY + 13.7;
        py = ny * FBM_LACUNARITY + 7.1;
        pz = nz * FBM_LACUNARITY + 3.9;
        a = a * 0.5;
    }
    return v / max(norm, 1e-6);
}

fn wick_curve_x(y: f32) -> f32 {
    let WICK_TOP = 0.012;
    let t = clamp((y + 0.006) / (WICK_TOP + 0.006), 0.0, 1.0);
    return t * t * 0.025;
}

fn flame_height_at(y01: f32, flame_height: f32) -> f32 {
    return FLAME_BASE + pow(y01, FLAME_HEIGHT_EXP) * flame_height;
}

fn base_pin_weight(y01: f32) -> f32 {
    let t = clamp(y01, 0.0, 1.0);
    return 1.0 - pow(min(1.0, t / 0.14), 1.6);
}

fn flame_envelope_width(y01: f32) -> f32 {
    let t = clamp(y01, 0.0, 1.0);
    let foot_open = sin(min(1.0, t / 0.11) * 1.5707963);
    let wick = pow(min(1.0, t / 0.06), 0.45);
    let tip = pow(1.0 - t, 0.82);
    let belly = exp(-pow((t - 0.3) / 0.28, 2.0));
    return (0.002 + 0.034 * wick * tip * (0.34 + 0.66 * belly)) * foot_open;
}

fn plume_unit_scale(flame_height: f32) -> f32 {
    return flame_height / DESIGN_FLAME_HEIGHT;
}

fn plume_turb_sample(
    lx: f32,
    ly: f32,
    lz: f32,
    time: f32,
    turbulence: f32,
    phase_seed: f32,
) -> vec2<f32> {
    let anim = time * PLUME_ANIM_SPEED;
    let rising = anim * (1.4 + turbulence * 0.6);
    let turb = fbm3(
        lx * PLUME_TURB_SCALE + rising * 0.22 + phase_seed,
        (ly - FLAME_BASE) * PLUME_TURB_HEIGHT - rising * 0.55,
        lz * PLUME_TURB_SCALE + rising * 0.18 + phase_seed * 0.61,
    ) - 0.5;
    let turb_fine = fbm3(
        lx * PLUME_TURB_FINE_SCALE + rising * 0.38 + phase_seed * 1.7,
        (ly - FLAME_BASE) * PLUME_TURB_FINE_HEIGHT - rising * 0.92,
        lz * PLUME_TURB_FINE_SCALE + rising * 0.31 + phase_seed * 0.43,
    ) - 0.5;
    return vec2<f32>(turb, turb_fine);
}

fn plume_anchor(y: f32, time: f32, wind: vec2<f32>, wind_strength: f32, turbulence: f32, flame_height: f32) -> vec2<f32> {
    let y01 = clamp(y / flame_height, 0.0, 1.0);
    var ax = wick_curve_x(y);
    var az = 0.0;
    let pin = base_pin_weight(y01);
    let plume_scale = plume_unit_scale(flame_height);

    let anim = time * PLUME_ANIM_SPEED;

    // Indoor sway: eddy on the plume axis even when wind vector is zero.
    let indoor_eddy = fbm3(
        y * PLUME_EDDY_Y - anim * 1.35,
        y01 * PLUME_EDDY_Y01 + anim * 0.65,
        anim * 0.35 + wind.x,
    ) - 0.5;
    ax = ax + indoor_eddy * 0.012 * turbulence * y01 * y01 * pin * plume_scale;
    az = az + indoor_eddy * 0.009 * turbulence * y01 * y01 * pin * plume_scale;

    if (wind_strength <= 0.0) {
        return vec2<f32>(ax, az);
    }

    let bend = y01 * y01 * wind_strength * pin;
    let eddy = fbm3(wind.x * 2.0, y * PLUME_WIND_EDDY_Y - anim * 1.2, wind.y * 2.0) - 0.5;
    ax = ax + wind.x * bend * 0.14 * plume_scale + eddy * 0.018 * turbulence * y01 * plume_scale;
    az = az + wind.y * bend * 0.1 * plume_scale + eddy * 0.012 * turbulence * y01 * plume_scale;
    return vec2<f32>(ax, az);
}

fn sim_displacement(
    lx: f32,
    ly: f32,
    lz: f32,
    y01: f32,
    time: f32,
    wind_strength: f32,
    turbulence: f32,
    phase_seed: f32,
    plume_scale: f32,
) -> vec3<f32> {
    let r = max(length(vec2<f32>(lx, lz)), 0.0001);
    let nx = lx / r;
    let nz = lz / r;
    let turb_pair = plume_turb_sample(lx, ly, lz, time, turbulence, phase_seed);
    let turb = turb_pair.x;
    let turb_fine = turb_pair.y;

    let breathe = (turb * 0.014 + turb_fine * 0.0025) * y01 * turbulence;
    let dy = (turb * 0.006 + turb_fine * 0.001) * turbulence * (0.3 + y01);
    let pin = base_pin_weight(y01);

    var out: vec3<f32>;
    if (wind_strength <= 0.0) {
        out = vec3<f32>(nx * breathe * pin, dy * pin, nz * breathe * pin);
    } else {
        let dx = (turb * 0.022 * turbulence * y01 + nx * breathe) * pin;
        let dz = (turb * 0.018 * turbulence * y01 + nz * breathe) * pin;
        out = vec3<f32>(dx, dy * pin, dz);
    }
    return out * plume_scale;
}

fn plume_brightness_flicker(turb: f32, y01: f32, turbulence: f32) -> f32 {
    let dance = y01 * y01;
    return 1.0 + turb * 0.30 * dance * turbulence;
}

fn flame_envelope_width_scaled(y01: f32, flame_height: f32, width_mul: f32) -> f32 {
    return flame_envelope_width(y01)
        * (flame_height / DESIGN_FLAME_HEIGHT)
        * width_mul;
}

fn flame_local_radial(y01: f32, lx: f32, lz: f32, flame_height: f32, width_mul: f32) -> f32 {
    let width = flame_envelope_width_scaled(y01, flame_height, width_mul);
    return min(1.0, length(vec2<f32>(lx, lz)) / max(width, 0.0001));
}

fn vs_plume(
    mesh_meta: vec3<f32>,
    inst_anchor_wind: vec4<f32>,
    inst_params: vec4<f32>,
    layer: u32,
) -> VsOut {
    let anchor = inst_anchor_wind.xyz;
    let wind = vec2<f32>(inst_anchor_wind.w, inst_params.w);
    let scale = inst_params.x;
    let brightness = inst_params.z;
    let time = globals.time + inst_params.y * 6.2831853;

    let cos_theta = mesh_meta.x;
    let y01 = mesh_meta.y;
    let sin_theta = mesh_meta.z;

    let flame_height = FLAME_HEIGHT_UNIT * view.tuning.flame_height_mul;
    let plume_scale = plume_unit_scale(flame_height);
    let phase_seed = inst_params.y * 6.2831853 + cos_theta * 1.73 + sin_theta * 2.41;
    let width = flame_envelope_width_scaled(y01, flame_height, view.tuning.flame_width_mul);
    let ry = flame_height_at(y01, flame_height);
    let rx = cos_theta * width;
    let rz = sin_theta * width;

    let anchor_xz = plume_anchor(ry, time, wind, view.tuning.wind_strength, view.tuning.turbulence, flame_height);
    let turb_pair = plume_turb_sample(rx, ry, rz, time, view.tuning.turbulence, phase_seed);
    let disp = sim_displacement(
        rx,
        ry,
        rz,
        y01,
        time,
        view.tuning.wind_strength,
        view.tuning.turbulence,
        phase_seed,
        plume_scale,
    );
    var lx = rx + disp.x;
    var ly = ry + disp.y;
    var lz = rz + disp.z;
    var px = anchor_xz.x + lx;
    var py = ly;
    var pz = anchor_xz.y + lz;

    let radial = flame_local_radial(y01, lx, lz, flame_height, view.tuning.flame_width_mul);
    var out_radial = radial;

    if (layer == 2u) {
        let core_pull = 0.62;
        px = anchor_xz.x + lx * core_pull;
        pz = anchor_xz.y + lz * core_pull;
        out_radial = radial * 0.72;
    } else if (layer == 0u) {
        let glow_scale = mix(2.65, 1.72, y01);
        lx = lx * glow_scale;
        lz = lz * glow_scale;
        px = anchor_xz.x + lx;
        py = ly + flame_height * 0.015 * (1.0 - y01);
        pz = anchor_xz.y + lz;
        out_radial = radial;
    }

    let world = anchor + vec3<f32>(px, pz, py) * scale;

    var out: VsOut;
    out.clip_position = view.view_proj * vec4<f32>(world, 1.0);
    out.flame = vec2<f32>(y01, out_radial);
    out.brightness = brightness * plume_brightness_flicker(turb_pair.x, y01, view.tuning.turbulence);
    return out;
}

@vertex
fn vs_glow(
    @location(0) mesh_meta: vec3<f32>,
    @location(1) mesh_nrm: vec3<f32>,
    @location(2) mesh_uv: vec2<f32>,
    @location(3) inst_anchor_wind: vec4<f32>,
    @location(4) inst_params: vec4<f32>,
) -> VsOut {
    _ = mesh_nrm;
    _ = mesh_uv;
    return vs_plume(mesh_meta, inst_anchor_wind, inst_params, 0u);
}

@vertex
fn vs_main(
    @location(0) mesh_meta: vec3<f32>,
    @location(1) mesh_nrm: vec3<f32>,
    @location(2) mesh_uv: vec2<f32>,
    @location(3) inst_anchor_wind: vec4<f32>,
    @location(4) inst_params: vec4<f32>,
) -> VsOut {
    _ = mesh_nrm;
    _ = mesh_uv;
    return vs_plume(mesh_meta, inst_anchor_wind, inst_params, 1u);
}

@vertex
fn vs_core(
    @location(0) mesh_meta: vec3<f32>,
    @location(1) mesh_nrm: vec3<f32>,
    @location(2) mesh_uv: vec2<f32>,
    @location(3) inst_anchor_wind: vec4<f32>,
    @location(4) inst_params: vec4<f32>,
) -> VsOut {
    _ = mesh_nrm;
    _ = mesh_uv;
    return vs_plume(mesh_meta, inst_anchor_wind, inst_params, 2u);
}

fn fs_body(in: VsOut) -> vec4<f32> {
    let y01 = in.flame.x;
    let radial = in.flame.y;

    let white = vec3<f32>(1.0, 0.98, 0.9);
    let pale = vec3<f32>(1.0, 0.92, 0.58);
    let amber = vec3<f32>(1.0, 0.74, 0.2);
    let orange = vec3<f32>(1.0, 0.48, 0.1);
    let blue = vec3<f32>(0.15, 0.35, 1.0);
    let blue_deep = vec3<f32>(0.08, 0.15, 0.75);

    let temp = clamp(y01 * 0.7 + (1.0 - radial) * 0.38, 0.0, 1.0);
    var col = mix(orange, amber, smoothstep(0.05, 0.42, temp));
    col = mix(col, pale, smoothstep(0.28, 0.62, temp));

    var core = (1.0 - smoothstep(0.0, 0.42, radial));
    core = core * smoothstep(0.05, 0.32, y01) * (1.0 - smoothstep(0.72, 0.98, y01));
    col = mix(col, white, core * 0.95);

    let shell = smoothstep(0.35, 0.95, radial);
    col = mix(col, amber * 1.15, shell * 0.42);

    let blue_height = smoothstep(0.46, 0.0, y01);
    let blue_skirt = blue_height * smoothstep(0.12, 0.92, radial);
    let blue_inner = blue_height * smoothstep(0.68, 0.0, radial) * 0.5;
    let blue_amt = clamp(max(blue_skirt, blue_inner), 0.0, 1.0);
    col = mix(col, mix(blue_deep, blue, radial), blue_amt * 0.88);

    let base_round = smoothstep(0.0, 0.1, y01 + (1.0 - radial) * 0.08);
    col = col * base_round;

    let tip_fade = smoothstep(0.82, 0.99, y01);
    col = mix(col, amber * 0.65, tip_fade * 0.22);

    var edge = 1.0 - smoothstep(0.88, 1.08, radial);
    edge = pow(edge, 0.65);
    var tip = 1.0 - smoothstep(0.78, 1.02, y01);
    tip = pow(tip, 0.9);
    let body = mix(0.72, 1.0, smoothstep(0.04, 0.42, y01));
    let blue_weight = mix(1.0, 0.55, blue_amt * 0.7);
    let base_w = smoothstep(0.0, 0.11, y01 + (1.0 - radial) * 0.09);
    let weight = edge * tip * body * blue_weight * base_w;

    let gain = view.tuning.emission_gain * in.brightness;
    return vec4<f32>(col * 2.05 * weight * gain, weight);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return fs_body(in);
}

@fragment
fn fs_core(in: VsOut) -> @location(0) vec4<f32> {
    let y01 = in.flame.x;
    let radial = in.flame.y;

    var core = (1.0 - smoothstep(0.0, 0.55, radial));
    core = core * smoothstep(0.08, 0.42, y01) * (1.0 - smoothstep(0.62, 0.95, y01));

    let col = mix(vec3<f32>(1.0, 0.88, 0.45), vec3<f32>(1.0, 0.98, 0.92), core);
    let weight = core * 1.45;
    let gain = view.tuning.emission_gain * in.brightness;
    return vec4<f32>(col * weight * gain, weight);
}

@fragment
fn fs_glow(in: VsOut) -> @location(0) vec4<f32> {
    let y01 = in.flame.x;
    let radial = in.flame.y;

    var inner = 1.0 - smoothstep(0.18, 0.74, radial);
    inner = pow(max(inner, 0.0), 1.15);
    var outer = 1.0 - smoothstep(0.42, 1.04, radial);
    outer = pow(max(outer, 0.0), 1.65);
    let vertical = smoothstep(0.0, 0.13, y01) * (1.0 - smoothstep(0.86, 1.0, y01));
    let aura = (inner * 0.52 + outer * 0.32) * vertical;

    var col = vec3<f32>(1.0, 0.50, 0.10) * (outer * 0.82 + inner * 0.58);
    col = col + vec3<f32>(0.2, 0.35, 1.0) * smoothstep(0.44, 0.0, y01) * inner * 0.22;

    var weight = aura * 0.58;
    let base_round = smoothstep(0.0, 0.12, y01 + (1.0 - radial) * 0.1);
    weight = weight * base_round;
    let gain = view.tuning.emission_gain * in.brightness;
    return vec4<f32>(col * gain * 1.35, weight);
}
