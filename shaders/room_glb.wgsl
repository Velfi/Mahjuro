// shop.glb environment — glTF-style punctual lights + metallic–roughness + ACES (fitted).
// Shares punctual attenuation and direct diffuse receiver math with `tile_3d.wgsl`
// and `lit_mesh.wgsl`.
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
// - `GLTF_PBR_FLAG_ROOM_READABLE_SURFACE` = GLB text/decal/UI surface: skip combined
//   receiver shadow, baked contact AO, and coarse room lightmap fill so copy stays legible.
// - `GLTF_PBR_FLAG_ROOM_SKIP_LIGHTMAP` = explicit opt-out for surfaces whose lighting is
//   fully supplied by material/emissive paths.
//
// Point `pos.w` = smooth radius or glTF inverse-square range depending on `params.x`.
// Spot `pos.w` = smooth radius. Both are in world units after upload.

const GLTF_PBR_FLAG_ROOM_HALLWAY_WALL_TINT: u32 = 1u << 0u;
const GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL: u32 = 1u << 1u;
const GLTF_PBR_FLAG_MAIN_MENU_MOON_PHASE: u32 = 1u << 2u;
const GLTF_PBR_FLAG_MAIN_MENU_STAR_RAINBOW: u32 = 1u << 3u;
const GLTF_PBR_FLAG_GAMEPLAY_CASH_IN_POLYCHROME: u32 = 1u << 4u;
const GLTF_PBR_FLAG_SKIP_BAKED_CONTACT_AO: u32 = 1u << 5u;
const GLTF_PBR_FLAG_ROOM_WAX_SUBSURFACE: u32 = 1u << 6u;
const GLTF_PBR_FLAG_ROOM_DYNAMIC_SHADOW_RECEIVER: u32 = 1u << 7u;
const GLTF_PBR_FLAG_ROOM_READABLE_SURFACE: u32 = 1u << 8u;
const GLTF_PBR_FLAG_ROOM_SKIP_LIGHTMAP: u32 = 1u << 9u;
// UI polychrome (The House) — coarser bands than 3D score pops at label sizes.
const POLYCHROME_COORD_X: f32 = 2.0;
const POLYCHROME_COORD_Y: f32 = 1.25;
const POLYCHROME_WARP_Y: f32 = 3.5;
const POLYCHROME_WARP_X: f32 = 2.5;
const POLYCHROME_LIGHT_GOLD: vec3<f32> = vec3<f32>(1.0, 0.68, 0.08);
const POLYCHROME_SATURATION: f32 = 1.42;
const HOUSE_BASE_RGB: vec3<f32> = vec3<f32>(0.90, 0.15, 0.10);
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
    room_lightmap_uv: vec4<f32>,
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
@group(0) @binding(9) var room_lightmap_tex: texture_2d<f32>;

@group(0) @binding(8) var<uniform> hd: HallwayDistortion;

