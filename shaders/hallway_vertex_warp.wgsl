// Shared pick-blind hallway vertex warp for `room_glb`, `tile_3d`, and `shadow.wgsl`.
// Prepended at compile time (`embedded_wgsl`). Lit passes declare `hd` at @group(0) @binding(8);
// shadow depth uses the same `HallwayDistortion` layout at @group(1) @binding(0).

/// Scales wall-clock `time_pulse.x` for pick-blind hallway warp (1 = authored rate).
const HALLWAY_ANIM_TIME_SCALE: f32 = 0.5;
const HALLWAY_TAU: f32 = 6.283185307;
/// Standing corrugation frequency vs traveling (`ripple.y` waves along depth `u`).
const HALLWAY_RIPPLE_STAND_FREQ_RATIO: f32 = 2.37;
/// `pow(abs(side_c), …)` in world units (same idea as breathe) — avoids `side_n` dying when
/// env/walls AABB inflates `flags.w`.
const HALLWAY_RIPPLE_WALL_POWER: f32 = 1.28;
/// Floor Z for vertical barrel midline only (walls bow — floor/ceiling stay put).
const HALLWAY_BALLOON_FLOOR_Z: f32 = 0.08;
/// Minimum depth/height barrel factors so corridor ends still read curved (not flat panels).
const HALLWAY_BALLOON_DEPTH_FLOOR: f32 = 0.52;
const HALLWAY_BALLOON_VERT_FLOOR: f32 = 0.62;

struct HallwayDistortion {
    bow: vec4<f32>,
    breathe: vec4<f32>,
    ceiling: vec4<f32>,
    stretch: vec4<f32>,
    twist: vec4<f32>,
    mask: vec4<f32>,
    time_pulse: vec4<f32>,
    flags: vec4<f32>,
    /// x = lateral amplitude, y = wave count along `u`, z = travel speed, w = travel mix 0..1.
    ripple: vec4<f32>,
    /// x = pattern id, y = stripe freq, z = dark-stripe mul, w = phase (rad).
    wallpaper: vec4<f32>,
}

