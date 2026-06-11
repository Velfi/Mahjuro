//! Candle plume simulation — ported from digital-garden `plumeSim.ts`.
//!
//! Unit space uses `flame_height = 1.0`; world scale is applied in `flame.wgsl`.

pub const FLAME_BASE: f32 = 0.004;
pub const FLAME_HEIGHT_EXP: f32 = 0.82;
/// digital-garden reference plume height the turbulence amplitudes were authored for.
pub const DESIGN_FLAME_HEIGHT: f32 = 0.34;
/// Matches `PLUME_ANIM_SPEED` in `shaders/flame.wgsl`.
pub const PLUME_ANIM_SPEED: f32 = 2.0;
const FBM_OCTAVES: u32 = 4;
const FBM_LACUNARITY: f32 = 1.9;
const PLUME_TURB_SCALE: f32 = 7.5;
const PLUME_TURB_HEIGHT: f32 = 9.5;
const PLUME_TURB_FINE_SCALE: f32 = 14.0;
const PLUME_TURB_FINE_HEIGHT: f32 = 12.0;
const PLUME_WIND_EDDY_Y: f32 = 4.5;

#[inline]
pub fn plume_unit_scale(flame_height: f32) -> f32 {
    flame_height / DESIGN_FLAME_HEIGHT
}

#[derive(Clone, Copy, Debug)]
pub struct PlumeParams {
    pub time: f32,
    pub wind_x: f32,
    pub wind_y: f32,
    pub wind_strength: f32,
    pub turbulence: f32,
    pub flame_height: f32,
}

fn hash3(x: f32, y: f32, z: f32) -> f32 {
    let mut p = [x, y, z];
    p = [
        (p[0] * 0.3183099 + 0.1) % 1.0,
        (p[1] * 0.3183099 + 0.2) % 1.0,
        (p[2] * 0.3183099 + 0.3) % 1.0,
    ];
    p = [
        p[0] - p[0].floor(),
        p[1] - p[1].floor(),
        p[2] - p[2].floor(),
    ];
    let q = [
        p[0] + p[1] * (p[2] + 19.19),
        p[1] + p[2] * (p[0] + 19.19),
        p[2] + p[0] * (p[1] + 19.19),
    ];
    let d = (q[0] + q[1]) * q[2] * 127.1;
    d - d.floor()
}

fn fade3(t: f32, u: f32, v: f32) -> (f32, f32, f32) {
    let f = |x: f32| x * x * x * (x * (x * 6.0 - 15.0) + 10.0);
    (f(t), f(u), f(v))
}

fn grad_dot(h: f32, x: f32, y: f32, z: f32) -> f32 {
    let u = if h < 0.5 { y } else { x };
    let v = if h < 0.25 || h >= 0.75 { z } else { x };
    let su = if (h * 767.0).fract() < 0.5 { -1.0 } else { 1.0 };
    let sv = if (h * 313.0).fract() < 0.5 { -1.0 } else { 1.0 };
    su * u + sv * v
}

fn noise3(x: f32, y: f32, z: f32) -> f32 {
    let ix = x.floor();
    let iy = y.floor();
    let iz = z.floor();
    let fx = x - ix;
    let fy = y - iy;
    let fz = z - iz;
    let (wx, wy, wz) = fade3(fx, fy, fz);

    let corner = |ox: f32, oy: f32, oz: f32, px: f32, py: f32, pz: f32| {
        grad_dot(hash3(ix + ox, iy + oy, iz + oz), px, py, pz)
    };

    let n000 = corner(0.0, 0.0, 0.0, fx, fy, fz);
    let n100 = corner(1.0, 0.0, 0.0, fx - 1.0, fy, fz);
    let n010 = corner(0.0, 1.0, 0.0, fx, fy - 1.0, fz);
    let n110 = corner(1.0, 1.0, 0.0, fx - 1.0, fy - 1.0, fz);
    let n001 = corner(0.0, 0.0, 1.0, fx, fy, fz - 1.0);
    let n101 = corner(1.0, 0.0, 1.0, fx - 1.0, fy, fz - 1.0);
    let n011 = corner(0.0, 1.0, 1.0, fx, fy - 1.0, fz - 1.0);
    let n111 = corner(1.0, 1.0, 1.0, fx - 1.0, fy - 1.0, fz - 1.0);

    let nx00 =
        n000 + (n100 - n000) * wx + (n010 - n000) * wy + (n110 - n100 - n010 + n000) * wx * wy;
    let nx10 =
        n001 + (n101 - n001) * wx + (n011 - n001) * wy + (n111 - n101 - n011 + n001) * wx * wy;
    nx00 + (nx10 - nx00) * wz
}

