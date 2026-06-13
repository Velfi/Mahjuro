struct TileFrameUniform {
    view_proj: mat4x4<f32>,
    cam_pos: vec3<f32>,
    _pad0: f32,
    /// x = ACES HDR path on; y = linear exposure; z = hemispheric ambient (albedo * 0.08);
    /// w = inverse document scale for embedded glTF punctual attenuation.
    tile_post_params: vec4<f32>,
    /// x = reserved; source punctual intensity is shared across room/tile/lit_mesh.
    tile_punctual_params: vec4<f32>,
};

// ACES tonemapping is applied once in `tonemap_composite.wgsl`. This shader
// writes linear HDR to `scene_color` (`Rgba16Float`).

@group(0) @binding(0) var<uniform> frame: TileFrameUniform;
@group(0) @binding(1) var base_color: texture_2d<f32>;
@group(0) @binding(2) var base_sampler: sampler;
@group(0) @binding(3) var decal_tex: texture_2d<f32>;
@group(0) @binding(4) var normal_tex: texture_2d<f32>;

/// Mirrors `GltfPbrUniform` in `gltf_helpers.rs` (std140 layout).
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

// Hallway vertex warp: `hallway_vertex_warp.wgsl` prepended in `embedded_wgsl::TILE_3D`.
@group(0) @binding(8) var<uniform> hd: HallwayDistortion;

struct PointLight {
    // xyz = world position after upload; w = smooth radius or inverse-square range.
    pos: vec4<f32>,
    // rgb = linear colour, a = intensity multiplier.
    color: vec4<f32>,
    // x = 0 smooth, 1 inverse-square (see `scene_pbr_lights.wgsl`).
    params: vec4<f32>,
};

struct PointLights {
    // count.x = number of active lights; rest is std140 padding.
    count: vec4<u32>,
    // extras.x = display gamma; extras.w = reserved.
    extras: vec4<f32>,
    lights: array<PointLight, 16>,
};

@group(1) @binding(0) var<uniform> lights: PointLights;

// ── Spotlights (group 3) ─────────────────────────────────────────────
// Directional cone lights used for focused visual highlights (e.g. hint
// indicators pooling green on a specific tile). Sampled by scene receivers
// through the same shared punctual diffuse helper as point lights.
struct SpotLight {
    // xyz = world-space position, w = falloff radius.
    pos: vec4<f32>,
    // xyz = normalized direction (light → fragment), w = cos(outer half-angle).
    dir: vec4<f32>,
    // rgb = linear colour, a = intensity.
    color: vec4<f32>,
    // x = cos(inner half-angle); Khronos angular falloff (linear in cos² then square).
    params: vec4<f32>,
};
struct SpotLights {
    count: vec4<u32>,
    lights: array<SpotLight, 8>,
};
@group(3) @binding(0) var<uniform> spot_lights: SpotLights;

