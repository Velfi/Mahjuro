// Smoke composite pass — bilateral (depth-aware) upsampling.
//
// Samples the reduced-resolution offscreen volumetric raymarch target and
// composites it onto the swap chain using premultiplied alpha. Instead of
// plain bilinear filtering we perform a 4-tap bilateral filter: each of
// the bilinear neighbours is weighted by how close its depth is to the
// centre pixel's full-resolution depth. This prevents smoke from bleeding
// across sharp foreground edges (e.g. tiles, wooden box) which was the
// main source of artifacts at 1/4 and 1/8 resolution.

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(0) @binding(2) var depth_tex: texture_depth_2d;

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
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let smoke_dims = vec2<f32>(textureDimensions(src_tex));
    let depth_dims = vec2<i32>(textureDimensions(depth_tex));

    // Full-resolution pixel coordinate for the depth lookup.
    let full_pix = vec2<i32>(in.clip_pos.xy);
    let centre_depth = textureLoad(depth_tex, clamp(full_pix, vec2<i32>(0), depth_dims - 1), 0);

    // If the centre depth is at the far plane there's no foreground edge
    // to protect — plain bilinear is fine and cheaper.
    if centre_depth > 0.9999 {
        return textureSample(src_tex, src_samp, in.uv);
    }

    // Coordinate in the low-res smoke texture (fractional).
    let smoke_coord = in.uv * smoke_dims - 0.5;
    let base = vec2<i32>(floor(smoke_coord));
    let frac = smoke_coord - vec2<f32>(base);

    // Scale factor from smoke texels to full-res pixels.
    let scale = vec2<f32>(depth_dims) / smoke_dims;

    // Depth threshold for bilateral rejection — scaled by the resolution
    // ratio so coarser settings get a proportionally wider depth band.
    let depth_sigma = 0.002 * max(scale.x, scale.y);

    var total_color = vec4<f32>(0.0);
    var total_weight = 0.0;

    // Bilinear weights for the 2x2 neighbourhood.
    let bw = array<f32, 4>(
        (1.0 - frac.x) * (1.0 - frac.y),
        frac.x          * (1.0 - frac.y),
        (1.0 - frac.x) * frac.y,
        frac.x          * frac.y,
    );
    let offsets = array<vec2<i32>, 4>(
        vec2<i32>(0, 0),
        vec2<i32>(1, 0),
        vec2<i32>(0, 1),
        vec2<i32>(1, 1),
    );

    for (var i = 0; i < 4; i++) {
        let tap = clamp(base + offsets[i], vec2<i32>(0), vec2<i32>(smoke_dims) - 1);

        // Sample the smoke colour at this texel (point-sampled).
        let color = textureLoad(src_tex, tap, 0);

        // Corresponding full-res depth at the centre of this low-res texel.
        let tap_full = vec2<i32>((vec2<f32>(tap) + 0.5) * scale);
        let tap_depth = textureLoad(
            depth_tex,
            clamp(tap_full, vec2<i32>(0), depth_dims - 1),
            0,
        );

        // Bilateral weight: exponential falloff with depth difference.
        let dd = abs(centre_depth - tap_depth);
        let depth_w = exp(-dd * dd / (depth_sigma * depth_sigma));

        let w = bw[i] * depth_w;
        total_color += color * w;
        total_weight += w;
    }

    if total_weight < 0.0001 {
        // All neighbours rejected — fall back to nearest.
        return textureLoad(src_tex, clamp(vec2<i32>(smoke_coord + 0.5), vec2<i32>(0), vec2<i32>(smoke_dims) - 1), 0);
    }

    return total_color / total_weight;
}
