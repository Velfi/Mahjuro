// shop.glb environment — glTF-style punctual lights + metallic–roughness + ACES (fitted).
// Separate from `tile_3d.wgsl` (candle pools + artistic lambert floor).
//
// Room-env uniform (`RoomEnvUniform` in Rust) keeps tile-layout parity while
// naming room semantics explicitly:
// - `room_debug_params.y` = animation-lab unlit debug (`1` = flat N·L).
// - `room_linear_exposure` = linear HDR exposure multiplier before tonemap.
// - `room_env_params.x` = ambient scale (0 = punctual-only interior)
// - `room_env_params.y` = 1/world_scale — inverse-square uses document-space distance (glTF units)
// - `room_env_params.z` = glTF emissive strength multiplier (see `SHOP_GLTF_EMISSIVE_SCALE`)
// - `room_env_params.w` = main-menu hub moon synodic phase (`0..1`; unused elsewhere)
// - `room_post_params.w` = main-menu pride rainbow scene time, or gameplay House
//   polychrome time on blocked cash-in (`0` = off for both)
// - `room_height_fog_params.xyz` = main-menu exponential height fog
//   (`floor_z`, `height_world`, `density_per_world_unit`; zero density = off)
// - `room_height_fog_color.xyz` = base height fog target color in linear HDR space
// - `room_height_fog_color.w` = distance-tint gradient start in world units
// - `room_height_fog_far_color.xyz` = distance tint color approached as distance grows
// - `room_height_fog_far_color.w` = distance-tint exponential scale in world units
//
// Point / spot `pos.w` = max light distance in **world units** (`KHR_lights_punctual` range),
// or `0` for infinite range (pure inverse-square with a minimum distance clamp).

const PI: f32 = 3.14159265358979323846;
const GLTF_PBR_FLAG_ROOM_HALLWAY_WALL_TINT: u32 = 1u << 0u;
const GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL: u32 = 1u << 1u;
const GLTF_PBR_FLAG_MAIN_MENU_MOON_PHASE: u32 = 1u << 2u;
const GLTF_PBR_FLAG_MAIN_MENU_STAR_RAINBOW: u32 = 1u << 3u;
const GLTF_PBR_FLAG_GAMEPLAY_CASH_IN_POLYCHROME: u32 = 1u << 4u;
// UI polychrome (The House) — coarser bands than 3D score pops at label sizes.
const POLYCHROME_COORD_X: f32 = 2.0;
const POLYCHROME_COORD_Y: f32 = 1.25;
const POLYCHROME_WARP_Y: f32 = 3.5;
const POLYCHROME_WARP_X: f32 = 2.5;
const POLYCHROME_LIGHT_GOLD: vec3<f32> = vec3<f32>(1.0, 0.68, 0.08);
const POLYCHROME_SATURATION: f32 = 1.42;
const HOUSE_BASE_RGB: vec3<f32> = vec3<f32>(0.90, 0.15, 0.10);
/// Default `RoomEnvLightingTune::linear_exposure_base` (`2^-9`); gold signage fill is
/// normalized against this so Scene Look exposure tracks punctual lights, not emissive.
const ROOM_LINEAR_EXPOSURE_BASE_DEFAULT: f32 = 1.0 / 512.0;
// Hallway vertex warp: `hallway_vertex_warp.wgsl` prepended in `embedded_wgsl::SHOP_GLB`.

struct RoomEnvUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    room_debug_params: vec4<f32>,
    cam_pos: vec3<f32>,
    room_linear_exposure: f32,
    room_env_params: vec4<f32>,
    room_post_params: vec4<f32>,
    room_height_fog_params: vec4<f32>,
    room_height_fog_color: vec4<f32>,
    room_height_fog_far_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> cam: RoomEnvUniform;
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
    flags: u32,
    _pad1_0: u32,
    _pad1_1: u32,
}

@group(0) @binding(5) var<uniform> pbr: GltfPbrUniform;
@group(0) @binding(6) var metallic_roughness_tex: texture_2d<f32>;
@group(0) @binding(7) var emissive_tex: texture_2d<f32>;

@group(0) @binding(8) var<uniform> hd: HallwayDistortion;

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