// ── Shadow sampling comes from `projected_shadow.wgsl` (group 2) ─────

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) wn: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) local_pos: vec3<f32>,
    @location(3) local_n: vec3<f32>,
    @location(4) world_pos: vec3<f32>,
    /// World-space tangent (xyz) + glTF handedness (w).
    @location(5) t_w: vec4<f32>,
    /// UV for normal / metallic-roughness / emissive (glTF secondary TEXCOORD when present).
    @location(6) uv_emr: vec2<f32>,
    @location(7) v_color: vec4<f32>,
    @location(8) tile_visual_params: vec4<f32>,
    @location(9) tile_decal_atlas_uv: vec4<f32>,
    /// x = per-tile material seed, y = opacity (1 = opaque).
    @location(10) tile_seed_opacity: vec2<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @location(4) uv_emr_in: vec2<f32>,
    @location(5) v_color_in: vec4<f32>,
    @location(6) inst_model_c0: vec4<f32>,
    @location(7) inst_model_c1: vec4<f32>,
    @location(8) inst_model_c2: vec4<f32>,
    @location(9) inst_model_c3: vec4<f32>,
    @location(10) inst_normal_c0: vec4<f32>,
    @location(11) inst_normal_c1: vec4<f32>,
    @location(12) inst_normal_c2: vec4<f32>,
    @location(13) inst_visual: vec4<f32>,
    @location(14) inst_decal_uv: vec4<f32>,
    @location(15) inst_seed_opacity: vec2<f32>,
) -> VsOut {
    let model = mat4x4<f32>(inst_model_c0, inst_model_c1, inst_model_c2, inst_model_c3);
    let normal_model = mat3x3<f32>(
        inst_normal_c0.xyz,
        inst_normal_c1.xyz,
        inst_normal_c2.xyz,
    );

    let world_h = (model * vec4<f32>(pos, 1.0)).xyz;
    let N0 = normalize(normal_model * n);
    let Tw = (model * vec4<f32>(tangent.xyz, 0.0)).xyz;
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
    let Tw2 = (model * vec4<f32>(tangent.xyz, 0.0)).xyz;
    let Torth2 = normalize(Tw2 - N * dot(N, Tw2));
    let Borth2 = normalize(cross(N, Torth2)) * tangent.w;

    var o: VsOut;
    o.clip_pos = frame.view_proj * vec4<f32>(world, 1.0);
    o.wn = N;
    o.uv = uv;
    o.local_pos = pos;
    o.local_n = n;
    o.world_pos = world;
    o.t_w = vec4<f32>(Torth2, tangent.w);
    o.uv_emr = uv_emr_in;
    o.v_color = v_color_in;
    o.tile_visual_params = inst_visual;
    o.tile_decal_atlas_uv = inst_decal_uv;
    o.tile_seed_opacity = inst_seed_opacity;
    return o;
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    // Candle-keyed lighting: no directional sun; illumination comes from the
    // wick PointLights below. Fragments outside any candle pool stay black.

    // Front face = vertices whose local normal's dominant component is +Y
    // (the tile's flat top face after Z-up→Y-up conversion).  Use a soft
    // threshold so beveled-edge fragments still pick up the decal.
    let is_front = in.local_n.y > 0.0
        && in.local_n.y >= abs(in.local_n.x)
        && in.local_n.y >= abs(in.local_n.z);

    // tile_visual_params.w — see `tile_body.rs`:
    //   4 = sample bound base-color texture, no decal projection (shop room),
    //   5 = sample bound base-color per primitive + mahjong decal on **Face** material only.
    // Tiles are always GLB meshes, so the albedo is always sampled from a bound texture.
    let body_kind = in.tile_visual_params.w;
    let use_textured_env = body_kind > 3.5 && body_kind < 4.5;
    let use_textured_tile_glb = body_kind > 4.5 && body_kind < 5.5;
    let use_textured_albedo = use_textured_env || use_textured_tile_glb;

    var base_rgb: vec3<f32>;
    /// Fragment alpha (blend mode only for textured glTF paths).
    var out_alpha: f32 = 1.0;
    {
        let base_s = textureSample(base_color, base_sampler, in.uv);
        let tex_a = base_s.a * in.v_color.a;
        // `GltfAlphaMode`: Opaque = 0, Mask = 1, Blend = 2.
        if (pbr.alpha_mode == 1u) {
            if (tex_a < pbr.alpha_cutoff) {
                discard;
            }
        }
        if (pbr.alpha_mode == 2u) {
            out_alpha = tex_a;
        }
        // See `room_glb.wgsl`: alpha-mask base colour with zero RGB (common on text meshes).
        var tex_rgb = base_s.rgb;
        let tex_lum = dot(tex_rgb, vec3<f32>(0.299, 0.587, 0.114));
        if ((pbr.alpha_mode == 1u || pbr.alpha_mode == 2u)
            && tex_lum < 1e-4
            && base_s.a > 1e-4) {
            tex_rgb = vec3<f32>(base_s.a);
        }
        base_rgb = tex_rgb * in.v_color.rgb;
    }

    // Tile decal projection + groove darkening — skipped for shop env (kind 4) only.
    var rgb: vec3<f32>;
    if (use_textured_env) {
        rgb = base_rgb;
    } else {
        // Project decal UVs from model-space position onto the front face.
        // glTF **Face** quads use the same projection after `normalize_mesh`; authored
        // TEXCOORD_1 is kept for MR/normal maps but not decal orientation (UVs do not
        // rotate with the Z-up → engine +Y axis fix applied at load time).
        // The mesh's long face axis is local X (extent 1.0, mapped to screen-vertical
        // by the renderer); local Z is the short axis (extent 0.734, screen-horizontal).
        // Decal U follows local Z (horizontal on the face) and V follows local X
        // (vertical). Invert U only so atlas glyphs match left-to-right reading.
        let use_glb_face = use_textured_tile_glb && in.v_color.a > 0.5;
        let raw_u = in.local_pos.z * 1.362 + 0.5;
        let raw_v = in.local_pos.x + 0.5;
        let proj_uv = vec2<f32>(1.0 - raw_u, raw_v);
        let decal_uv_face = proj_uv;
        let decal_uv =
            decal_uv_face * in.tile_decal_atlas_uv.zw + in.tile_decal_atlas_uv.xy;
        let decal = textureSample(decal_tex, base_sampler, decal_uv);
        let in_uv = decal_uv_face.x >= 0.0 && decal_uv_face.x <= 1.0 && decal_uv_face.y >= 0.0 && decal_uv_face.y <= 1.0;
        // Imported tile meshes: decal only on the authored **Face** material (`v_color.a`).
        // Do not fall back to procedural `is_front` on body / side-band primitives.
        let decal_face = select(is_front, use_glb_face, use_textured_tile_glb);
        let decal_a = select(0.0, decal.a, decal_face && in_uv);
        let decal_rgb = decal.rgb;

        // Groove-floor darkening: the carved recess is slightly shadowed by
        // the groove walls before the paint/ink is laid in.
        let groove = smoothstep(0.05, 0.35, decal_a);
        rgb = mix(base_rgb, base_rgb * 0.55, groove);
        // Composite the decal colour on top of the darkened groove.
        rgb = mix(rgb, decal_rgb, decal_a);
    }

    // ── Point-light pass ────────────────────────────────────────────────
    // Accumulate candle / point-light contributions on top of the base
    // shade. Each light uses shared scene attenuation plus the shared
    // punctual diffuse receiver term so tiles match the other 3D receivers.
    // Lighting tints existing colour (rgb * contrib) instead of overwriting
    // it, so the tile's albedo still reads through.
    // glTF / OpenGL tangent-space normal (+Y up in TS); RGB linear unpacked.
    let nm = textureSample(normal_tex, base_sampler, in.uv_emr).rgb * 2.0 - 1.0;
    var Ngeom = normalize(in.wn);
    if (!front_facing) {
        Ngeom = -Ngeom;
    }
    let T = scene_safe_normalize(in.t_w.xyz, vec3<f32>(1.0, 0.0, 0.0));
    let B = scene_safe_normalize(cross(Ngeom, T), vec3<f32>(0.0, 1.0, 0.0)) * in.t_w.w;
    let n_world = scene_safe_normalize(nm.x * T + nm.y * B + nm.z * Ngeom, Ngeom);

    // Enhancement kind from tile_visual_params.z:
    //   0 = none, 1 = pearl, 2 = gilded, 3 = polychrome.
    let enh = in.tile_visual_params.z;
    let has_enh = enh > 0.5;

    // View direction from the actual camera position passed via uniform.
    let view_dir = normalize(frame.cam_pos - in.world_pos);
    let ndv_global = max(dot(n_world, view_dir), 0.0);

    var point_contrib = vec3<f32>(0.0);
    var sheen_acc = vec3<f32>(0.0);
    let boss_press = clamp(hd.flags.y, 0.0, 1.0);
    let punc_rgb_mul = mix(
        vec3<f32>(1.0),
        vec3<f32>(1.14, 0.42, 0.36),
        boss_press,
    );
    let light_count = lights.count.x;
    for (var i: u32 = 0u; i < light_count; i = i + 1u) {
        let pl = lights.lights[i];
        let point_sample = scene_pbr_sample_point_light(
            in.world_pos,
            pl.pos.xyz,
            pl.pos.w,
            vec4<f32>(pl.color.rgb * punc_rgb_mul, pl.color.a),
            pl.params.x,
            frame.tile_post_params.w,
        );
        if (length(point_sample.radiance) <= 0.0) {
            continue;
        }
        let l_dir = point_sample.direction;
        let radiance = point_sample.radiance;
        let nl_raw = dot(n_world, l_dir);
        let nl = max(nl_raw, 0.0);
        let lambert = scene_punctual_diffuse_weight(nl_raw);
        let punc_vis = punctual_shadow_vis(i, in.world_pos);
        let shadowed_radiance = radiance * punc_vis;
        point_contrib = point_contrib + shadowed_radiance * lambert;

        // ── Enhancement sheen lobes ────────────────────────────────────
        // Fresnel-masked specular highlights per enhancement type, matching
        // the talisman material language: pearl nacre, gold conductor,
        // polychrome holographic.
        if (has_enh) {
            let h = normalize(l_dir + view_dir);
            let nh = max(dot(n_world, h), 0.0);
            let vdh = max(dot(view_dir, h), 0.0);
            let ndv = max(dot(n_world, view_dir), 0.0);
            let broad = nl;

            if (enh < 1.5) {
                // Pearl: pearlescent nacre with pink-to-blue color shift.
                let fresnel = 0.08 + 0.40 * pow(1.0 - ndv, 3.0);
                let phase = ndv * 3.14159;
                let pearl_tint = vec3<f32>(
                    0.95 + 0.05 * cos(phase),
                    0.85 + 0.15 * cos(phase + 1.8),
                    0.90 + 0.10 * cos(phase + 2.8)
                );
                let lobe = pow(nh, 18.0) * 0.6 + broad * 0.15;
                sheen_acc = sheen_acc + shadowed_radiance * lobe * fresnel * pearl_tint;
            } else if (enh < 2.5) {
                // Gilded: metallic gold conductor — Schlick Fresnel tinted
                // by gold base so highlights read warm.
                let f0 = vec3<f32>(0.95, 0.75, 0.30);
                let f_gold = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vdh, 5.0);
                let lobe = pow(nh, 24.0) * 0.9 + broad * 0.08;
                sheen_acc = sheen_acc + shadowed_radiance * lobe * f_gold;
            } else {
                // Polychrome: holographic thin-film rainbow driven by
                // viewing angle + surface position.
                let theta = ndv * 6.2832 + dot(in.local_pos, vec3<f32>(8.0, 8.0, 8.0));
                let holo_r = 0.5 + 0.5 * cos(theta);
                let holo_g = 0.5 + 0.5 * cos(theta + 2.094);
                let holo_b = 0.5 + 0.5 * cos(theta + 4.189);
                let holo_tint = vec3<f32>(holo_r, holo_g, holo_b);
                let fresnel = 0.10 + 0.50 * pow(1.0 - ndv, 2.5);
                let lobe = pow(nh, 12.0) * 0.7 + broad * 0.18;
                sheen_acc = sheen_acc + shadowed_radiance * lobe * fresnel * holo_tint;
            }
        }
    }

    // ── Spotlights ───────────────────────────────────────────────────────
    // Same distance falloff as point lights, plus Khronos `KHR_lights_punctual`
    // angular attenuation. Spotlights fold
    // into `point_contrib` so downstream material composition (blocked-
    // tile dim, hover fresnel) treats them the same as candle light.
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
            vec4<f32>(s.color.rgb * punc_rgb_mul, s.color.a),
            0.0,
            frame.tile_post_params.w,
        );
        if (length(spot_sample.radiance) <= 0.0) {
            continue;
        }
        let lambert = scene_punctual_diffuse_weight(dot(n_world, spot_sample.direction));
        let spot_vis = 1.0;
        point_contrib = point_contrib + spot_sample.radiance * lambert * spot_vis;
    }

    // ── Enhancement fresnel albedo tint ─────────────────────────────────
    // View-dependent color shift baked into the surface so it reads as a
    // material property (always visible), not just a specular highlight.
    if (has_enh) {
        let edge = 1.0 - ndv_global;
        if (enh < 1.5) {
            // Pearl: cool iridescent white-pink shift at edges.
            let rim = pow(edge, 2.0) * 0.25;
            let phase = ndv_global * 3.14159;
            let pearl = vec3<f32>(0.95, 0.88 + 0.08 * cos(phase), 0.95);
            rgb = mix(rgb, pearl, rim);
        } else if (enh < 2.5) {
            // Gilded: warm gold rim.
            let rim = pow(edge, 2.0) * 0.25;
            rgb = mix(rgb, vec3<f32>(1.0, 0.90, 0.60), rim);
        } else {
            // Polychrome: rainbow fresnel shifts surface hue at edges.
            let rim = pow(edge, 1.5) * 0.35;
            let theta = ndv_global * 6.2832 + dot(in.local_pos, vec3<f32>(4.0, 4.0, 4.0));
            let holo = vec3<f32>(
                0.5 + 0.5 * cos(theta),
                0.5 + 0.5 * cos(theta + 2.094),
                0.5 + 0.5 * cos(theta + 4.189)
            );
            rgb = mix(rgb, holo, rim);
        }
    }

    // Per-light `punctual_shadow_vis` in the loop above — do not min all caster
    // frustums here (misaligns for multi-light maps and crushes celebration /
    // procedural scenes where `tile_post_params.w` is zero).
    var lit_rgb = rgb * point_contrib + sheen_acc;

    // glTF metallic–roughness + emissive (linear), sampled on `uv_emr`.
    // `tile_decal_atlas_uv.z` carries room emissive scale for imported shop/hallway only;
    // showcase `tile.glb` uses zw as decal atlas scale — keep multiplier at 1 there.
    var gltf_emissive_hdr = vec3<f32>(0.0);
    if (use_textured_albedo) {
        let mr_s = textureSample(metallic_roughness_tex, base_sampler, in.uv_emr);
        let metallic = clamp(mr_s.b * pbr.metallic_factor, 0.0, 1.0);
        let roughness = clamp(mr_s.g * pbr.roughness_factor, 0.04, 1.0);
        let emissive_base = textureSample(emissive_tex, base_sampler, in.uv_emr).rgb * pbr.emissive_factor.rgb;
        let emissive_scale = select(1.0, in.tile_decal_atlas_uv.z, use_textured_env);
        let emissive = emissive_base * emissive_scale;
        gltf_emissive_hdr = emissive;
        lit_rgb = lit_rgb * (1.0 - metallic * 0.78);
        lit_rgb = lit_rgb + emissive;
    }

    // ── Blocked-tile dimming (solitaire) ───────────────────────────
    // tile_visual_params.x: 1.0 = free/playable, <1.0 = blocked.
    // Desaturate toward luminance then scale down so blocked tiles
    // read as inert stone without becoming illegible.
    let brightness = in.tile_visual_params.x;
    if (brightness < 0.99) {
        let lum = dot(lit_rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        // Staging / blocked tiles: extra desaturate below ~0.35 so previews read ghostly.
        let desat = select(0.35, 0.72, brightness < 0.35);
        lit_rgb = mix(lit_rgb, vec3<f32>(lum), desat) * brightness;
    }

    // ── Hover / selection fresnel ───────────────────────────────────
    // tile_visual_params.y: 0.0 = none, 0.5 = hovered, 1.0 = selected.
    // Hover: saturated electric-blue rim (thin, tight).
    // Selected: warm champagne-gold rim (wider, brighter).
    let sel = in.tile_visual_params.y;
    if (sel > 0.25) {
        let edge = 1.0 - ndv_global;
        if (sel < 0.75) {
            // Hover — strong blue (matches tile_outline shell).
            let cool = vec3<f32>(0.06, 0.42, 1.00);
            let rim = pow(edge, 3.5) * 1.05;
            lit_rgb = lit_rgb + cool * rim;
        } else {
            // Selected — warm gold fresnel, wider.
            let gold = vec3<f32>(1.00, 0.84, 0.42);
            let rim = pow(edge, 2.5) * 1.2;
            lit_rgb = lit_rgb + gold * rim;
        }
    }

    let inv_g = 1.0 / max(lights.extras.x, 0.01);
    var out_rgb: vec3<f32>;
    if (frame.tile_post_params.x > 0.5) {
        // Table / room linear HDR path: write the un-tonemapped HDR into
        // `scene_color`. `tonemap_composite.wgsl` applies the single ACES
        // pass + sRGB encode; the per-shader `lights.extras.x` gamma slider
        // is intentionally a no-op here.
        //
        // `tile_post_params.y` crushes punctual-lit albedo (gameplay feel).
        // glTF emissive is authored as outgoing radiance — if it goes through the same multiplier,
        // bright point lights on the same mesh (e.g. hallway lamp bulbs) swamp it and changing
        // emissive scale is invisible. Keep emissive out of that multiply (same idea as
        // `room_glb.wgsl`: emissive is not scaled by room linear exposure).
        let hem = frame.tile_post_params.z * rgb * vec3<f32>(0.08);
        var hdr = (lit_rgb - gltf_emissive_hdr + hem) * frame.tile_post_params.y;
        hdr = hdr + gltf_emissive_hdr;
        // Clamp to prevent Rgba16Float overflow (Infinity) which causes NaN during bloom bilinear filtering on Metal
        out_rgb = min(hdr, vec3<f32>(65000.0));
    } else {
        // Legacy non-HDR scenes still apply the user gamma slider in-shader.
        out_rgb = pow(lit_rgb, vec3<f32>(inv_g));
    }

    // Staging meld previews (per-instance opacity < 1): cool additive ghost keyed
    // off decal albedo — standard alpha left only specular highlights visible.
    let ghost_opacity = clamp(in.tile_seed_opacity.y, 0.0, 1.0);
    if (ghost_opacity < 0.999) {
        let decal_body = mix(vec3<f32>(0.20, 0.30, 0.46), rgb, 0.78);
        let lum = dot(out_rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        let soft_lit = mix(vec3<f32>(lum), out_rgb, 0.32);
        let ghost_rgb = mix(decal_body, soft_lit, 0.40) + vec3<f32>(0.06);
        let ghost_a = ghost_opacity * 0.48;
        return vec4(clamp(ghost_rgb, vec3<f32>(0.0), vec3<f32>(0.68)), ghost_a);
    }
    out_alpha = out_alpha * ghost_opacity;
    return vec4<f32>(out_rgb, out_alpha);
}
