struct CompositeParams {
    // x = unused (bloom threshold, not used at composite)
    // y = bloom strength
    // z = 1/bloom_w
    // w = 1/bloom_h
    data0: vec4<f32>,
    // x = fisheye_strength (0 = off; positive = barrel distortion toward
    //     the centre, squashing the edges inward to sell a long hallway
    //     receding to the vanishing point)
    // y = vignette_strength (matched to fisheye so the corners fall off
    //     into shadow rather than showing the sampler clamp seam)
    // z/w reserved
    data1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: CompositeParams;
@group(0) @binding(1) var scene_tex: texture_2d<f32>;
@group(0) @binding(2) var bloom_tex: texture_2d<f32>;
@group(0) @binding(3) var src_smp: sampler;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    let p = pos[vid];
    var out: VsOut;
    out.clip_pos = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    return out;
}

// Barrel distortion: map the output UV to a source UV that pulls pixels
// from farther out on the scene texture as we approach the frame edges.
// Using a radial inverse mapping keeps straight lines through the centre
// straight and bends everything else toward the vanishing point.
//   d = (uv - 0.5) scaled to aspect-neutral space
//   r2 = d·d
//   scale = 1 + k*r2 + k2*r2^2
//   src_uv = 0.5 + d_aspect / scale
// Positive k bows the image inward (fisheye look).
fn barrel(uv: vec2<f32>, k: f32) -> vec2<f32> {
    if (k <= 0.0) { return uv; }
    // Aspect-correct so distortion is circularly symmetric on non-square
    // targets. The scene_tex is sampled with a CLAMP sampler so edge
    // pixels repeat rather than wrapping — corners past the mapped
    // domain show that clamp; we rely on the vignette to hide it.
    let dims = vec2<f32>(textureDimensions(scene_tex, 0));
    let aspect = dims.x / max(dims.y, 1.0);
    var d = uv - vec2<f32>(0.5, 0.5);
    d.x = d.x * aspect;
    let r2 = dot(d, d);
    // k2 is a second-order term that accelerates the bend near the
    // corners. Tuned so a fisheye_strength ~0.35 gives a clearly
    // curved frame without a black hole at centre.
    let k2 = k * 0.45;
    let scale = 1.0 + k * r2 + k2 * r2 * r2;
    var d2 = d / scale;
    d2.x = d2.x / aspect;
    return vec2<f32>(0.5, 0.5) + d2;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let k = params.data1.x;
    let vignette_strength = params.data1.y;

    let warped_uv = barrel(in.uv, k);

    // Clamp to [0,1] — wgpu's ClampToEdge addressing mode handles OOB,
    // but we also fade to black via the vignette so the clamp isn't
    // visible.
    let sample_uv = clamp(warped_uv, vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0));

    let scene = textureSample(scene_tex, src_smp, sample_uv).rgb;
    let bloom = textureSample(bloom_tex, src_smp, sample_uv).rgb;
    var color = scene + bloom * params.data0.y;

    // Radial vignette anchored to the warped UV so the darkest band
    // tracks the fisheye falloff. When fisheye is off, so is vignette.
    if (vignette_strength > 0.0) {
        let d = warped_uv - vec2<f32>(0.5, 0.5);
        let r = length(d) * 1.4142; // normalise so corners ≈ 1.0
        let v = clamp(1.0 - vignette_strength * r * r, 0.0, 1.0);
        color = color * v;
    }

    return vec4<f32>(color, 1.0);
}
