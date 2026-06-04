// Fullscreen pass: linear HDR scene+bloom → display encoding.
// SDR (sRGB swapchain, mode=0): exposure × ACES fitted → linear out (surface applies sRGB encode).
// HDR swapchain (Rgba16Float, mode=1): linear × exposure pass-through so values > 1.0 reach
// the OS / display tonemapper (scRGB / EDR). Re-tonemapping on HDR crushes extended range.
// Journal prepass (mode=1): same linear path into offscreen float for book mesh sampling.
//
// VHS overlay (vhs_enabled = 1, swapchain path only): subtle CRT/tape look applied
// after ACES so the rest of the scene's exposure/color math is unchanged. Per-
// component amounts are read from the uniform — see `crate::game::tonemap_tuning`
// for the per-scene tuning that drives them.

struct TonemapParams {
    exposure: f32,
    /// 0 = ACES tonemap (SDR swapchain); 1 = linear × exposure (HDR swapchain);
    /// 2 = linear × exposure, no VHS (journal prepass only).
    mode: f32,
    /// 1.0 = run the VHS branch; 0.0 = skip everything below ACES.
    vhs_enabled: f32,
    /// Seconds since renderer start; drives VHS scanline phase only.
    time: f32,
    /// Per-effect amplitudes — set per scene by the debug Tonemap overlay.
    vhs_chromatic: f32,
    vhs_scanline: f32,
    vhs_grain: f32,
    vhs_vignette: f32,
    /// Monotonic present counter — re-rolls tape grain each frame without UV scroll.
    grain_frame: f32,
    /// User display-gamma slider; applied as pow(color, 1/gamma) at the end of
    /// the visible swapchain path. 1.0 (or the prepass) is a no-op.
    gamma: f32,
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

// Cheap hash → [0, 1).
fn vhs_rand(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// VHS tape noise: horizontally correlated luma speckle + stronger chroma (Cb/Cr),
// per-scanline AGC wobble, and whole-frame flutter. Masked into midtones so crushed
// blacks and paper whites stay clean like a well-tracked consumer deck.
fn apply_vhs_grain(
    color: vec3<f32>,
    frag_coord: vec2<f32>,
    frame: f32,
    time: f32,
    strength: f32,
) -> vec3<f32> {
    if strength <= 0.0 {
        return color;
    }

    let px = floor(frag_coord);
    let line_y = px.y;
    let f = floor(frame);

    // Tracking / AGC: noise floor shifts per scanline and slowly over time.
    let line_wobble = vhs_rand(vec2(line_y * 0.013, f * 7.1 + time * 0.37)) * 2.0 - 1.0;
    let line_gain = 0.82 + 0.36 * (line_wobble * 0.5 + 0.5);

    // Luminance bandwidth on VHS was ~3 MHz — noise clumps horizontally.
    let h_block = floor(px.x * 0.32);
    let h_seed = vec2(h_block + line_y * 0.061, f * 13.0 + time * 0.21);
    let fine = vhs_rand(px * vec2(0.78, 1.0) + vec2(f * 17.0, f * 31.0)) * 2.0 - 1.0;
    let coarse_luma = vhs_rand(h_seed) * 2.0 - 1.0;
    let luma_noise = (fine * 0.52 + coarse_luma * 0.48) * line_gain;

    // Chroma path is noisier than luma on consumer tape (Y/C separation bleed).
    let chroma_r = vhs_rand(px + vec2(f * 23.0 + 1.7, line_y * 0.19)) * 2.0 - 1.0;
    let chroma_b = vhs_rand(px.yx + vec2(line_y * 0.11, f * 29.0 + 4.3)) * 2.0 - 1.0;
    let grain = vec3<f32>(
        luma_noise + chroma_r * 0.44,
        luma_noise,
        luma_noise + chroma_b * 0.44,
    );

    // Whole-frame exposure flutter (capstan / dropout shimmer).
    let flicker = 0.88 + 0.14 * vhs_rand(vec2(f, f * 0.673 + time * 0.05));

    let lum = dot(color, vec3<f32>(0.299, 0.587, 0.114));
    let mask = smoothstep(0.025, 0.14, lum) * (1.0 - smoothstep(0.86, 0.97, lum));

    return color + grain * (strength * flicker * mask);
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

    if params.mode > 1.5 {
        // Journal prepass: keep linear; never apply VHS to a buffer the
        // book-page mesh resamples (we don't want compounded artifacts).
        return vec4<f32>(rgb, 1.0);
    }

    var color: vec3<f32>;
    if params.mode > 0.5 {
        // HDR swapchain: pass extended-range linear scRGB to the OS tonemapper.
        color = rgb;
    } else {
        color = aces_fitted(rgb);
    }

    if on {
        // Scanlines: ~360 dark ridges across the screen; amplitude per uniform.
        if params.vhs_scanline > 0.0 {
            let scan = sin(in.uv.y * 360.0 * 6.2831853) * 0.5 + 0.5;
            color = color * (1.0 - scan * params.vhs_scanline);
        }

        if params.vhs_grain > 0.0 {
            color = apply_vhs_grain(
                color,
                in.clip_pos.xy,
                params.grain_frame,
                t,
                params.vhs_grain,
            );
        }

        // Vignette: corner darkening (aspect-correct so ultrawide isn't weak in
        // the horizontal rails). Wider smoothstep than an inscribed circle in
        // raw UV so moderate slider values read clearly on real aspect ratios.
        if params.vhs_vignette > 0.0 {
            let dim_u = textureDimensions(hdr_tex, 0);
            let aspect = f32(dim_u.x) / max(f32(dim_u.y), 1.0);
            let p = (in.uv - vec2<f32>(0.5, 0.5)) * vec2<f32>(aspect, 1.0);
            let d = length(p);
            let vig = smoothstep(0.28, 0.82, d);
            color = color * (1.0 - vig * params.vhs_vignette);
        }
    }

    // User gamma slider. Matches the UI/legacy convention (pow(color, 1/gamma))
    // so the HDR scene path honors the Options slider, not just the 2D UI.
    let inv_g = 1.0 / max(params.gamma, 0.01);
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(inv_g));

    return vec4<f32>(color, 1.0);
}
