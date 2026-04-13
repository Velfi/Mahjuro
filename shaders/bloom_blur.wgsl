struct BloomParams {
    data0: vec4<f32>, // zw = texel size
    data1: vec4<f32>, // xy = direction
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
    let texel = params.data0.zw;
    let dir = params.data1.xy * texel;
    var color = textureSample(src_tex, src_smp, in.uv).rgb * 0.227027;
    color += textureSample(src_tex, src_smp, in.uv + dir * 1.384615).rgb * 0.316216;
    color += textureSample(src_tex, src_smp, in.uv - dir * 1.384615).rgb * 0.316216;
    color += textureSample(src_tex, src_smp, in.uv + dir * 3.230769).rgb * 0.070270;
    color += textureSample(src_tex, src_smp, in.uv - dir * 3.230769).rgb * 0.070270;
    return vec4<f32>(color, 1.0);
}
