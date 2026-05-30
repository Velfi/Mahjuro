// Volumetric candle flame — Godot-style shell + black-body colour.
// https://godotshaders.com/shader/lighter-candle-flame/
//
// Mesh authored Y-up; mapped to Mahjuro Z-up in the vertex stage.
// Instance: [`GpuFlameInstance`] → locations 3–4 (anchor+wind_x, scale+phase+brightness+wind_y).
// Tuning: live [`flame_tuning::FlameTuning`] uploaded in `FlameView.tuning`.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct FlameShaderTuning {
    flame_height: f32,
    flame_width: f32,
    taper_factor: f32,
    height_rw_rate: f32,
    height_rw_amp: f32,
    bright_rw_rate: f32,
    micro_rw_rate: f32,
    bright_rw_amp: f32,
    emission_gain: f32,
    max_alpha: f32,
    border_width: f32,
    bottom_fade_y_start: f32,
    bottom_fade_y_end: f32,
};

struct FlameView {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
    tuning: FlameShaderTuning,
};
@group(1) @binding(0) var<uniform> view: FlameView;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) brightness: f32,
    @location(2) world_pos: vec3<f32>,
    @location(3) world_nrm: vec3<f32>,
    @location(4) @interpolate(flat) dance_w: f32,
};

fn hash21(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = hash21(i + vec2<f32>(0.0, 0.0));
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

/// Smoothed stepped noise — fast random-walk feel (sync rates with `flame_volume.rs`).
fn flame_rw_1d(seed: f32, t: f32, rate: f32) -> f32 {
    let x = t * rate;
    let cell = floor(x);
    let f = fract(x);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(vec2<f32>(seed, cell)) * 2.0 - 1.0;
    let b = hash21(vec2<f32>(seed, cell + 1.0)) * 2.0 - 1.0;
    return mix(a, b, u);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var pp = p;
    for (var i = 0; i < 3; i = i + 1) {
        v = v + amp * vnoise(pp);
        pp = pp * 2.03;
        amp = amp * 0.5;
    }
    return v;
}

@vertex
fn vs_main(
    @location(0) mesh_pos: vec3<f32>,
    @location(1) mesh_nrm: vec3<f32>,
    @location(2) mesh_uv: vec2<f32>,
    @location(3) inst_anchor_wind: vec4<f32>,
    @location(4) inst_params: vec4<f32>,
) -> VsOut {
    let anchor = inst_anchor_wind.xyz;
    let wind = vec2<f32>(inst_anchor_wind.w, inst_params.w);
    let scale = inst_params.x;
    let brightness = inst_params.z;
    let phase = inst_params.y;
    let t = globals.time;
    let ph = phase * 6.2831853;

    let height = 1.0 - mesh_uv.y;
    let tip_t = mesh_uv.y;
    let dance_w = tip_t * tip_t;

    var pos = mesh_pos;
    pos.x = mix(pos.x, 0.0, tip_t * view.tuning.taper_factor);
    pos.z = mix(pos.z, 0.0, tip_t * view.tuning.taper_factor);

    // Indoor: whole column barely leans; gust wind is heavily damped in the emitter.
    let col_phase = pos.y * 1.1 + ph;
    let sway_x = sin(t * 0.72 + col_phase) * 0.10;
    let sway_z = cos(t * 0.68 + col_phase * 0.92) * 0.08;
    let move_amp = scale * 0.018 * dance_w;
    pos.x += sway_x * move_amp + wind.x * scale * dance_w * 0.02;
    pos.z += sway_z * move_amp + wind.y * scale * dance_w * 0.02;

    let h_rw = flame_rw_1d(ph * 0.41 + 2.7, t, view.tuning.height_rw_rate);
    let breathe = 1.0 + h_rw * view.tuning.height_rw_amp * dance_w;
    pos.y *= view.tuning.flame_height * breathe;
    let tip_rw = flame_rw_1d(ph * 0.63 + 5.1, t, view.tuning.height_rw_rate * 1.25);
    pos.y += max(0.0, tip_rw) * tip_t * tip_t * scale * 0.008;

    let world_offset = vec3<f32>(
        pos.x * view.tuning.flame_width,
        pos.z * view.tuning.flame_width,
        pos.y,
    ) * scale;
    let world = anchor + world_offset;

    // Bent normal: blend mesh normal toward displacement direction for rim lighting.
    var nrm = mesh_nrm;
    nrm.x *= 1.0 - tip_t * view.tuning.taper_factor * 0.85;
    nrm.z *= 1.0 - tip_t * view.tuning.taper_factor * 0.85;
    let sway_bias = normalize(vec3<f32>(sway_x, 0.0, sway_z + 0.15));
    nrm = normalize(vec3<f32>(nrm.x, nrm.z, nrm.y) + sway_bias * dance_w * 0.08);

    var out: VsOut;
    out.clip_position = view.view_proj * vec4<f32>(world, 1.0);
    out.uv = mesh_uv;
    out.brightness = brightness;
    out.world_pos = world;
    out.world_nrm = nrm;
    out.dance_w = dance_w;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let height = 1.0 - in.uv.y;
    let view_dir = normalize(view.view_pos.xyz - in.world_pos);
    let vision_dot = dot(normalize(in.world_nrm), view_dir);
    let edge = 1.0 - vision_dot;

    var height_fade = smoothstep(0.0, 0.65, height);
    height_fade = pow(height_fade, 4.0);
    var alpha = height_fade - pow(max(vision_dot, 0.0), 3.0) * 0.1;
    alpha = mix(edge, alpha, pow(in.uv.y, 0.05));
    alpha *= mix(0.42, 1.0, pow(max(vision_dot, 0.0), 1.15));

    let t = globals.time;
    let flow_uv = vec2<f32>(
        in.uv.x * 4.0 + fbm(in.uv * 3.5 + vec2(t * 0.2, -t * 0.65)) * 0.22,
        in.uv.y * 5.5 - t * 1.6,
    );
    let flame_noise = fbm(flow_uv) * 0.5 + fbm(flow_uv * 1.9 + 2.7) * 0.5;
    alpha *= mix(0.78, 1.06, smoothstep(0.32, 0.68, flame_noise));
    alpha *= smoothstep(
        view.tuning.bottom_fade_y_start,
        view.tuning.bottom_fade_y_end,
        in.uv.y,
    );
    let micro_rw = flame_rw_1d(in.uv.y * 2.0 + in.uv.x * 0.4, t, view.tuning.micro_rw_rate);
    alpha *= 1.0 + micro_rw * view.tuning.bright_rw_amp * in.dance_w;
    alpha = clamp(alpha, 0.0, view.tuning.max_alpha);
    if (alpha < 0.004) {
        discard;
    }

    // Black-body column + chemiluminescence at wick; Godot rim on top.
    var col = candle_blackbody_rgb_srgb(height);
    col = mix(col, candle_chemiluminescence_rgb_srgb(), (1.0 - smoothstep(0.04, 0.28, in.uv.y)) * 0.88);
    let rim = mix(
        vec3<f32>(1.0, 1.0, 1.0),
        vec3<f32>(1.0, 0.75, 0.0),
        pow(edge, 1.0 - view.tuning.border_width) + 0.5 - in.uv.y,
    );
    col = mix(col, rim, edge * 0.45);

    let flick = 1.0
        + flame_rw_1d(in.uv.y * 1.5 + in.uv.x * 0.25, t, view.tuning.bright_rw_rate)
        * view.tuning.bright_rw_amp;
    let emission = col * max(vision_dot, 0.12) * view.tuning.emission_gain * in.brightness * flick;
    let rgb = emission / (1.0 + emission * 0.12);

    return vec4<f32>(rgb * alpha, alpha);
}
