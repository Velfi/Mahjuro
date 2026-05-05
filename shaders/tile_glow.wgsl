// Soft additive radial glow for selected tiles. Drawn as an oversized
// screen-space quad behind the tile so the warm gold light spills out
// past the tile silhouette and pulses gently with the candlelight rhythm.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    gamma: f32,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
) -> VsOut {
    let p = vec2<f32>(rect.x + corner.x * rect.z, rect.y + corner.y * rect.w);
    let ndc = vec2<f32>(
        p.x / globals.screen.x * 2.0 - 1.0,
        1.0 - p.y / globals.screen.y * 2.0,
    );
    var o: VsOut;
    o.clip_pos = vec4<f32>(ndc, 0.0, 1.0);
    o.uv = corner;
    o.color = color;
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Centered offset in [-1, 1] across the quad.
    let d = (in.uv - vec2<f32>(0.5, 0.5)) * 2.0;
    // Slight vertical squash so the glow reads as elliptical along the
    // tile's long axis (which is screen-vertical for hand tiles).
    let scaled = vec2<f32>(d.x, d.y * 0.85);
    let r = length(scaled);
    // Tight falloff: narrow rim, minimal spill — just enough to read as
    // a selection indicator without overpowering the tile face.
    let core = pow(max(1.0 - r, 0.0), 4.5) * 0.45;
    let spill = pow(max(1.0 - r, 0.0), 8.0) * 0.15;
    let falloff = core + spill;
    // Gentle ~1.5 Hz breathing pulse so selected tiles feel "alive".
    let pulse = 0.82 + 0.18 * sin(globals.time * 3.0);
    let strength = falloff * pulse * in.color.a;
    let inv_g = 1.0 / max(globals.gamma, 0.01);
    let rgb = pow(in.color.rgb * strength, vec3<f32>(inv_g));
    return vec4<f32>(rgb, strength);
}
