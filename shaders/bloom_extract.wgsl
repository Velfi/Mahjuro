// Pull bright pixels out of the linear HDR `scene_color` for the bloom blur
// ping-pong. All scene shaders now write linear HDR — see
// `tonemap_composite.wgsl` for the single ACES + sRGB encode pass.
struct BloomParams {
    // data0.x = linear HDR luminance threshold
    // data0.y = bloom composite strength (consumed by `bloom_composite.wgsl`)
    data0: vec4<f32>,
    // data1.w = extract response scale (multiplier on the above-threshold HDR)
    data1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: BloomParams;
@group(0) @binding(1) var scene_tex: texture_2d<f32>;
@group(0) @binding(2) var src_smp: sampler;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let scene = textureSample(scene_tex, src_smp, in.uv).rgb;
    let th = params.data0.x;
    let lum = dot(scene, vec3<f32>(0.2126, 0.7152, 0.0722));
    // Wide knee: matches the legacy `scene_tex` branch when the threshold is
    // ~1.0 scene-linear (only strong HDR peaks bloom; small sources stay sharp).
    let soft = smoothstep(th - 0.25, th + 0.35, lum);
    let bright = max(scene - vec3<f32>(th), vec3<f32>(0.0));
    let out_rgb =
        max(bright * (0.8 + soft * 1.6), vec3<f32>(0.0)) * params.data1.w;
    return vec4<f32>(out_rgb, 1.0);
}
