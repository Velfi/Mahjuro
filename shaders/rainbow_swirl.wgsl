// Shared pride-flag rainbow for main-menu moon/stars and the starfield overlay.
//
// `rainbow_swirl_rgb(uv, time)` returns saturated linear RGB (≈ six stairstepped
// Gilbert Baker stripes). Callers multiply by an emissive mask so black space
// stays black.

const RAINBOW_PRIDE_BAND_COUNT: f32 = 6.0;

/// Classic six-stripe pride flag colors (linear-ish, tuned for emissive HDR).
fn rainbow_pride_band_color(band: i32) -> vec3<f32> {
    switch band {
        case 0: { return vec3<f32>(1.05, 0.04, 0.04); } // red
        case 1: { return vec3<f32>(1.05, 0.48, 0.02); } // orange
        case 2: { return vec3<f32>(1.05, 0.90, 0.04); } // yellow
        case 3: { return vec3<f32>(0.04, 0.72, 0.18); } // green
        case 4: { return vec3<f32>(0.06, 0.22, 0.85); } // blue
        case 5: { return vec3<f32>(0.62, 0.06, 0.78); } // violet
        default: { return vec3<f32>(1.05, 0.04, 0.04); }
    }
}

/// Domain-warped scalar field → hard pride stripe index (no smooth blending).
fn rainbow_swirl_rgb(uv: vec2<f32>, time: f32) -> vec3<f32> {
    let t = time * 0.14;

    // Gentle whole-field rotation.
    let c0 = cos(t * 0.18);
    let s0 = sin(t * 0.18);
    var p = vec2<f32>(c0 * uv.x - s0 * uv.y, s0 * uv.x + c0 * uv.y);

    // Low-frequency domain warp so stripes curve slowly across the surface.
    let w1 = vec2<f32>(
        sin(p.y * 0.32 + t * 0.42 + sin(p.x * 0.24 - t * 0.22) * 0.55),
        cos(p.x * 0.28 - t * 0.32 + cos(p.y * 0.36 + t * 0.18) * 0.45),
    );
    let w2 = vec2<f32>(
        sin(p.x * 0.22 + t * 0.35 + w1.y * 0.48),
        cos(p.y * 0.20 - t * 0.45 + w1.x * 0.40),
    );
    p += (w1 * 0.30 + w2 * 0.20);

    // Scalar stripe coordinate — slow drift, then quantize into six flat bands.
    let field = p.x * 0.38 + p.y * 0.24 - t * 0.05;
    let stripe_pos = fract(field * 0.48 + 0.5);
    var band = i32(floor(stripe_pos * RAINBOW_PRIDE_BAND_COUNT));
    band = min(band, i32(RAINBOW_PRIDE_BAND_COUNT) - 1);

    return rainbow_pride_band_color(band);
}
