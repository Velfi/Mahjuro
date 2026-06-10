// Blit a hardware depth attachment into an R32Float texture (SSR history / probe readback).
// D3D12 integrated GPUs reject `copy_texture_to_buffer` from Depth32Float — sampling in a
// render pass is the portable path.

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
}

@group(0) @binding(0) var depth_tex: texture_depth_2d;

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
    return out;
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(i32(pos.x), i32(pos.y));
    let d = textureLoad(depth_tex, px, 0);
    return vec4<f32>(d, 0.0, 0.0, 1.0);
}
