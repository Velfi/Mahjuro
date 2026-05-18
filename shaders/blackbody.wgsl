// Black-body chromaticity approximations for emissive shaders.
//
// `blackbody_rgb_srgb` implements Tanner Helland's piecewise fit (1000–40000 K).
// Output is sRGB in [0, 1] — matches our flame additive path, which targets an
// sRGB surface without baking gamma twice.

/// Helland 2012 temperature → sRGB (Kelvin).
fn blackbody_rgb_srgb(temp_k: f32) -> vec3<f32> {
    let temp = clamp(temp_k, 1000.0, 40000.0) / 100.0;
    var r: f32;
    var g: f32;
    var b: f32;

    if (temp <= 66.0) {
        r = 255.0;
    } else {
        r = temp - 60.0;
        r = 329.698727446 * pow(r, -0.1332047592);
    }
    r = clamp(r, 0.0, 255.0);

    if (temp <= 66.0) {
        g = temp;
        g = 99.4708025861 * log(max(g, 1.0)) - 161.1195681661;
    } else if (temp <= 190.0) {
        g = temp - 60.0;
        g = 288.1221695283 * pow(g, -0.0755148492);
    } else {
        g = 255.0;
    }
    g = clamp(g, 0.0, 255.0);

    if (temp >= 66.0) {
        b = 0.0;
    } else if (temp <= 19.0) {
        b = 0.0;
    } else if (temp <= 32.0) {
        b = temp - 10.0;
        b = 138.5177312231 * log(max(b, 1.0)) - 305.0447927307;
    } else {
        b = 255.0;
    }
    b = clamp(b, 0.0, 255.0);

    return vec3<f32>(r, g, b) / 255.0;
}

/// Candle soot incandescence: tip (cool red) → body (orange/yellow) → core (white-yellow).
fn candle_blackbody_rgb_srgb(heat: f32) -> vec3<f32> {
    const TIP_K: f32 = 680.0;
    const CORE_K: f32 = 2100.0;
    let t = pow(clamp(heat, 0.0, 1.0), 0.78);
    return blackbody_rgb_srgb(mix(TIP_K, CORE_K, t));
}

/// CH*/C2* chemiluminescence at the wick (molecular emission, not Planck).
fn candle_chemiluminescence_rgb_srgb() -> vec3<f32> {
    // Band near 430 nm — blue-violet inner cone visible on real candles.
    return vec3<f32>(0.22, 0.48, 1.0);
}
