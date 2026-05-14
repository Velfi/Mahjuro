// Downsample full-res scene_color into half-res scene_prev (SSR history).
//
// Replaces the full-res `copy_texture_to_texture` color publish with a
// fullscreen-triangle blit so the per-frame `scene_prev` write stays
// inside ~3–4 MB instead of ~16 MB at 1080p (Steam Deck SD bandwidth
// budget). A single `textureSample` at the half-res output dimensions
// is a 2-tap bilinear box filter — slightly softer than a 4-tap
// average but fine for SSR which already loses detail along long
// reflection rays.

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
    out.clip_pos = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src_tex, src_samp, in.uv);
}
