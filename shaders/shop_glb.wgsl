// shop.glb environment — glTF-style punctual lights + metallic–roughness + ACES (fitted).
// Separate from `tile_3d.wgsl` (candle pools + artistic lambert floor).
//
// Uniform hacks (same `CameraUniform` layout as tiles; shop writer only):
// - `tile_seed`     = linear HDR exposure multiplier before tonemap
// - `decal_atlas_uv.x` = ambient scale (0 = punctual-only interior)
// - `decal_atlas_uv.y` = 1/world_scale — inverse-square uses document-space distance (glTF units)
// - `decal_atlas_uv.z` = glTF emissive strength multiplier (see `SHOP_GLTF_EMISSIVE_SCALE`)
//
// Point / spot `pos.w` = max light distance in **world units** (`KHR_lights_punctual` range),
// or `0` for infinite range (pure inverse-square with a minimum distance clamp).

const PI: f32 = 3.14159265358979323846;
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

struct CameraUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    base_color_factor: vec4<f32>,
    cam_pos: vec3<f32>,
    tile_seed: f32,
    decal_atlas_uv: vec4<f32>,
    /// Must match `wgpu_renderer::CameraUniform::hdr_tonemap` (layout parity with `lit_mesh` felt row).
    /// `w` — when > 0.5, fragment outputs **linear HDR** (`hdr` before ACES/γ) for a bloom pre-pass;
    /// normal shop draws keep this at 0.
    hdr_tonemap: vec4<f32>,
};

@group(0) @binding(0) var<uniform> cam: CameraUniform;
@group(0) @binding(1) var base_color: texture_2d<f32>;
@group(0) @binding(2) var base_sampler: sampler;
@group(0) @binding(3) var decal_tex: texture_2d<f32>;
@group(0) @binding(4) var normal_tex: texture_2d<f32>;

struct GltfPbrUniform {
    metallic_factor: f32,
    roughness_factor: f32,
    alpha_cutoff: f32,
    _pad0: f32,
    emissive_factor: vec4<f32>,
    alpha_mode: u32,
    _pad1_0: u32,
    _pad1_1: u32,
    _pad1_2: u32,
}

@group(0) @binding(5) var<uniform> pbr: GltfPbrUniform;
@group(0) @binding(6) var metallic_roughness_tex: texture_2d<f32>;
@group(0) @binding(7) var emissive_tex: texture_2d<f32>;

/// Pick-blind hallway warp (disabled when `flags.x` = 0 — zero buffer for shop/tiles).
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
};
@group(0) @binding(8) var<uniform> hd: HallwayDistortion;

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

    // Wall barrel bow (`bow.w` world units). `wall_dist = |side_c|` — not normalized by `flags.w`.
    if (h.bow.w > 1e-6) {
        let balloon_k = h.bow.w * mask_f * mix(1.0, 1.0 + bp * 0.65, sm0);
        let wall_dist = abs(side_c);
        let on_wall = smoothstep(0.12, 1.35, wall_dist);
        let depth_barrel = max(sin(u * HALLWAY_TAU * 0.5), 0.22);
        let z_mid = mix(HALLWAY_BALLOON_FLOOR_Z, h.ceiling.y, 0.5);
        let z_half = max((h.ceiling.y - HALLWAY_BALLOON_FLOOR_Z) * 0.5, 0.18);
        let z_n = clamp((w.z - z_mid) / z_half, -1.0, 1.0);
        let vert_barrel = max(1.0 - z_n * z_n, 0.4);
        let bulge = balloon_k * on_wall * depth_barrel * vert_barrel;
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

    let twist_dir = select(-1.0, 1.0, h.twist.w >= 0.0);
    let ang = h.twist.x * twist_dir * side_n * pow(mask_f, h.twist.y) * mix(1.0, 1.0 + bp * 1.25, sm0);
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

struct PointLight {
    pos: vec4<f32>,
    color: vec4<f32>,
    params: vec4<f32>,
};

struct PointLights {
    count: vec4<u32>,
    extras: vec4<f32>,
    lights: array<PointLight, 16>,
};

@group(1) @binding(0) var<uniform> lights: PointLights;

struct SpotLight {
    pos: vec4<f32>,
    dir: vec4<f32>,
    color: vec4<f32>,
    params: vec4<f32>,
};
struct SpotLights {
    count: vec4<u32>,
    lights: array<SpotLight, 8>,
};
@group(3) @binding(0) var<uniform> spot_lights: SpotLights;

struct ShadowGlobals {
    light_view_proj: mat4x4<f32>,
    params: vec4<f32>,
};
@group(2) @binding(0) var<uniform> shadow_globals: ShadowGlobals;
@group(2) @binding(1) var shadow_map: texture_depth_2d;
@group(2) @binding(2) var shadow_samp: sampler_comparison;

fn sample_shadow_visibility(world_pos: vec3<f32>) -> f32 {
    if (shadow_globals.params.x < 0.5) {
        return 1.0;
    }
    let lp = shadow_globals.light_view_proj * vec4<f32>(world_pos, 1.0);
    let proj = lp.xyz / lp.w;
    let uv = vec2<f32>(proj.x * 0.5 + 0.5, proj.y * -0.5 + 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || proj.z < 0.0 || proj.z > 1.0) {
        return 1.0;
    }
    let bias = shadow_globals.params.y;
    let depth_ref = proj.z - bias;
    let texel = shadow_globals.params.z;
    var sum = 0.0;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let off = vec2<f32>(f32(dx), f32(dy)) * texel;
            sum = sum + textureSampleCompare(shadow_map, shadow_samp, uv + off, depth_ref);
        }
    }
    return sum / 9.0;
}

