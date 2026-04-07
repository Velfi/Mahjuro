/// Fluid render shader: samples the density texture and outputs alpha-blended colored smoke.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    _pad: f32,
};

struct FluidRenderParams {
    max_alpha: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var t_density: texture_2d<f32>;
@group(1) @binding(1) var s_density: sampler;
@group(1) @binding(2) var<uniform> render_params: FluidRenderParams;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
) -> VsOut {
    let x = rect.x + corner.x * rect.z;
    let y = rect.y + corner.y * rect.w;
    let nx = (x / globals.screen.x) * 2.0 - 1.0;
    let ny = 1.0 - (y / globals.screen.y) * 2.0;
    var out: VsOut;
    out.clip_pos = vec4<f32>(nx, ny, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = textureSample(t_density, s_density, in.uv);
    // Perceived luminance for alpha calculation.
    let lum = d.r * 0.299 + d.g * 0.587 + d.b * 0.114;
    let alpha = clamp(lum * 2.0, 0.0, 1.0) * render_params.max_alpha;
    // Output premultiplied alpha.
    return vec4<f32>(d.rgb * alpha, alpha);
}
