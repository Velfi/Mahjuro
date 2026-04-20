// Upsample + additive composite for the half-res shooting-star cascade.
// The cascade fragment shader is expensive, so it renders into a half-res
// offscreen target in its own pass; this shader bilinearly samples that
// target and is drawn into the main scene pass with One/One additive
// blending, matching the old in-pass behaviour.

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 3.0,  1.0),
    );
    let p = pos[vid];
    var out: VsOut;
    out.clip_pos = vec4<f32>(p, 0.9999, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let rgb = textureSample(src_tex, src_samp, in.uv).rgb;
    return vec4<f32>(rgb, 0.0);
}
