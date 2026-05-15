/// Text quad shader: renders a CPU-rasterized text bitmap as an alpha-masked
/// quad in screen space.  The `rect` instance attribute positions the quad in
/// pixel coordinates; the text bitmap is sampled as an alpha channel.
/// `user` low byte: 0=flat, 1=rainbow, 2=pulse, 3=shimmer, 4=gold tint.
/// bits 8–9: clockwise quarter-turns (0 = upright, 1 = 90°, …).

const TAU: f32 = 6.2831855;
const RAINBOW_TIME_SCALE: f32 = 0.22;
const RAINBOW_U_SCALE: f32 = 2.8;
const RAINBOW_V_SCALE: f32 = 0.4;
const RAINBOW_MIX: f32 = 0.82;
const PULSE_TIME_SCALE: f32 = 3.1;
const PULSE_ALPHA_TIME_SCALE: f32 = 2.7;
const SHIMMER_U_SCALE: f32 = 18.0;
const SHIMMER_TIME_SCALE: f32 = 4.0;
const SHIMMER_POWER: f32 = 8.0;
const SHIMMER_ADD: f32 = 0.35;

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var t_text: texture_2d<f32>;
@group(1) @binding(1) var s_text: sampler;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) effect_id: u32,
};

fn rotate_local(local: vec2<f32>, quarters: u32) -> vec2<f32> {
    switch quarters {
        case 1u: {
            return vec2<f32>(-local.y, local.x);
        }
        case 2u: {
            return vec2<f32>(-local.x, -local.y);
        }
        case 3u: {
            return vec2<f32>(local.y, -local.x);
        }
        default: {
            return local;
        }
    }
}

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) user: u32,
) -> VsOut {
    let quarters = (user >> 8u) & 0x3u;
    let local = vec2<f32>((corner.x - 0.5) * rect.z, (corner.y - 0.5) * rect.w);
    let rotated = rotate_local(local, quarters);
    let cx = rect.x + rect.z * 0.5;
    let cy = rect.y + rect.w * 0.5;
    let x = cx + rotated.x;
    let y = cy + rotated.y;
    let nx = (x / globals.screen.x) * 2.0 - 1.0;
    let ny = 1.0 - (y / globals.screen.y) * 2.0;
    var out: VsOut;
    out.clip_pos = vec4<f32>(nx, ny, 0.0, 1.0);
    out.uv = corner;
    out.color = color;
    out.effect_id = user & 0xFFu;
    return out;
}

fn apply_text_effect(
    base_rgb: vec3<f32>,
    a: f32,
    uv: vec2<f32>,
    effect_id: u32,
    t: f32,
) -> vec4<f32> {
    var rgb = base_rgb;
    var out_a = a;

    if effect_id == 1u {
        let phase = fract(t * RAINBOW_TIME_SCALE + uv.x * RAINBOW_U_SCALE + uv.y * RAINBOW_V_SCALE);
        let rainbow = vec3<f32>(
            sin(phase * TAU) * 0.5 + 0.5,
            sin(phase * TAU + 2.094) * 0.5 + 0.5,
            sin(phase * TAU + 4.189) * 0.5 + 0.5,
        );
        rgb = mix(rgb, rainbow, RAINBOW_MIX);
    } else if effect_id == 2u {
        let pulse = 0.55 + 0.45 * sin(t * PULSE_TIME_SCALE);
        rgb = rgb * pulse;
        out_a = a * (0.75 + 0.25 * sin(t * PULSE_ALPHA_TIME_SCALE + 0.5));
    } else if effect_id == 3u {
        let band = pow(max(0.0, sin(uv.x * SHIMMER_U_SCALE - t * SHIMMER_TIME_SCALE)), SHIMMER_POWER);
        rgb = rgb + vec3<f32>(band * SHIMMER_ADD);
    } else if effect_id == 4u {
        let warm = vec3<f32>(1.08, 0.92, 0.72);
        rgb = rgb * warm;
    }

    return vec4<f32>(rgb, out_a);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let samp_a = textureSample(t_text, s_text, in.uv).a;
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    let base_rgb = pow(in.color.rgb, vec3<f32>(inv_g));
    let tinted = apply_text_effect(
        base_rgb,
        in.color.a * samp_a,
        in.uv,
        in.effect_id,
        globals.time,
    );
    return vec4<f32>(tinted.rgb, tinted.a);
}
