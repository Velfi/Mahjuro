//! Helland black-body chromaticity — keep in sync with `shaders/blackbody.wgsl`.

/// Tanner Helland 2012 temperature → sRGB in `[0, 1]` (Kelvin clamped to 1000–40000).
pub fn blackbody_rgb_srgb(temp_k: f32) -> [f32; 3] {
    let temp = temp_k.clamp(1000.0, 40_000.0) / 100.0;

    let r = if temp <= 66.0 {
        255.0
    } else {
        let t = temp - 60.0;
        (329.698_727_446 * t.powf(-0.133_204_759_2)).clamp(0.0, 255.0)
    };

    let g = if temp <= 66.0 {
        let t = temp.max(1.0);
        (99.470_802_586_1 * t.ln() - 161.119_568_166_1).clamp(0.0, 255.0)
    } else if temp <= 190.0 {
        let t = temp - 60.0;
        (288.122_169_528_3 * t.powf(-0.075_514_849_2)).clamp(0.0, 255.0)
    } else {
        255.0
    };

    let b = if temp >= 66.0 {
        0.0
    } else if temp <= 19.0 {
        0.0
    } else if temp <= 32.0 {
        let t = temp - 10.0;
        (138.517_731_223_1 * t.max(1.0).ln() - 305.044_792_730_7).clamp(0.0, 255.0)
    } else {
        255.0
    };

    [r / 255.0, g / 255.0, b / 255.0]
}

#[inline]
fn srgb_channel_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Helland fit → linear RGB for punctual / PBR tints ([`crate::render::wgpu_renderer::PointLight`]).
pub fn blackbody_rgb_linear(temp_k: f32) -> [f32; 3] {
    let s = blackbody_rgb_srgb(temp_k);
    [
        srgb_channel_to_linear(s[0]),
        srgb_channel_to_linear(s[1]),
        srgb_channel_to_linear(s[2]),
    ]
}

/// Bright flame body — matches `CORE_K` in `candle_blackbody_rgb_srgb` (`blackbody.wgsl`).
pub const CANDLE_PUNCTUAL_TEMP_K: f32 = 2100.0;

/// `light_candle*` punctual tint: Planck wick chromaticity × debug [`crate::render::room_glb::RoomEnvLightingTune::candle_light_color_mul`].
pub fn candle_punctual_rgb_linear(color_mul: [f32; 3]) -> [f32; 3] {
    let base = blackbody_rgb_linear(CANDLE_PUNCTUAL_TEMP_K);
    [
        (base[0] * color_mul[0]).clamp(0.0, 1.0),
        (base[1] * color_mul[1]).clamp(0.0, 1.0),
        (base[2] * color_mul[2]).clamp(0.0, 1.0),
    ]
}