// `aces_fitted` — see `scene_hdr_tonemap.wgsl` (prepended at pipeline creation).

fn fresnel_schlick(cos_theta: f32, F0: vec3<f32>) -> vec3<f32> {
    return F0 + (vec3<f32>(1.0) - F0) * pow(max(1.0 - cos_theta, 0.0), 5.0);
}

fn distribution_ggx(NdotH: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = NdotH * NdotH * (a2 - 1.0) + 1.0;
    return a2 / max(PI * denom * denom, 1e-8);
}

fn geometry_schlick_ggx(NdotX: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    return NdotX / max(NdotX * (1.0 - k) + k, 1e-8);
}

fn geometry_smith(NdotV: f32, NdotL: f32, roughness: f32) -> f32 {
    return geometry_schlick_ggx(NdotV, roughness) * geometry_schlick_ggx(NdotL, roughness);
}

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) wn: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) local_pos: vec3<f32>,
    @location(3) local_n: vec3<f32>,
    @location(4) world_pos: vec3<f32>,
    @location(5) t_w: vec3<f32>,
    @location(6) b_w: vec3<f32>,
    @location(7) uv_emr: vec2<f32>,
    @location(8) v_color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @location(4) uv_emr_in: vec2<f32>,
    @location(5) v_color_in: vec4<f32>,
) -> VsOut {
    let world_h = (cam.model * vec4<f32>(pos, 1.0)).xyz;
    let N0 = normalize((cam.model * vec4<f32>(n, 0.0)).xyz);
    let Tw = (cam.model * vec4<f32>(tangent.xyz, 0.0)).xyz;
    let Torth = normalize(Tw - N0 * dot(N0, Tw));
    let Borth = normalize(cross(N0, Torth)) * tangent.w;

    let world = select(
        world_h,
        apply_hallway_distortion(world_h, hd),
        hd.flags.x > 0.5,
    );
    let N = select(
        N0,
        world_normal_after_distortion(world_h, Torth, Borth, hd, N0),
        hd.flags.x > 0.5,
    );
    let Tw2 = (cam.model * vec4<f32>(tangent.xyz, 0.0)).xyz;
    let Torth2 = normalize(Tw2 - N * dot(N, Tw2));
    let Borth2 = normalize(cross(N, Torth2)) * tangent.w;

    var o: VsOut;
    o.clip_pos = cam.view_proj * vec4<f32>(world, 1.0);
    o.wn = N;
    o.uv = uv;
    o.local_pos = pos;
    o.local_n = n;
    o.world_pos = world;
    o.t_w = Torth2;
    o.b_w = Borth2;
    o.uv_emr = uv_emr_in;
    o.v_color = v_color_in;
    return o;
}

struct ShopShaded {
    hdr: vec3<f32>,
    emissive: vec3<f32>,
    out_alpha: f32,
}

struct ShopHdrMrtOut {
    @location(0) hdr: vec4<f32>,
    @location(1) emissive: vec4<f32>,
}

