// Fullscreen pass: linear HDR scene+bloom → display encoding.
// SDR (sRGB swapchain): exposure × ACES fitted curve → linear out (surface applies sRGB encode).
// HDR / journal Rgba16Float: exposure scale only — leaves headroom for OS/display mapping.

struct TonemapParams {
    exposure: f32,
    /// 0 = ACES tonemap for SDR surfaces; 1 = linear × exposure only (HDR float output).
    mode: f32,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> params: TonemapParams;
@group(0) @binding(1) var hdr_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    let p = pos[vid];
    var out: VsOut;
    out.clip_pos = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    return out;
}

// Stephen Hill / Narkowicz ACES fitted — compact, neutral.
fn aces_fitted(color: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp(
        (color * (a * color + b)) / (color * (c * color + d) + e),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let rgb = textureSample(hdr_tex, samp, in.uv).rgb * params.exposure;
    if params.mode > 0.5 {
        return vec4<f32>(rgb, 1.0);
    }
    return vec4<f32>(aces_fitted(rgb), 1.0);
}
