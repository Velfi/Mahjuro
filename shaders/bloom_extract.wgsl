struct BloomParams {
    data0: vec4<f32>, // x = threshold
    data1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: BloomParams;
@group(0) @binding(1) var src_tex: texture_2d<f32>;
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
    let color = textureSample(src_tex, src_smp, in.uv).rgb;
    let threshold = params.data0.x;
    let lum = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let soft = smoothstep(threshold - 0.25, threshold + 0.35, lum);
    let bright = max(color - vec3<f32>(threshold), vec3<f32>(0.0));
    return vec4<f32>(max(bright * (0.8 + soft * 1.6), vec3<f32>(0.0)), 1.0);
}
