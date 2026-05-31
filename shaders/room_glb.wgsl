// shop.glb environment — glTF-style punctual lights + metallic–roughness + ACES (fitted).
// Separate from `tile_3d.wgsl` (candle pools + artistic lambert floor).
//
// Uniform hacks (same `CameraUniform` layout as tiles; shop writer only):
// - `base_color_factor.y` = animation-lab unlit debug (`1` = flat N·L).
// - `tile_seed`     = linear HDR exposure multiplier before tonemap
// - `decal_atlas_uv.x` = ambient scale (0 = punctual-only interior)
// - `decal_atlas_uv.y` = 1/world_scale — inverse-square uses document-space distance (glTF units)
// - `decal_atlas_uv.z` = glTF emissive strength multiplier (see `SHOP_GLTF_EMISSIVE_SCALE`)
//
// Point / spot `pos.w` = max light distance in **world units** (`KHR_lights_punctual` range),
// or `0` for infinite range (pure inverse-square with a minimum distance clamp).

const PI: f32 = 3.14159265358979323846;
// Hallway vertex warp: `hallway_vertex_warp.wgsl` prepended in `embedded_wgsl::SHOP_GLB`.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    base_color_factor: vec4<f32>,
    cam_pos: vec3<f32>,
    tile_seed: f32,
    decal_atlas_uv: vec4<f32>,
    /// Must match `wgpu_renderer::CameraUniform::hdr_tonemap` (layout parity with `lit_mesh`).
    /// `w` = main-menu pride rainbow scene time when active (`0` = off); moon/star
    /// meshes tagged via `pbr.emissive_factor.w`.
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
/// (`SHOP_ENV_DIELECTRIC_AMBIENT_MIN` floor on shop) to stand in for glTF's missing World node.
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
    // Archive decal boards tag `COLOR_0.a = 2` in `room_env_gltf` (see `decode_env_primitive`).
    let is_archive_decal = abs(in.v_color.a - 2.0) < 0.01;
    let is_archive_no_dir_shadow = abs(in.v_color.a - 3.0) < 0.01;
    if (is_archive_decal) {
        let dec = textureSample(decal_tex, base_sampler, in.uv);
        albedo = mix(albedo, dec.rgb, dec.a);
    }

    // Animation-lab false shading: albedo × simple N·L (skips punctual PBR / shadows).
    if (cam.base_color_factor.y > 0.5) {
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
        * cam.decal_atlas_uv.z;
    // Main-menu `MoonObject` / `star*`: `emissive_factor.w` tags the rainbow path;
    // `hdr_tonemap.w` carries scene time while June pride (or debug override) is on.
    if (pbr.emissive_factor.w > 0.5 && cam.hdr_tonemap.w > 0.0) {
        let mask = max(emissive.r, max(emissive.g, emissive.b));
        let swirl_uv = in.uv_emr * 0.65
            + vec2<f32>(in.world_pos.x, in.world_pos.y) * 0.004;
        let rainbow = rainbow_swirl_rgb(swirl_uv, cam.hdr_tonemap.w);
        emissive = rainbow * mask;
    }

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

    let ambient_scale = cam.decal_atlas_uv.x;
    let amb_dielectric = room_world_hemisphere_ambient(n_world, albedo, metallic, ambient_scale);
    let metal_amb_tint = mix(
        vec3<f32>(0.58, 0.50, 0.40),
        albedo,
        clamp(albedo_lum * 22.0, 0.0, 1.0),
    );
    let amb_metal = ambient_scale * metallic * metal_amb_tint * vec3<f32>(0.14);
    let ambient = amb_dielectric + amb_metal;

    // Shop authoring may set ambient scale to 0; runtime still floors dielectric fill via
    // `SHOP_ENV_DIELECTRIC_AMBIENT_MIN` so punctual-only interiors do not black out.
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

    // `tile_seed` is scene exposure for punctual PBR (often ≪ 1 to tame imported
    // glTF light energy). Keep the runtime hemisphere fill in scene-linear units
    // so the Scene Look "Room ambient" slider remains visible.
    var lit_hdr = Lo * cam.tile_seed + ambient + metal_hemi;
    // Per-light projected shadows are applied in the punctual / spot loops above.
    let hdr = lit_hdr + emissive;
    return ShopShaded(hdr, emissive, out_alpha);
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    // Always linear HDR. `tonemap_composite.wgsl` applies the single ACES pass.
    let s = shop_shade(in, front_facing);
    return vec4<f32>(s.hdr, s.out_alpha);
}

/// Emissive-only pre-pass for screen-space GI (writes `room_emissive_view`).
/// `s.emissive` = texture × factor × strength (outgoing radiance), not BRDF.
@fragment
fn fs_main_emissive(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let s = shop_shade(in, front_facing);
    return vec4<f32>(s.emissive, s.out_alpha);
}
