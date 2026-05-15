/// Image quad shader: renders a full-colour RGBA texture as a screen-space quad.
/// Instance `color` is multiplied with the texture sample (tint).

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var t_img: texture_2d<f32>;
@group(1) @binding(1) var s_img: sampler;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) _user: u32,
) -> VsOut {
    let x = rect.x + corner.x * rect.z;
    let y = rect.y + corner.y * rect.w;
    let nx = (x / globals.screen.x) * 2.0 - 1.0;
    let ny = 1.0 - (y / globals.screen.y) * 2.0;
    var out: VsOut;
    out.clip_pos = vec4<f32>(nx, ny, 0.0, 1.0);
    out.uv = corner;
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let tex = textureSample(t_img, s_img, in.uv);
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    let rgb = pow(tex.rgb * in.color.rgb, vec3<f32>(inv_g));
    return vec4<f32>(rgb, tex.a * in.color.a);
}
