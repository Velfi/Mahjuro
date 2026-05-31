struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) depth: f32,
    @location(1) alpha: f32,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) user: u32,
) -> VsOut {
    let x = rect.x + corner.x * rect.z;
    let y = rect.y + corner.y * rect.w;
    let nx = (x / globals.screen.x) * 2.0 - 1.0;
    let ny = 1.0 - (y / globals.screen.y) * 2.0;
    let depth = clamp(bitcast<f32>(user), 0.0, 1.0);
    var out: VsOut;
    out.clip_position = vec4<f32>(nx, ny, depth, 1.0);
    out.depth = depth;
    out.alpha = color.a;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let t = clamp(in.depth, 0.0, 1.0);
    let near_col = vec3<f32>(0.95, 0.25, 0.20);
    let far_col = vec3<f32>(0.15, 0.40, 1.00);
    let rgb = mix(near_col, far_col, t);
    return vec4<f32>(rgb, in.alpha);
}