// Collision-mesh AABBs (group 1 binding 1 — same buffer as table tile occluders).
struct RoomOccluder {
    center: vec4<f32>,
    half_extents: vec4<f32>,
};
struct RoomOccluders {
    count: vec4<u32>,
    boxes: array<RoomOccluder, 16>,
};
@group(1) @binding(1) var<uniform> room_occluders: RoomOccluders;

fn room_punctual_occlusion(light_pos: vec3<f32>, frag_pos: vec3<f32>, frag_scr: vec2<f32>) -> f32 {
    let n = room_occluders.count.x;
    if (n == 0u) {
        return 1.0;
    }
    let dir = frag_pos - light_pos;
    let dist = length(dir);
    if (dist < 1e-4) {
        return 1.0;
    }
    let l = dir / dist;
    // Small tangent-plane jitter so shelf edges don't alias as a hard screen-space seam.
    let up = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0), abs(l.z) > 0.92);
    let t1 = normalize(cross(up, l));
    let t2 = cross(l, t1);
    let jitter = (scene_ign(frag_scr) - 0.5) * min(dist * 0.04, 2.5);
    let lp = light_pos + (t1 * cos(jitter * 6.2831853) + t2 * sin(jitter * 6.2831853)) * jitter;
    let ray = frag_pos - lp;
    let safe = ray + vec3<f32>(1e-6);
    let inv = vec3<f32>(1.0) / safe;
    let near_bias = 0.015;
    let far_bias = 0.985;
    for (var k: u32 = 0u; k < n; k = k + 1u) {
        let c = room_occluders.boxes[k].center.xyz;
        let h = room_occluders.boxes[k].half_extents.xyz;
        if (scene_segment_hits_aabb(lp, inv, c, h, near_bias, far_bias)) {
            return 0.0;
        }
    }
    return 1.0;
}

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

/// Diffuse weight with a small floor so punctual-only scenes do not black out at grazing
/// view (Schlick `kD → 0`) — approximates Blender World + EEVEE ambient/indirect on dielectrics.
fn dielectric_kD(kS: vec3<f32>, metallic: f32) -> vec3<f32> {
    let dielectric = 1.0 - metallic;
    let kd = (vec3<f32>(1.0) - kS) * dielectric;
    return max(kd, vec3<f32>(0.04 * dielectric));
}