pub fn fbm3(x: f32, y: f32, z: f32) -> f32 {
    let mut v = 0.0;
    let mut a = 0.5;
    let mut norm = 0.0;
    let mut px = x;
    let mut py = y;
    let mut pz = z;
    for _ in 0..FBM_OCTAVES {
        v += a * noise3(px, py, pz);
        norm += a;
        let nx = 0.8 * px - 0.6 * py + 0.2 * pz;
        let ny = 0.6 * px + 0.8 * py + 0.1 * pz;
        let nz = -0.2 * px + 0.1 * py + 0.9 * pz;
        px = nx * FBM_LACUNARITY + 13.7;
        py = ny * FBM_LACUNARITY + 7.1;
        pz = nz * FBM_LACUNARITY + 3.9;
        a *= 0.5;
    }
    v / norm.max(1e-6)
}

pub fn wick_curve_x(y: f32) -> f32 {
    const WICK_TOP: f32 = 0.012;
    let t = ((y + 0.006) / (WICK_TOP + 0.006)).clamp(0.0, 1.0);
    t * t * 0.025
}

pub fn plume_anchor(y: f32, params: &PlumeParams) -> (f32, f32) {
    let y01 = (y / params.flame_height).clamp(0.0, 1.0);
    let mut ax = wick_curve_x(y);
    let mut az = 0.0;

    if params.wind_strength <= 0.0 {
        return (ax, az);
    }

    let pin = base_pin_weight(y01);
    let bend = y01 * y01 * params.wind_strength * pin;
    let eddy = fbm3(
        params.wind_x * 2.0,
        y * PLUME_WIND_EDDY_Y - params.time * PLUME_ANIM_SPEED * 1.2,
        params.wind_y * 2.0,
    ) - 0.5;
    ax += params.wind_x * bend * 0.14 + eddy * 0.018 * params.turbulence * y01;
    az += params.wind_y * bend * 0.1 + eddy * 0.012 * params.turbulence * y01;

    (ax, az)
}

pub fn flame_height_at(y01: f32, flame_height: f32) -> f32 {
    FLAME_BASE + y01.powf(FLAME_HEIGHT_EXP) * flame_height
}

pub fn y01_from_height(y: f32, flame_height: f32) -> f32 {
    ((y - FLAME_BASE) / flame_height)
        .max(0.0)
        .powf(1.0 / FLAME_HEIGHT_EXP)
        .clamp(0.0, 1.0)
}

pub fn base_pin_weight(y01: f32) -> f32 {
    let t = y01.clamp(0.0, 1.0);
    1.0 - (t / 0.14).min(1.0).powf(1.6)
}

pub fn flame_envelope_width(y01: f32) -> f32 {
    let t = y01.clamp(0.0, 1.0);
    let foot_open = ((t / 0.11).min(1.0) * std::f32::consts::FRAC_PI_2).sin();
    let wick = (t / 0.06).min(1.0).powf(0.45);
    let tip = (1.0 - t).powf(0.82);
    let belly = (-((t - 0.3) / 0.28).powi(2)).exp();
    (0.002 + 0.034 * wick * tip * (0.34 + 0.66 * belly)) * foot_open
}

pub fn flame_local_radial(y01: f32, lx: f32, lz: f32) -> f32 {
    let width = flame_envelope_width(y01);
    (lx.hypot(lz) / width.max(0.0001)).min(1.0)
}