fn shop_shade(in: VsOut, front_facing: bool) -> ShopShaded {
    let base_s = textureSample(base_color, base_sampler, in.uv);
    let is_hallway_wall_tint = abs(in.v_color.a - 3.0) < 0.01;
    let vtx_alpha = select(in.v_color.a, 1.0, is_hallway_wall_tint);
    let tex_a = base_s.a * vtx_alpha;
    if (pbr.alpha_mode == 1u) {
        if (tex_a < pbr.alpha_cutoff) {
            discard;
        }
    }
    var out_alpha: f32 = 1.0;
    if (pbr.alpha_mode == 2u) {
        out_alpha = tex_a;
    }
    // Many Blender text / curve exports store glyph coverage in alpha only (RGB ≈ 0).
    // Multiplying that by vertex colour yields a black albedo while bevels still catch
    // highlights — reads as an outline with a black fill. Recover greyscale from alpha
    // for cutout materials only so opaque true-black surfaces stay black.
    var tex_rgb = base_s.rgb;
    let tex_lum = dot(tex_rgb, vec3<f32>(0.299, 0.587, 0.114));
    if ((pbr.alpha_mode == 1u || pbr.alpha_mode == 2u)
        && tex_lum < 1e-4
        && base_s.a > 1e-4) {
        tex_rgb = vec3<f32>(base_s.a);
    }
    var albedo = tex_rgb * in.v_color.rgb;
    if (is_hallway_wall_tint && hd.flags.x > 0.5) {
        albedo = albedo * hd.bow.rgb;
    }
    // Archive `sign_description_*` meshes tag `COLOR_0.a = 2` in `room_env_gltf` (see `decode_env_primitive`).
    if (in.v_color.a > 1.5 && !is_hallway_wall_tint) {
        let dec = textureSample(decal_tex, base_sampler, in.uv);
        albedo = mix(albedo, dec.rgb, dec.a);
    }
    let albedo_lum = dot(albedo, vec3<f32>(0.299, 0.587, 0.114));

    let mr_s = textureSample(metallic_roughness_tex, base_sampler, in.uv_emr);
    let metallic = clamp(mr_s.b * pbr.metallic_factor, 0.0, 1.0);
    let roughness = clamp(mr_s.g * pbr.roughness_factor, 0.04, 1.0);

    let nm = textureSample(normal_tex, base_sampler, in.uv_emr).rgb * 2.0 - 1.0;
    var Ngeom = normalize(in.wn);
    if (!front_facing) {
        Ngeom = -Ngeom;
    }
    let T = normalize(in.t_w);
    let B = normalize(in.b_w);
    let n_world = normalize(nm.x * T + nm.y * B + nm.z * Ngeom);

    let emissive = textureSample(emissive_tex, base_sampler, in.uv_emr).rgb
        * pbr.emissive_factor.rgb
        * cam.decal_atlas_uv.z;

    let V = normalize(cam.cam_pos - in.world_pos);
    let NdotV = max(dot(n_world, V), 1e-4);
    // Punctual-only shading has no IBL: metallic BRDF uses baseColor as F0. Near-black
    // baseColor + metallic≈1 makes F0≈0 so large flat facets read black while bevels
    // still catch specular. Floor F0 for that case and add a small metal ambient fill.
    let metal_f0_floor = vec3<f32>(0.52, 0.42, 0.24);
    let boost_dark_metal_f0 = metallic > 0.55 && albedo_lum < 0.07;
    let f0_base = select(albedo, max(albedo, metal_f0_floor), boost_dark_metal_f0);
    let F0 = mix(vec3<f32>(0.04), f0_base, metallic);

    var Lo = vec3<f32>(0.0);
    // Boss blind (`hd.flags.y` ≈ 1): wash punctual + spots toward red without touching emissive.
    let boss_press = clamp(hd.flags.y, 0.0, 1.0);
    let punctual_rgb_mul = mix(
        vec3<f32>(1.0),
        vec3<f32>(1.14, 0.42, 0.36),
        boss_press,
    );

    let light_count = lights.count.x;
    for (var i: u32 = 0u; i < light_count; i = i + 1u) {
        let pl = lights.lights[i];
        let light_pos = pl.pos.xyz;
        let range_w = pl.pos.w;
        let to_light = light_pos - in.world_pos;
        let dist = length(to_light);
        let L = to_light / max(dist, 1e-4);
        let kind = pl.params.x;
        let atten = select(
            scene_smooth_point_atten(dist, range_w),
            punctual_attenuation_with_inv_doc_scale(dist, range_w, cam.decal_atlas_uv.y),
            kind > 0.5,
        );
        let radiance = pl.color.rgb * punctual_rgb_mul * pl.color.a * atten;
        let NdotL = max(dot(n_world, L), 0.0);
        if (NdotL <= 0.0 || length(radiance) <= 0.0) {
            continue;
        }
        let H = normalize(V + L);
        let NdotH = max(dot(n_world, H), 0.0);
        let VdotH = max(dot(V, H), 0.0);

        let F = fresnel_schlick(VdotH, F0);
        let kS = F;
        let kD = (vec3<f32>(1.0) - kS) * (1.0 - metallic);

        let diffuse = kD * albedo / PI * radiance * NdotL;

        let D = distribution_ggx(NdotH, roughness);
        let G = geometry_smith(NdotV, NdotL, roughness);
        let spec_brdf = D * G * F / max(4.0 * NdotV * NdotL, 1e-6);
        let specular = spec_brdf * radiance * NdotL;

        Lo = Lo + diffuse + specular;
    }

    let spot_count = spot_lights.count.x;
    for (var si: u32 = 0u; si < spot_count; si = si + 1u) {
        let s = spot_lights.lights[si];
        let to_frag = in.world_pos - s.pos.xyz;
        let dist = length(to_frag);
        let range_spot = s.pos.w;
        let atten_spot = punctual_attenuation_with_inv_doc_scale(dist, range_spot, cam.decal_atlas_uv.y);
        if (atten_spot <= 0.0) {
            continue;
        }
        let L = -to_frag / max(dist, 1e-4);
        let frag_dir = to_frag / max(dist, 1e-4);
        let cos_a = dot(frag_dir, s.dir.xyz);
        let cos_outer = s.dir.w;
        let cos_inner = s.params.x;
        let spot_factor = khr_spot_angle_attenuation_scene(cos_a, cos_inner, cos_outer);
        if (spot_factor <= 0.0) {
            continue;
        }
        let radiance = s.color.rgb * punctual_rgb_mul * s.color.a * atten_spot * spot_factor;
        let NdotL = max(dot(n_world, L), 0.0);
        if (NdotL <= 0.0) {
            continue;
        }
        let H = normalize(V + L);
        let NdotH = max(dot(n_world, H), 0.0);
        let VdotH = max(dot(V, H), 0.0);

        let F = fresnel_schlick(VdotH, F0);
        let kS = F;
        let kD = (vec3<f32>(1.0) - kS) * (1.0 - metallic);

        let diffuse = kD * albedo / PI * radiance * NdotL;

        let D = distribution_ggx(NdotH, roughness);
        let G = geometry_smith(NdotV, NdotL, roughness);
        let spec_brdf = D * G * F / max(4.0 * NdotV * NdotL, 1e-6);
        let specular = spec_brdf * radiance * NdotL;

        Lo = Lo + diffuse + specular;
    }

    let ambient_scale = cam.decal_atlas_uv.x;
    let amb_dielectric = ambient_scale * albedo * (1.0 - metallic) * vec3<f32>(0.08);
    let metal_amb_tint = mix(
        vec3<f32>(0.58, 0.50, 0.40),
        albedo,
        clamp(albedo_lum * 22.0, 0.0, 1.0),
    );
    let amb_metal = ambient_scale * metallic * metal_amb_tint * vec3<f32>(0.14);
    let ambient = amb_dielectric + amb_metal;

    // `SHOP_ENV_AMBIENT_SCALE` defaults to 0 for this scene — dielectric ambient is off.
    // Without IBL, dark-metallic facets still need a tiny direction-dependent fill so
    // flat faces toward the camera read as metal, not void.
    let dark_metal_face = metallic > 0.5 && albedo_lum < 0.08;
    let hemi_tint = mix(
        vec3<f32>(0.062, 0.054, 0.041),
        albedo,
        clamp(albedo_lum * 15.0, 0.0, 1.0),
    );
    let metal_hemi = select(
        vec3<f32>(0.0),
        metallic * hemi_tint * (0.10 + 0.26 * NdotV),
        dark_metal_face,
    );

    // `tile_seed` is scene exposure for punctual/ambient PBR (often ≪ 1). glTF emissive is
    // already outgoing radiance; scaling it by the same crush makes lamps invisible.
    var hdr = (ambient + Lo + metal_hemi) * cam.tile_seed;
    hdr = hdr + emissive;

    // Gameplay shadow map is ortho-fit around the mahjong table near the origin.
    // shop.glb uses the same Z-up frame but geometry is scaled by `window_h`, so
    // light-space coverage is meaningless here and PCF often zeros the room.
    let gameplay_shadow = sample_shadow_visibility(in.world_pos);
    // `clamp(gs+1,1,2) - gs` is always 1.0 but keeps the shadow sample live for the layout.
    let shadow_neutral =
        clamp(gameplay_shadow + 1.0, 1.0, 2.0) - gameplay_shadow;
    hdr = hdr * shadow_neutral;

    return ShopShaded(hdr, emissive, out_alpha);
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let s = shop_shade(in, front_facing);
    if (cam.hdr_tonemap.w > 0.5) {
        return vec4<f32>(s.hdr, s.out_alpha);
    }

    let mapped = aces_fitted(s.hdr);
    let inv_g = 1.0 / max(lights.extras.x, 0.01);
    let out_rgb = pow(mapped, vec3<f32>(inv_g));
    return vec4<f32>(out_rgb, s.out_alpha);
}

/// Linear-HDR MRT pass (shop/hallway bloom pre-pass): RT0 matches `fs_main` linear path;
/// RT1 is **emissive only** (texture × factor × strength) for screen-space GI, not BRDF.
@fragment
fn fs_main_mrt(in: VsOut, @builtin(front_facing) front_facing: bool) -> ShopHdrMrtOut {
    let s = shop_shade(in, front_facing);
    return ShopHdrMrtOut(
        vec4<f32>(s.hdr, s.out_alpha),
        vec4<f32>(s.emissive, s.out_alpha),
    );
}
