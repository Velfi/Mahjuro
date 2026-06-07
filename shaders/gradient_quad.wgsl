// Alpha-gradient quad. Same vertex layout as `quad.wgsl` plus a per-instance
// `feather` vec4: x=horizontal edge softness, z=vertical edge softness
// (when z≈0, both axes use x), y=radial vs. axial (0=axial-rect, 1=radial),
// w=reserved.
//
// axial mode (y=0): alpha smoothly ramps from 0 at the outer edge to full
// `color.a` at an inner rect inset by `feather.x` on each side. Reads as a
// "dark panel behind UI" that feathers into its surroundings.
//
// radial mode (y=1): alpha is smoothstep from center (full) to edge (zero),
// producing a soft dark halo. Blends 0→1 between axial and radial.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Centered UV in [-1, 1] across the instance rect. Used by the fragment
    // shader to compute falloff independently of screen coordinates.
    @location(1) uv: vec2<f32>,
    @location(2) feather: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) feather: vec4<f32>,
) -> VsOut {
    let x = rect.x + corner.x * rect.z;
    let y = rect.y + corner.y * rect.w;
    let nx = (x / globals.screen.x) * 2.0 - 1.0;
    let ny = 1.0 - (y / globals.screen.y) * 2.0;
    var out: VsOut;
    out.clip_position = vec4<f32>(nx, ny, 0.0, 1.0);
    out.color = color;
    // `corner` is in [0,1]; map to centered [-1,1].
    out.uv = corner * 2.0 - vec2<f32>(1.0, 1.0);
    out.feather = feather;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let edge_x = clamp(in.feather.x, 0.001, 0.999);
    let edge_y_raw = in.feather.z;
    let edge_y = clamp(
        select(edge_x, edge_y_raw, edge_y_raw > 0.001),
        0.001,
        0.999,
    );

    // Axial (rect): alpha = 1 in center, ramps to 0 within `edge` of each
    // edge. abs(uv) is the normalized distance to center along each axis.
    let ax = 1.0 - smoothstep(1.0 - edge_x, 1.0, abs(in.uv.x));
    let ay = 1.0 - smoothstep(1.0 - edge_y, 1.0, abs(in.uv.y));
    let axial = ax * ay;

    // Radial: alpha smoothstepped from center to corner distance.
    let r = length(in.uv);
    let edge_r = max(edge_x, edge_y);
    let radial = 1.0 - smoothstep(1.0 - edge_r, 1.0, r);

    let mode = clamp(in.feather.y, 0.0, 1.0);
    let falloff = mix(axial, radial, mode);

    let a = in.color.a * falloff;
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    let rgb = pow(in.color.rgb, vec3<f32>(inv_g));
    return vec4<f32>(rgb, a);
}