pub fn sim_displacement(
    lx: f32,
    ly: f32,
    lz: f32,
    y01: f32,
    params: &PlumeParams,
    phase_seed: f32,
) -> (f32, f32, f32) {
    let r = lx.hypot(lz).max(0.0001);
    let nx = lx / r;
    let nz = lz / r;
    let plume_scale = plume_unit_scale(params.flame_height);

    let anim = params.time * PLUME_ANIM_SPEED;
    let rising = anim * (1.4 + params.turbulence * 0.6);
    let turb = fbm3(
        lx * PLUME_TURB_SCALE + rising * 0.22 + phase_seed,
        (ly - FLAME_BASE) * PLUME_TURB_HEIGHT - rising * 0.55,
        lz * PLUME_TURB_SCALE + rising * 0.18 + phase_seed * 0.61,
    ) - 0.5;
    let turb_fine = fbm3(
        lx * PLUME_TURB_FINE_SCALE + rising * 0.38 + phase_seed * 1.7,
        (ly - FLAME_BASE) * PLUME_TURB_FINE_HEIGHT - rising * 0.92,
        lz * PLUME_TURB_FINE_SCALE + rising * 0.31 + phase_seed * 0.43,
    ) - 0.5;

    let breathe = (turb * 0.014 + turb_fine * 0.0025) * y01 * params.turbulence;
    let dy = (turb * 0.006 + turb_fine * 0.001) * params.turbulence * (0.3 + y01);
    let pin = base_pin_weight(y01);

    let (dx, dz, dy_out) = if params.wind_strength <= 0.0 {
        (nx * breathe * pin, nz * breathe * pin, dy * pin)
    } else {
        (
            (turb * 0.022 * params.turbulence * y01 + nx * breathe) * pin,
            (turb * 0.018 * params.turbulence * y01 + nz * breathe) * pin,
            dy * pin,
        )
    };

    (dx * plume_scale, dy_out * plume_scale, dz * plume_scale)
}

/// Fast turbulence sample for vertex brightness flicker (matches `flame.wgsl`).
pub fn plume_turbulence_flicker(
    lx: f32,
    ly: f32,
    lz: f32,
    y01: f32,
    params: &PlumeParams,
    phase_seed: f32,
) -> f32 {
    let anim = params.time * PLUME_ANIM_SPEED;
    let rising = anim * (1.4 + params.turbulence * 0.6);
    let turb = fbm3(
        lx * PLUME_TURB_SCALE + rising * 0.22 + phase_seed,
        (ly - FLAME_BASE) * PLUME_TURB_HEIGHT - rising * 0.55,
        lz * PLUME_TURB_SCALE + rising * 0.18 + phase_seed * 0.61,
    ) - 0.5;
    let dance = y01 * y01;
    1.0 + turb * 0.30 * dance * params.turbulence
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_opens_above_wick() {
        assert!(flame_envelope_width(0.05) > flame_envelope_width(0.0));
        assert!(flame_envelope_width(0.05) > 0.0);
    }

    #[test]
    fn y01_roundtrip_near_tip() {
        let h = 1.0;
        let y01 = 0.85;
        let y = flame_height_at(y01, h);
        let back = y01_from_height(y, h);
        assert!((back - y01).abs() < 0.02);
    }

    #[test]
    fn displacement_scales_with_plume_height() {
        let params = PlumeParams {
            time: 1.25,
            wind_x: 0.0,
            wind_y: 0.0,
            wind_strength: 0.45,
            turbulence: 0.75,
            flame_height: 0.34,
        };
        let short = sim_displacement(0.012, 0.05, 0.004, 0.08, &params, 0.0);
        let mut tall = params;
        tall.flame_height = 4.87;
        let tall_d = sim_displacement(0.012, 0.05, 0.004, 0.08, &tall, 0.0);
        let short_len = (short.0 * short.0 + short.1 * short.1 + short.2 * short.2).sqrt();
        let tall_len = (tall_d.0 * tall_d.0 + tall_d.1 * tall_d.1 + tall_d.2 * tall_d.2).sqrt();
        assert!(
            tall_len > short_len * 10.0,
            "short={short_len} tall={tall_len}"
        );
    }
}