struct PointLight {
    pos: vec4<f32>,
    color: vec4<f32>,
    // params.x = attenuation kind, params.y = candle flame flag.
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
    @location(9) lightmap_uv: vec2<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @location(4) uv_emr_in: vec2<f32>,
    @location(5) v_color_in: vec4<f32>,
    @location(6) lightmap_uv_in: vec2<f32>,
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
    o.lightmap_uv = lightmap_uv_in;
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

fn room_lightmap_wrap_coord(v: f32, mode: u32) -> f32 {
    if (mode == 0u) {
        return clamp(v, 0.0, 1.0);
    }
    if (mode == 2u) {
        let t = v - floor(v / 2.0) * 2.0;
        return select(t, 2.0 - t, t > 1.0);
    }
    return fract(v);
}

fn room_lightmap_texel(p: vec2<i32>) -> vec3<f32> {
    return textureLoad(room_lightmap_tex, p, 0).rgb;
}

fn room_lightmap_bilerp(uv: vec2<f32>) -> vec3<f32> {
    let dims_u = max(textureDimensions(room_lightmap_tex), vec2<u32>(1u));
    let dims = vec2<f32>(f32(dims_u.x), f32(dims_u.y));
    let max_p = vec2<i32>(i32(dims_u.x - 1u), i32(dims_u.y - 1u));
    let p = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * dims - vec2<f32>(0.5);
    let p0 = vec2<i32>(floor(p));
    let p1 = min(p0 + vec2<i32>(1), max_p);
    let p00 = clamp(p0, vec2<i32>(0), max_p);
    let w = clamp(p - vec2<f32>(p00), vec2<f32>(0.0), vec2<f32>(1.0));
    let c00 = room_lightmap_texel(p00);
    let c10 = room_lightmap_texel(vec2<i32>(p1.x, p00.y));
    let c01 = room_lightmap_texel(vec2<i32>(p00.x, p1.y));
    let c11 = room_lightmap_texel(p1);
    return mix(mix(c00, c10, w.x), mix(c01, c11, w.x), w.y);
}

fn sample_room_lightmap_indirect(uv: vec2<f32>) -> vec3<f32> {
    let rect = cam.room_lightmap_uv;
    let lm_uv = rect.xy + clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * rect.zw;
    return max(room_lightmap_bilerp(lm_uv), vec3<f32>(0.0));
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
    let is_wax_subsurface = (pbr.flags & GLTF_PBR_FLAG_ROOM_WAX_SUBSURFACE) != 0u;
    let receives_dynamic_room_shadow =
        (pbr.flags & GLTF_PBR_FLAG_ROOM_DYNAMIC_SHADOW_RECEIVER) != 0u;
    let is_readable_room_surface =
        (pbr.flags & GLTF_PBR_FLAG_ROOM_READABLE_SURFACE) != 0u;
    let skips_room_lightmap =
        (pbr.flags & GLTF_PBR_FLAG_ROOM_SKIP_LIGHTMAP) != 0u;

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
    let T = scene_safe_normalize(in.t_w, vec3<f32>(1.0, 0.0, 0.0));
    let B = scene_safe_normalize(in.b_w, vec3<f32>(0.0, 1.0, 0.0));
    let n_world = scene_safe_normalize(nm.x * T + nm.y * B + nm.z * Ngeom, Ngeom);

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
    let pbr_surface = scene_pbr_direct_surface(albedo, n_world, V, metallic, roughness);

    // Shop gold lettering (SHOP / tagline): it should read as warm metal reflecting
    // the room's lanterns, not as a self-lit sign. Classify once here so the light
    // loops below can add a small rough-gold reflection from actual punctual lights.
    let warm_gold_sign = albedo_lum > 0.45
        && albedo.r > albedo.g * 0.90
        && albedo.g > albedo.b * 1.20
        && albedo.r > albedo.b * 2.2;
    let gold_sign_ramp = smoothstep(0.45, 0.85, metallic)
        * select(0.0, 1.0, warm_gold_sign);
    let gold_face_view = 0.22 + 0.50 * pow(NdotV, 0.7);

    var Lo = vec3<f32>(0.0);
    var gold_reflected_fill = vec3<f32>(0.0);
    var wax_subsurface = vec3<f32>(0.0);
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
        let kind = pl.params.x;
        let point_sample = scene_pbr_sample_point_light(
            in.world_pos,
            light_pos,
            range_w,
            vec4<f32>(pl.color.rgb * boss_light_rgb_mul, pl.color.a),
            kind,
            cam.room_env_params.y,
        );
        let L = point_sample.direction;
        let dist = point_sample.distance;
        let radiance = point_sample.radiance;
        if (length(radiance) <= 0.0) {
            continue;
        }

        let projected_shadows_on =
            shadow_globals.params.x > 0.5 && receives_dynamic_room_shadow;
        let punc_vis = select(
            1.0,
            punctual_shadow_vis(i, in.world_pos),
            projected_shadows_on,
        );
        let gold_soft_ndl = smoothstep(-0.12, 0.62, dot(n_world, L));
        gold_reflected_fill = gold_reflected_fill
            + gold_sign_ramp * albedo * radiance * gold_soft_ndl * gold_face_view * 0.018 * punc_vis;
        if (is_wax_subsurface) {
            let backlit = smoothstep(-0.16, 0.86, dot(-n_world, L));
            let view_rim = 0.34 + 0.66 * pow(1.0 - NdotV, 1.45);
            let wax_tint = mix(vec3<f32>(1.0), max(albedo, vec3<f32>(0.04)), 0.42);
            wax_subsurface = wax_subsurface
                + radiance * wax_tint * backlit * view_rim * punc_vis * 0.048;
        }

        let direct = scene_pbr_direct_sampled_light(pbr_surface, point_sample, punc_vis);
        Lo = Lo + direct.total;
    }