/// Warm sky / darker ground hemispheric fill (Z-up). `ambient_scale` is runtime-tuned
/// (`RoomEnvLightingTune::ambient_scale`) to stand in for glTF's missing World node.
fn room_world_hemisphere_ambient(n_world: vec3<f32>, albedo: vec3<f32>, metallic: f32, ambient_scale: f32) -> vec3<f32> {
    let dielectric = 1.0 - metallic;
    let world_up = vec3<f32>(0.0, 0.0, 1.0);
    let hemi_mix = clamp(dot(n_world, world_up) * 0.5 + 0.5, 0.0, 1.0);
    let sky = vec3<f32>(0.58, 0.55, 0.50);
    let ground = vec3<f32>(0.26, 0.22, 0.18);
    let hemi_col = mix(ground, sky, hemi_mix);
    return ambient_scale * albedo * dielectric * hemi_col * 0.09;
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

fn saturate_rgb(rgb: vec3<f32>, amount: f32) -> vec3<f32> {
    let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(vec3<f32>(luma), rgb, amount);
}

fn linear_rgb_to_oklab(rgb: vec3<f32>) -> vec3<f32> {
    let safe_rgb = max(rgb, vec3<f32>(0.0));
    let lms = vec3<f32>(
        dot(safe_rgb, vec3<f32>(0.4122214708, 0.5363325363, 0.0514459929)),
        dot(safe_rgb, vec3<f32>(0.2119034982, 0.6806995451, 0.1073969566)),
        dot(safe_rgb, vec3<f32>(0.0883024619, 0.2817188376, 0.6299787005)),
    );
    let lms_cbrt = pow(max(lms, vec3<f32>(0.0)), vec3<f32>(1.0 / 3.0));
    return vec3<f32>(
        dot(lms_cbrt, vec3<f32>(0.2104542553, 0.7936177850, -0.0040720468)),
        dot(lms_cbrt, vec3<f32>(1.9779984951, -2.4285922050, 0.4505937099)),
        dot(lms_cbrt, vec3<f32>(0.0259040371, 0.7827717662, -0.8086757660)),
    );
}

fn oklab_to_linear_rgb(lab: vec3<f32>) -> vec3<f32> {
    let lms_cbrt = vec3<f32>(
        lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z,
        lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z,
        lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z,
    );
    let lms = lms_cbrt * lms_cbrt * lms_cbrt;
    return max(
        vec3<f32>(
            dot(lms, vec3<f32>(4.0767416621, -3.3077115913, 0.2309699292)),
            dot(lms, vec3<f32>(-1.2684380046, 2.6097574011, -0.3413193965)),
            dot(lms, vec3<f32>(-0.0041960863, -0.7034186147, 1.7076147010)),
        ),
        vec3<f32>(0.0),
    );
}

fn mix_fog_color_perceptual(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    return oklab_to_linear_rgb(mix(linear_rgb_to_oklab(a), linear_rgb_to_oklab(b), t));
}

fn exponential_height_fog_alpha(world_pos: vec3<f32>) -> f32 {
    let density = max(cam.room_height_fog_params.z, 0.0);
    let height = max(cam.room_height_fog_params.y, 1e-3);
    if (density <= 0.0) {
        return 0.0;
    }
    let ray = world_pos - cam.cam_pos;
    let dist = length(ray);
    if (dist <= 1e-3) {
        return 0.0;
    }

    let floor_z = cam.room_height_fog_params.x;
    let k0 = clamp(-(cam.cam_pos.z - floor_z) / height, -32.0, 16.0);
    let k1 = clamp(-(world_pos.z - floor_z) / height, -32.0, 16.0);
    let dk = k1 - k0;
    var density_integral: f32;
    if (abs(dk) < 1e-3) {
        density_integral = dist * exp(0.5 * (k0 + k1));
    } else {
        density_integral = dist * (exp(k1) - exp(k0)) / dk;
    }
    let tau = density * max(density_integral, 0.0);
    return clamp(1.0 - exp(-min(tau, 80.0)), 0.0, 1.0);
}

fn apply_exponential_height_fog(hdr: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let fog = exponential_height_fog_alpha(world_pos);
    if (fog <= 0.0) {
        return hdr;
    }
    let dist = length(world_pos - cam.cam_pos);
    let gradient_start = max(cam.room_height_fog_color.w, 0.0);
    let gradient_scale = max(cam.room_height_fog_far_color.w, 1e-3);
    let gradient_dist = max(dist - gradient_start, 0.0);
    let gradient_t = clamp(1.0 - exp(-gradient_dist / gradient_scale), 0.0, 1.0);
    let fog_color = mix_fog_color_perceptual(
        cam.room_height_fog_color.xyz,
        cam.room_height_fog_far_color.xyz,
        gradient_t,
    );
    return mix(hdr, fog_color, fog);
}

// Port of `score_glyph_band_albedo_uv` in text_quad.wgsl — band timing in sync with UI House text.
fn score_glyph_band_albedo_uv(base: vec3<f32>, band_coord: vec2<f32>, uv: vec2<f32>, time: f32) -> vec3<f32> {
    let local = (uv - vec2<f32>(0.5)) * 2.0;
    let drift = time * 0.8;
    let warp = sin(time * 2.2 + band_coord.y * POLYCHROME_WARP_Y) * 0.28
             + sin(time * 1.4 + band_coord.x * POLYCHROME_WARP_X) * 0.18;
    let coord = band_coord.x * POLYCHROME_COORD_X + band_coord.y * POLYCHROME_COORD_Y + warp + drift;
    let wave = 0.5 + 0.5 * sin(coord * 6.2831855);
    let band = smoothstep(0.26, 0.74, wave);
    let bright = saturate_rgb(min(POLYCHROME_LIGHT_GOLD * 1.08, vec3<f32>(1.0)), POLYCHROME_SATURATION);
    let dark = saturate_rgb(base * 0.58, POLYCHROME_SATURATION * 1.08);
    var albedo = mix(dark, bright, band);
    let edge = length(local);
    let rim = pow(min(edge * 1.4, 1.0), 1.8) * 0.38;
    let rim_tint = saturate_rgb(
        mix(dark * 1.15, bright * 0.95, 0.62),
        POLYCHROME_SATURATION,
    );
    albedo = mix(albedo, rim_tint, rim);
    return saturate_rgb(albedo, 1.12);
}


fn shop_shade(in: VsOut, front_facing: bool) -> ShopShaded {
    let base_s = textureSample(base_color, base_sampler, in.uv);
    let is_hallway_wall_tint = (pbr.flags & GLTF_PBR_FLAG_ROOM_HALLWAY_WALL_TINT) != 0u;
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
    let is_archive_decal = (pbr.flags & GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL) != 0u;
    if (is_archive_decal) {
        let dec = textureSample(decal_tex, base_sampler, in.uv);
        albedo = mix(albedo, dec.rgb, dec.a);
    }
    if ((pbr.flags & GLTF_PBR_FLAG_GAMEPLAY_CASH_IN_POLYCHROME) != 0u
        && cam.room_post_params.w > 0.0) {
        let band_coord = in.world_pos.xy * 0.02;
        albedo = score_glyph_band_albedo_uv(HOUSE_BASE_RGB, band_coord, in.uv, cam.room_post_params.w);
    }
    let is_house_polychrome = (pbr.flags & GLTF_PBR_FLAG_GAMEPLAY_CASH_IN_POLYCHROME) != 0u
        && cam.room_post_params.w > 0.0;

    // Animation-lab false shading: albedo × simple N·L (skips punctual PBR / shadows).
    if (cam.room_debug_params.y > 0.5) {
        var n_geom = normalize(in.wn);
        if (!front_facing) {
            n_geom = -n_geom;
        }
        let key = normalize(vec3<f32>(0.32, -0.58, 0.75));
        let ndl = max(dot(n_geom, key), 0.0);
        let fake = albedo * (0.18 + 0.82 * ndl);
        return ShopShaded(fake, vec3<f32>(0.0), out_alpha);
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

    var emissive = textureSample(emissive_tex, base_sampler, in.uv_emr).rgb
        * pbr.emissive_factor.rgb
        * cam.room_env_params.z;
    // Main-menu moon mesh: phase terminator always; pride rainbow tints the lit
    // hemisphere when `room_post_params.w` > 0.
    if ((pbr.flags & GLTF_PBR_FLAG_MAIN_MENU_MOON_PHASE) != 0u) {
        let V = normalize(cam.cam_pos - in.world_pos);
        let phase_col =
            moon_hub_phase_emissive(albedo, n_world, V, cam.room_env_params.w);
        let phase_glow =
            moon_hub_phase_outer_glow(n_world, V, cam.room_env_params.w);
        if (cam.room_post_params.w > 0.0) {
            let lit_mask = moon_phase_lit_mask(n_world, V, cam.room_env_params.w);
            let mask = max(emissive.r, max(emissive.g, emissive.b));
            let swirl_uv = in.uv_emr * 0.65
                + vec2<f32>(in.world_pos.x, in.world_pos.y) * 0.004;
            let rainbow = rainbow_swirl_rgb(swirl_uv, cam.room_post_params.w);
            let rb = rainbow * mask;
            let phase_lum = dot(phase_col, vec3<f32>(0.299, 0.587, 0.114));
            let rb_lum = max(dot(rb, vec3<f32>(0.299, 0.587, 0.114)), 1e-4);
            let tint = rb * (phase_lum / rb_lum);
            emissive = mix(phase_col, tint, clamp(lit_mask * 0.94, 0.0, 1.0)) + phase_glow;
        } else {
            emissive = phase_col + phase_glow;
        }
        return ShopShaded(emissive, emissive, out_alpha);
    }
    // Main-menu star meshes: smooth pride fade only (no phase shading).
    if ((pbr.flags & GLTF_PBR_FLAG_MAIN_MENU_STAR_RAINBOW) != 0u && cam.room_post_params.w > 0.0) {
        let mask = max(emissive.r, max(emissive.g, emissive.b));
        let swirl_uv = in.uv_emr * 0.65
            + vec2<f32>(in.world_pos.x, in.world_pos.y) * 0.004;
        emissive = rainbow_swirl_smooth_rgb(swirl_uv, cam.room_post_params.w) * mask;
    }

    let V = normalize(cam.cam_pos - in.world_pos);
    let NdotV = max(dot(n_world, V), 1e-4);
    // Punctual-only shading has no IBL: metallic BRDF uses baseColor as F0. Near-black
    // baseColor + metallic≈1 makes F0≈0 so large flat facets read black while bevels
    // still catch specular. Floor F0 for that case and add a small metal ambient fill.
    let metal_f0_floor = vec3<f32>(0.52, 0.42, 0.24);
    // Smoothly floor F0 toward gold for dark metallic facets (embossed text fronts).
    // Hard thresholds flip across GPU backends when albedo sits on the boundary.
    let metal_ramp = smoothstep(0.45, 0.65, metallic);
    let dark_ramp = 1.0 - smoothstep(0.04, 0.16, albedo_lum);
    let f0_boost = metal_ramp * dark_ramp;
    let f0_base = mix(albedo, max(albedo, metal_f0_floor), f0_boost);
    let F0 = mix(vec3<f32>(0.04), f0_base, metallic);

    var Lo = vec3<f32>(0.0);
    // Boss blind (`hd.flags.y` ≈ 1): wash punctual, spots, and lamp emissive toward red.
    let boss_press = clamp(hd.flags.y, 0.0, 1.0);
    let boss_light_rgb_mul = mix(
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
            punctual_attenuation_with_inv_doc_scale(dist, range_w, cam.room_env_params.y),
            kind > 0.5,
        );
        let radiance = pl.color.rgb * boss_light_rgb_mul * pl.color.a * atten;
        let NdotL = max(dot(n_world, L), 0.0);
        if (NdotL <= 0.0 || length(radiance) <= 0.0) {
            continue;
        }
        let H = normalize(V + L);
        let NdotH = max(dot(n_world, H), 0.0);
        let VdotH = max(dot(V, H), 0.0);

        let F = fresnel_schlick(VdotH, F0);
        let kS = F;
        let kD = dielectric_kD(kS, metallic);

        let projected_shadows_on = shadow_globals.params.x > 0.5;
        let punc_vis = select(
            1.0,
            punctual_shadow_vis(i, in.world_pos),
            projected_shadows_on,
        );
        let diffuse = kD * albedo / PI * radiance * NdotL * punc_vis;

        let D = distribution_ggx(NdotH, roughness);
        let G = geometry_smith(NdotV, NdotL, roughness);
        let spec_brdf = D * G * F / max(4.0 * NdotV * NdotL, 1e-6);
        let specular = spec_brdf * radiance * NdotL * punc_vis;

        Lo = Lo + diffuse + specular;
    }

    let spot_count = spot_lights.count.x;
    for (var si: u32 = 0u; si < spot_count; si = si + 1u) {
        let s = spot_lights.lights[si];
        let to_frag = in.world_pos - s.pos.xyz;
        let dist = length(to_frag);
        let range_spot = s.pos.w;
        let atten_spot = punctual_attenuation_with_inv_doc_scale(dist, range_spot, cam.room_env_params.y);
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
        let radiance = s.color.rgb * boss_light_rgb_mul * s.color.a * atten_spot * spot_factor;
        let NdotL = max(dot(n_world, L), 0.0);
        if (NdotL <= 0.0) {
            continue;
        }
        let H = normalize(V + L);
        let NdotH = max(dot(n_world, H), 0.0);
        let VdotH = max(dot(V, H), 0.0);

        let F = fresnel_schlick(VdotH, F0);
        let kS = F;
        let kD = dielectric_kD(kS, metallic);

        let projected_shadows_on = shadow_globals.params.x > 0.5;
        let punc_vis = select(
            1.0,
            1.0,
            projected_shadows_on,
        );
        let diffuse = kD * albedo / PI * radiance * NdotL * punc_vis;

        let D = distribution_ggx(NdotH, roughness);
        let G = geometry_smith(NdotV, NdotL, roughness);
        let spec_brdf = D * G * F / max(4.0 * NdotV * NdotL, 1e-6);
        let specular = spec_brdf * radiance * NdotL * punc_vis;

        Lo = Lo + diffuse + specular;
    }

    let ambient_scale = cam.room_env_params.x;
    let amb_dielectric = room_world_hemisphere_ambient(n_world, albedo, metallic, ambient_scale);
    let metal_amb_tint = mix(
        vec3<f32>(0.58, 0.50, 0.40),
        albedo,
        clamp(albedo_lum * 22.0, 0.0, 1.0),
    );
    let amb_metal = ambient_scale * metallic * metal_amb_tint * vec3<f32>(0.14);
    let ambient = amb_dielectric + amb_metal;

    // Authoring may set ambient scale to 0; tune via RoomEnvLightingTune if fill is needed.
    // Without IBL, dark-metallic facets still need a tiny direction-dependent fill so
    // flat faces toward the camera read as metal, not void.
    let dark_metal_ramp = (1.0 - smoothstep(0.04, 0.18, albedo_lum))
        * smoothstep(0.40, 0.60, metallic);
    let hemi_tint = mix(
        vec3<f32>(0.062, 0.054, 0.041),
        albedo,
        clamp(albedo_lum * 15.0, 0.0, 1.0),
    );
    let metal_hemi = dark_metal_ramp * metallic * hemi_tint * (0.10 + 0.26 * NdotV);

    // Shop gold lettering (SHOP / tagline): bright baseColor + high metallic suppresses
    // diffuse; flat facets toward the camera miss punctual N·L. View-facing body fill
    // keeps the front face gold instead of edge-only specular (GPU-stable; not thresholded).
    let warm_gold_sign = albedo_lum > 0.45
        && albedo.r > albedo.g * 0.90
        && albedo.g > albedo.b * 1.20
        && albedo.r > albedo.b * 2.2;
    let gold_sign_ramp = smoothstep(0.45, 0.85, metallic)
        * select(0.0, 1.0, warm_gold_sign);
    let gold_sign_body = gold_sign_ramp * albedo * (0.22 + 0.40 * pow(NdotV, 0.7));
    // Scale with `room_linear_exposure` but keep default authored brightness (fill was tuned at
    // `linear_exposure_base == ROOM_LINEAR_EXPOSURE_BASE_DEFAULT`). Unscaled add looked
    // emissive when crushing room exposure in Scene Look.
    let gold_sign_hdr = gold_sign_body
        * cam.room_linear_exposure
        / max(ROOM_LINEAR_EXPOSURE_BASE_DEFAULT, 1e-6);

    // `room_linear_exposure` is scene exposure for punctual PBR (often ≪ 1 to tame imported
    // glTF light energy). Keep the runtime hemisphere fill in scene-linear units
    // so the Scene Look "Room ambient" slider remains visible.
    var lit_hdr = Lo * cam.room_linear_exposure + gold_sign_hdr + ambient + metal_hemi;
    lit_hdr = lit_hdr * sample_contact_ao(in.world_pos);
    // Per-light projected shadows are applied in the punctual / spot loops above.
    let emissive_out = emissive * boss_light_rgb_mul;
    var hdr = lit_hdr + emissive_out;
    if (is_house_polychrome) {
        // House ordeal cash-in: keep band colours stable under candle swing.
        let self_lit = albedo * 0.90;
        hdr = mix(lit_hdr, self_lit, 0.62) + emissive_out;
    }
    return ShopShaded(hdr, emissive_out, out_alpha);
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    // Always linear HDR. `tonemap_composite.wgsl` applies the single ACES pass.
    let s = shop_shade(in, front_facing);
    return vec4<f32>(apply_exponential_height_fog(s.hdr, in.world_pos), s.out_alpha);
}

/// Emissive-only pre-pass for screen-space GI (writes `room_emissive_view`).
/// `s.emissive` = texture × factor × strength (outgoing radiance), not BRDF.
@fragment
fn fs_main_emissive(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let s = shop_shade(in, front_facing);
    return vec4<f32>(s.emissive, s.out_alpha);
}
