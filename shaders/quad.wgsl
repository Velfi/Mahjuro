struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) _user: u32,
) -> VsOut {
    let x = rect.x + corner.x * rect.z;
    let y = rect.y + corner.y * rect.w;
    let nx = (x / globals.screen.x) * 2.0 - 1.0;
    let ny = 1.0 - (y / globals.screen.y) * 2.0;
    var out: VsOut;
    out.clip_position = vec4<f32>(nx, ny, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Post-tonemap overlay passes use user `globals.gamma`; Pass A binds
    // `globals_scene_hdr` (gamma = 1.0) and defers to `tonemap_composite`.
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    let rgb = pow(in.color.rgb, vec3<f32>(inv_g));
    return vec4<f32>(rgb, in.color.a);
}
