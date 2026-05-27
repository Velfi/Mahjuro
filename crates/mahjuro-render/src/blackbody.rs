//! Helland black-body chromaticity — keep in sync with `shaders/blackbody.wgsl`.

/// Tanner Helland 2012 temperature → sRGB in `[0, 1]` (Kelvin clamped to 1000–40000).
pub fn blackbody_rgb_srgb(temp_k: f32) -> [f32; 3] {
    let temp = temp_k.clamp(1000.0, 40_000.0) / 100.0;

    let r = if temp <= 66.0 {
        255.0
    } else {
        let t = temp - 60.0;
        (329.698_73 * t.powf(-0.133_204_76)).clamp(0.0, 255.0)
    };

    let g = if temp <= 66.0 {
        let t = temp.max(1.0);
        (99.470_8 * t.ln() - 161.119_57).clamp(0.0, 255.0)
    } else if temp <= 190.0 {
        let t = temp - 60.0;
        (288.122_16 * t.powf(-0.075_514_846)).clamp(0.0, 255.0)
    } else {
        255.0
    };

    let b = if temp >= 66.0 || temp <= 19.0 {
        0.0
    } else if temp <= 32.0 {
        let t = temp - 10.0;
        (138.517_73 * t.max(1.0).ln() - 305.044_8).clamp(0.0, 255.0)
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

/// Helland fit → linear RGB for punctual / PBR tints ([`crate::wgpu_renderer::PointLight`]).
pub fn blackbody_rgb_linear(temp_k: f32) -> [f32; 3] {
    let s = blackbody_rgb_srgb(temp_k);
    [
        srgb_channel_to_linear(s[0]),
        srgb_channel_to_linear(s[1]),
        srgb_channel_to_linear(s[2]),
    ]
}
