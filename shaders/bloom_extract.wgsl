struct BloomParams {
    // data0.x = tonemapped scene luminance threshold (display-referred)
    data0: vec4<f32>,
    // data1.z = linear HDR threshold (shop pre-pass, scene-linear)
    // data1.w = linear extract response scale
    data1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: BloomParams;
@group(0) @binding(1) var scene_tex: texture_2d<f32>;
@group(0) @binding(2) var linear_hdr_tex: texture_2d<f32>;
@group(0) @binding(3) var src_smp: sampler;

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
    let linear_hdr = textureSample(linear_hdr_tex, src_smp, in.uv).rgb;

    let th_s = params.data0.x;
    let lum_s = dot(scene, vec3<f32>(0.2126, 0.7152, 0.0722));
    let soft_s = smoothstep(th_s - 0.25, th_s + 0.35, lum_s);
    let bright_s = max(scene - vec3<f32>(th_s), vec3<f32>(0.0));
    let out_s = max(bright_s * (0.8 + soft_s * 1.6), vec3<f32>(0.0));

    let th_l = params.data1.z;
    let lum_l = dot(linear_hdr, vec3<f32>(0.2126, 0.7152, 0.0722));
    let soft_l = smoothstep(th_l - 0.04, th_l + 0.12, lum_l);
    let bright_l = max(linear_hdr - vec3<f32>(th_l), vec3<f32>(0.0));
    let out_l =
        max(bright_l * (0.8 + soft_l * 1.6), vec3<f32>(0.0)) * params.data1.w;

    // `linear_hdr_tex` is cleared each frame; only the shop redraw pass writes it.
    let use_linear = lum_l > 1e-5;
    let merged = select(out_s, max(out_s, out_l), use_linear);
    return vec4<f32>(merged, 1.0);
}
