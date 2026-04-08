// Smoke composite pass.
//
// Samples the half-resolution (or quarter / eighth / native) offscreen
// volumetric raymarch target with bilinear filtering and blends it onto
// the swap chain using premultiplied alpha. The fluid shader writes
// premultiplied colour into the offscreen target so the only thing this
// pass needs is a `textureSample` and the right blend state — the
// upsample is the bilinear sampler.
//
// Future enhancement: nearest-depth upsampling to avoid silhouette
// bleeding when smoke passes behind a sharp foreground object. The
// candle plume case rarely triggers this so plain bilinear is fine for
// now.

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    // Standard fullscreen triangle.
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 3.0,  1.0),
    );
    let p = pos[vid];
    var out: VsOut;
    out.clip_pos = vec4<f32>(p, 0.0, 1.0);
    // Map clip-space to UV. Y is flipped because the offscreen target
    // shares the same top-down convention as the swap chain.
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src_tex, src_samp, in.uv);
}
