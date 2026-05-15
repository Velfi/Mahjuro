// Fullscreen pass: linear HDR scene+bloom → display encoding.
// SDR (sRGB swapchain): exposure × ACES fitted → linear out (surface applies sRGB encode).
// HDR swapchain (Rgba16Float): same ACES fitted as SDR (linear out; OS maps extended range).
// Journal prepass (mode=1): linear × exposure into offscreen float for book mesh sampling.
//
// VHS overlay (vhs_enabled = 1, swapchain path only): subtle CRT/tape look applied
// after ACES so the rest of the scene's exposure/color math is unchanged. Per-
// component amounts are read from the uniform — see `crate::game::tonemap_tuning`
// for the per-scene tuning that drives them.

struct TonemapParams {
    exposure: f32,
    /// 0 = ACES tonemap (swapchain SDR or HDR); 1 = linear × exposure (journal prepass only).
    mode: f32,
    /// 1.0 = run the VHS branch; 0.0 = skip everything below ACES.
    vhs_enabled: f32,
    /// Seconds since renderer start; drives noise animation.
    time: f32,
    /// Per-effect amplitudes — set per scene by the debug Tonemap overlay.
    vhs_chromatic: f32,
    vhs_scanline: f32,
    vhs_grain: f32,
    vhs_vignette: f32,
}

@group(0) @binding(0) var<uniform> params: TonemapParams;
@group(0) @binding(1) var hdr_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

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

fn aces_fitted(color: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp(
        (color * (a * color + b)) / (color * (c * color + d) + e),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
}

// Cheap hash → [0, 1). Same canonical 12.9898 / 78.233 sin trick used in
// `golden_dust.wgsl` etc.; resolution-independent because we feed it pixel-ish
// coords (uv * synthetic CRT line count) plus a time term so noise re-rolls.
fn vhs_rand(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let on = params.vhs_enabled > 0.5;
    let t = params.time;

    var rgb: vec3<f32>;
    if on && params.vhs_chromatic > 0.0 {
        // Split R/B by vhs_chromatic (in UV).
        let ca = params.vhs_chromatic;
        rgb.r = textureSample(hdr_tex, samp, in.uv + vec2<f32>(ca, 0.0)).r;
        rgb.g = textureSample(hdr_tex, samp, in.uv).g;
        rgb.b = textureSample(hdr_tex, samp, in.uv - vec2<f32>(ca, 0.0)).b;
    } else {
        rgb = textureSample(hdr_tex, samp, in.uv).rgb;
    }
    rgb = rgb * params.exposure;

    if params.mode > 0.5 {
        // Journal prepass: keep linear; never apply VHS to a buffer the
        // book-page mesh resamples (we don't want compounded artifacts).
        return vec4<f32>(rgb, 1.0);
    }

    var color = aces_fitted(rgb);

    if on {
        // Scanlines: ~360 dark ridges across the screen; amplitude per uniform.
        if params.vhs_scanline > 0.0 {
            let scan = sin(in.uv.y * 360.0 * 6.2831853) * 0.5 + 0.5;
            color = color * (1.0 - scan * params.vhs_scanline);
        }

        // Static grain: small luminance jitter, animated. Signed around 0.
        if params.vhs_grain > 0.0 {
            let n = vhs_rand(in.uv * vec2<f32>(1024.0, 700.0) + vec2<f32>(t, t * 1.37));
            color = color + vec3<f32>((n - 0.5) * params.vhs_grain);
        }

        // Vignette: gentle corner darkening with smoothstep falloff.
        if params.vhs_vignette > 0.0 {
            let d = distance(in.uv, vec2<f32>(0.5, 0.5));
            let vig = smoothstep(0.45, 0.95, d);
            color = color * (1.0 - vig * params.vhs_vignette);
        }
    }

    return vec4<f32>(color, 1.0);
}
