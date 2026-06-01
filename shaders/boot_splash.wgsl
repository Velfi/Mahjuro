/// Minimal boot splash: solid quads + single-channel SDF label + RGBA logo.
/// `user` instance field: 0 = solid fill, 1 = SDF text sample, 2 = logo texture.

struct BootGlobals {
    screen: vec2<f32>,
    gamma: f32,
    spread: f32,
    msdf_uv_min: vec2<f32>,
    msdf_uv_max: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: BootGlobals;
@group(1) @binding(0) var t_msdf: texture_2d<f32>;
@group(1) @binding(1) var s_msdf: sampler;
@group(1) @binding(2) var t_logo: texture_2d<f32>;
@group(1) @binding(3) var s_logo: sampler;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) mode: u32,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) user: u32,
) -> VsOut {
    let x = rect.x + corner.x * rect.z;
    let y = rect.y + corner.y * rect.w;
    let nx = (x / globals.screen.x) * 2.0 - 1.0;
    let ny = 1.0 - (y / globals.screen.y) * 2.0;
    var out: VsOut;
    out.clip_pos = vec4<f32>(nx, ny, 0.0, 1.0);
    out.color = color;
    out.uv = corner;
    out.mode = user;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    let rgb = pow(in.color.rgb, vec3<f32>(inv_g));

    if in.mode == 0u {
        return vec4<f32>(rgb, in.color.a);
    }

    if in.mode == 2u {
        let sample = textureSample(t_logo, s_logo, in.uv);
        let lum = dot(sample.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        let a = lum * in.color.a;
        if a < 0.004 {
            discard;
        }
        return vec4<f32>(vec3<f32>(1.0), a);
    }

    let atlas_uv = mix(globals.msdf_uv_min, globals.msdf_uv_max, in.uv);
    let d = textureSample(t_msdf, s_msdf, atlas_uv).r;
    let w = globals.spread / max(textureDimensions(t_msdf).x, 1u);
    let alpha = smoothstep(0.5 - w, 0.5 + w, d) * in.color.a;
    if alpha < 0.004 {
        discard;
    }
    return vec4<f32>(rgb, alpha);
}
