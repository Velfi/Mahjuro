/// Tile quad shader: renders a mahjong tile as a rounded rectangle with
/// an ivory face, subtle bamboo-coloured border, and slight bevel.

struct Globals {
    screen: vec2<f32>,
    time: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,          // 0..1 within the tile rect
    @location(1) rect_size: vec2<f32>,    // pixel dimensions of the tile
    @location(2) light: vec4<f32>,        // rgb = light color, a = intensity (0 = off)
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,       // unit quad corner (0..1)
    @location(1) rect: vec4<f32>,         // [x, y, w, h] in pixels
    @location(2) color: vec4<f32>,        // rgb = hint light color, a = intensity; rgb=0 means no light
) -> VsOut {
    let x = rect.x + corner.x * rect.z;
    let y = rect.y + corner.y * rect.w;
    let nx = (x / globals.screen.x) * 2.0 - 1.0;
    let ny = 1.0 - (y / globals.screen.y) * 2.0;
    var out: VsOut;
    out.clip_pos = vec4<f32>(nx, ny, 0.0, 1.0);
    out.uv = corner;
    out.rect_size = vec2<f32>(rect.z, rect.w);
    out.light = color;
    return out;
}

/// Signed distance to a rounded rectangle centered at the origin.
fn sd_rounded_rect(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let size = in.rect_size;
    let half = size * 0.5;

    // Position relative to tile center, in pixels.
    let p = (in.uv - 0.5) * size;

    // Corner radius: ~8% of the shorter dimension.
    let radius = min(size.x, size.y) * 0.08;

    // SDF for the outer tile shape.
    let d = sd_rounded_rect(p, half, radius);

    // Anti-aliased edge: discard outside, smooth at boundary.
    let aa = 1.0;  // 1 pixel feather
    let outer_alpha = 1.0 - smoothstep(-aa, aa, d);
    if outer_alpha < 0.01 {
        discard;
    }

    // Border: the bamboo-coloured edge frames the ivory face.
    let border_width = min(size.x, size.y) * 0.06;
    let inner_d = sd_rounded_rect(p, half - vec2<f32>(border_width), radius * 0.6);

    let ivory = vec3<f32>(0.95, 0.92, 0.85);
    let bamboo = vec3<f32>(0.60, 0.48, 0.28);
    let bamboo_dark = vec3<f32>(0.45, 0.35, 0.20);

    // Slight vertical gradient on the border for a rounded/bevel look.
    let bevel_t = smoothstep(-half.y, half.y, p.y);
    let border_color = mix(bamboo, bamboo_dark, bevel_t);

    // Blend face vs border.
    let face_t = 1.0 - smoothstep(-aa, aa, inner_d);
    var rgb = mix(border_color, ivory, face_t);

    // Directional light hint: when light.rgb > 0, tint the face with a diagonal wash.
    let light_sum = in.light.r + in.light.g + in.light.b;
    if light_sum > 0.01 && face_t > 0.01 {
        // Light direction: upper-right to lower-left.
        let dir = normalize(vec2<f32>(0.7, -0.5));
        let diagonal = dot(in.uv - 0.5, dir);
        // Soft falloff across the tile face.
        let falloff = smoothstep(-0.2, 0.5, diagonal);
        let intensity = falloff * in.light.a * face_t;
        // Tint toward the light color (mix, not add — ivory is too bright for additive).
        let tint = vec3<f32>(0.7, 1.0, 0.75); // pale green target
        rgb = mix(rgb, tint, intensity);
    }

    return vec4<f32>(rgb, outer_alpha);
}