    let spot_count = spot_lights.count.x;
    for (var si: u32 = 0u; si < spot_count; si = si + 1u) {
        let s = spot_lights.lights[si];
        let spot_sample = scene_pbr_sample_spot_light(
            in.world_pos,
            s.pos.xyz,
            s.pos.w,
            s.dir.xyz,
            s.params.x,
            s.dir.w,
            vec4<f32>(s.color.rgb * boss_light_rgb_mul, s.color.a),
            0.0,
            cam.room_env_params.y,
        );
        let L = spot_sample.direction;
        let radiance = spot_sample.radiance;
        if (length(radiance) <= 0.0) {
            continue;
        }
        let punc_vis = 1.0;
        let gold_soft_ndl = smoothstep(-0.12, 0.62, dot(n_world, L));
        gold_reflected_fill = gold_reflected_fill
            + gold_sign_ramp * albedo * radiance * gold_soft_ndl * gold_face_view * 0.018 * punc_vis;
        if (is_wax_subsurface) {
            let backlit = smoothstep(-0.16, 0.86, dot(-n_world, L));
            let view_rim = 0.34 + 0.66 * pow(1.0 - NdotV, 1.45);
            let wax_tint = mix(vec3<f32>(1.0), max(albedo, vec3<f32>(0.04)), 0.42);
            wax_subsurface = wax_subsurface
                + radiance * wax_tint * backlit * view_rim * punc_vis * 0.048;
        }

        let direct = scene_pbr_direct_sampled_light(pbr_surface, spot_sample, punc_vis);
        Lo = Lo + direct.total;
    }

    if (!is_readable_room_surface && !skips_room_lightmap) {
        let room_indirect_shadow_vis = select(
            1.0,
            dynamic_receiver_shadow_vis(in.world_pos),
            receives_dynamic_room_shadow && !is_readable_room_surface,
        );
        Lo = Lo + sample_room_lightmap_indirect(in.lightmap_uv) * room_indirect_shadow_vis;
    }

    Lo = Lo + gold_reflected_fill + wax_subsurface;

    // `room_linear_exposure` is scene exposure for punctual PBR (often ≪ 1 to tame imported
    // glTF light energy). Indirect/environment terms are supplied by the room lightmap,
    // which is baked through the shared scene PBR helpers using the same exposure convention.
    var lit_hdr = Lo * cam.room_linear_exposure;
    let skips_baked_contact_ao = ((pbr.flags & GLTF_PBR_FLAG_SKIP_BAKED_CONTACT_AO) != 0u)
        || is_readable_room_surface;
    if (!skips_baked_contact_ao) {
        lit_hdr = lit_hdr * sample_contact_ao(in.world_pos);
    }
    // Per-light projected shadows are applied in the punctual / spot loops above.
    let emissive_out = min(emissive * boss_light_rgb_mul, vec3<f32>(65000.0));
    var hdr = lit_hdr + emissive_out;
    if (is_house_polychrome) {
        // House ordeal cash-in: keep band colours stable under candle swing.
        let self_lit = albedo * 0.90;
        hdr = mix(lit_hdr, self_lit, 0.62) + emissive_out;
    }
    // Clamp to prevent Rgba16Float overflow (Infinity) which causes NaN during bloom bilinear filtering on Metal
    hdr = min(hdr, vec3<f32>(65000.0));
    return ShopShaded(hdr, emissive_out, out_alpha);
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    // Always linear HDR. `tonemap_composite.wgsl` applies the single ACES pass.
    let s = shop_shade(in, front_facing);
    return vec4<f32>(apply_exponential_height_fog(s.hdr, in.world_pos), s.out_alpha);
}

/// Emissive-only pre-pass for bloom separation (writes `room_emissive_view`).
/// `s.emissive` = texture × factor × strength (outgoing radiance), not BRDF.
@fragment
fn fs_main_emissive(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let s = shop_shade(in, front_facing);
    return vec4<f32>(s.emissive, s.out_alpha);
}