fn hallway_depth_axis_sel(idx: f32) -> vec3<f32> {
    if (idx < 0.5) {
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    if (idx < 1.5) {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    return vec3<f32>(0.0, 0.0, 1.0);
}

fn apply_hallway_distortion(world_in: vec3<f32>, h: HallwayDistortion) -> vec3<f32> {
    if (h.flags.x < 0.5) {
        return world_in;
    }
    let axis = hallway_depth_axis_sel(h.mask.x) * vec3<f32>(h.mask.y);
    let up = vec3<f32>(0.0, 0.0, 1.0);
    var lateral = cross(axis, up);
    let ll = length(lateral);
    if (ll < 1e-5) {
        lateral = vec3<f32>(1.0, 0.0, 0.0);
    } else {
        lateral = lateral / ll;
    }
    let depth = dot(world_in, axis);
    let d0 = h.mask.z;
    let d1 = h.mask.w;
    let span = max(d1 - d0, 1e-4);
    let u = clamp((depth - d0) / span, 0.0, 1.0);
    let t = h.time_pulse.x * HALLWAY_ANIM_TIME_SCALE;
    let drift = h.time_pulse.y * t;
    var sm0: f32;
    if (h.twist.z > 0.5) {
        // Bell curve: strongest mid-corridor, tapering toward both ends of the GLB span.
        let edge = 0.20;
        sm0 = smoothstep(0.0, edge, u) * smoothstep(1.0, 1.0 - edge, u);
    } else {
        sm0 = smoothstep(d0, d1, depth);
    }
    let g = h.flags.z;
    let pulse = 1.0 + h.time_pulse.w * sin(t * h.time_pulse.z + h.breathe.z + drift);
    let mask_f = sm0 * g * pulse;
    // Full boss blind sets `flags.y` = 1; warp uses 0.25× so amplitude matches prior tuning.
    let bp = h.flags.y * 0.25;

    let side_c = dot(world_in, lateral) - h.stretch.y;
    let lat_half = max(h.flags.w, 0.25);
    // Clamp so both walls reach full amp even when glTF root is off the lateral midline.
    let side_n = clamp(side_c / lat_half, -1.0, 1.0);

    var w = world_in;

    // Breathe: world-space offset from glTF root (symmetric); `breathe.w` falloff is in world units.
    let breathe_af = pow(max(abs(side_c), 1e-4), h.breathe.w);
    w = w + lateral * (sign(side_c) * breathe_af * h.breathe.x * sin(t * h.breathe.y + h.breathe.z + u * HALLWAY_TAU * 0.42 + drift) * mask_f);

    // Lateral wall ripple (traveling + standing); must match `tile_3d.wgsl`.
    if (h.ripple.x > 1e-6) {
        let wc = max(h.ripple.y, 0.5);
        let travel_ph = u * wc * HALLWAY_TAU - t * h.ripple.z + h.breathe.z;
        let stand_ph = u * wc * HALLWAY_TAU * HALLWAY_RIPPLE_STAND_FREQ_RATIO + h.breathe.z;
        let ripple_h = mix(sin(stand_ph), sin(travel_ph), clamp(h.ripple.w, 0.0, 1.0));
        let ripple_wall = pow(max(abs(side_c), 1e-4), HALLWAY_RIPPLE_WALL_POWER);
        w = w + lateral * (sign(side_c) * h.ripple.x * ripple_h * ripple_wall * mask_f);
    }

    // Wall barrel bow: `bow.w` × |side_c| ≈ lateral bulge in world units (~5–8% of wall distance at default tuning).
    if (h.bow.w > 1e-6) {
        let balloon_k = h.bow.w * mask_f * mix(1.0, 1.0 + bp * 0.65, sm0);
        let wall_gain = abs(side_c);
        let depth_barrel = max(sin(u * HALLWAY_TAU * 0.5), HALLWAY_BALLOON_DEPTH_FLOOR);
        let z_mid = mix(HALLWAY_BALLOON_FLOOR_Z, h.ceiling.y, 0.5);
        let z_half = max((h.ceiling.y - HALLWAY_BALLOON_FLOOR_Z) * 0.5, 0.18);
        let z_n = clamp((w.z - z_mid) / z_half, -1.0, 1.0);
        let vert_barrel = max(1.0 - z_n * z_n, HALLWAY_BALLOON_VERT_FLOOR);
        let bulge = balloon_k * wall_gain * depth_barrel * vert_barrel;
        w = w + lateral * (sign(side_c) * bulge);
    }

    let z_above = w.z - h.ceiling.y;
    if (z_above > 0.0) {
        let cp = 1.0 + h.ceiling.z * sin(t * h.ceiling.w);
        let sag = z_above * h.ceiling.x * cp * mask_f * mix(1.0, 1.0 + bp * 1.1, sm0);
        w = w - vec3<f32>(0.0, 0.0, 1.0) * sag;
    }

    let stretch_k = h.stretch.x * h.stretch.w * mask_f * mix(1.0, 1.0 + bp * 0.9, sm0);
    w = w + axis * (stretch_k * u);

    // Rigid spiral around corridor depth: every vertex at the same `u` shares one angle
    // (handedness from twist.w). Scale by `u` so rotation accumulates down the hall; no
    // `side_n` — that wrings left/right and shears centerline props (lamps, ceiling trim).
    let twist_dir = select(-1.0, 1.0, h.twist.w >= 0.0);
    let twist_env = pow(mask_f, h.twist.y) * mix(1.0, 1.0 + bp * 1.25, sm0);
    let ang = h.twist.x * twist_dir * u * twist_env;
    let proj_len = dot(w, axis);
    let p_perp = w - axis * proj_len;
    let c = cos(ang);
    let s = sin(ang);
    let p_rot = p_perp * c + cross(axis, p_perp) * s;
    w = axis * proj_len + p_rot;

    return w;
}

fn world_normal_after_distortion(
    world0: vec3<f32>,
    t_u: vec3<f32>,
    t_v: vec3<f32>,
    h: HallwayDistortion,
    n0: vec3<f32>,
) -> vec3<f32> {
    // Large enough epsilon vs breathe wavelengths to reduce numeric cancellation;
    // when the cross is tiny or points into the surface, fall back so punctual BRDF
    // does not go uniformly dark.
    let e = 0.06;
    let d_u = (apply_hallway_distortion(world0 + t_u * e, h) - apply_hallway_distortion(world0 - t_u * e, h)) / (2.0 * e);
    let d_v = (apply_hallway_distortion(world0 + t_v * e, h) - apply_hallway_distortion(world0 - t_v * e, h)) / (2.0 * e);
    let raw = cross(d_u, d_v);
    let len = length(raw);
    let nd = raw / max(len, 1e-10);
    let oriented = select(nd, -nd, dot(nd, n0) < 0.0);
    return select(n0, oriented, len > 1e-7);
}
