struct CameraUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    base_color_factor: vec4<f32>,
    cam_pos: vec3<f32>,
    // Per-tile seed written by the renderer (slot index as f32). Used to
    // offset procedural noise so each tile's pattern is unique — currently
    // only sampled by the tortoise-shell branch in `tortoise_albedo`.
    tile_seed: f32,
    // Showcase decal atlas: xy = origin in normalized atlas coords, zw = scale per axis.
    decal_atlas_uv: vec4<f32>,
    /// x = ACES HDR path on; y = linear exposure; z = hemispheric ambient (albedo * 0.08);
    /// w = inverse document scale for embedded glTF punctual attenuation.
    hdr_tonemap: vec4<f32>,
};

// ACES tonemapping is applied once in `tonemap_composite.wgsl`. This shader
// writes linear HDR to `scene_color` (`Rgba16Float`).

@group(0) @binding(0) var<uniform> cam: CameraUniform;
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
    _pad1_0: u32,
    _pad1_1: u32,
    _pad1_2: u32,
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
    // extras.x = display gamma; extras.w = inverse-square scale for procedural meshes when embedded GLB.
    extras: vec4<f32>,
    lights: array<PointLight, 16>,
};

@group(1) @binding(0) var<uniform> lights: PointLights;

// ── Spotlights (group 3) ─────────────────────────────────────────────
// Directional cone lights used for focused visual highlights (e.g. hint
// indicators pooling green on a specific tile). Only sampled by the tile
// pipeline — candles/table/smoke do not receive spotlight contribution.
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

// ── Shadow sampling (group 2, shared frame-wide) ─────────────────────
struct PunctualShadowSlot {
    light_view_proj: mat4x4<f32>,
    atlas_rect: vec4<f32>,
};

struct ShadowGlobals {
    light_view_proj: mat4x4<f32>,
    // x = enabled (0/1), y = depth bias, z = texel size, w = unused
    params: vec4<f32>,
    room_baked_light_view_proj: mat4x4<f32>,
    punctual_params: vec4<f32>,
    punctual_lights: array<PunctualShadowSlot, 8>,
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

// ── Procedural bamboo helpers ───────────────────────────────────────────
// Cheap value-noise primitives used to build a bamboo wood-fiber texture
// for the tile sides. Bamboo's defining traits are (a) long parallel
// fibers running along the cane's growth axis — visible as fine vertical
// striations on side faces — and (b) circular fiber cross-sections on
// end-grain faces, which read as a cluster of darker dots in a lighter
// matrix. Each tile-mesh fiber runs along local X (the tile's long axis).
fn hash21(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453);
}

fn vnoise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Returns a bamboo albedo for a fragment whose surface lies on the tile
// body. `local_pos` is in normalised mesh space (X≈[-0.5,0.5] long axis,
// Y≈[-0.21,0.21] thickness, Z≈[-0.37,0.37] short axis). `local_n` is the
// raw mesh normal so we can detect end-grain vs side-grain faces.
fn bamboo_albedo(local_pos: vec3<f32>, local_n: vec3<f32>) -> vec3<f32> {
    // Warm cream fiber colour and a deeper amber for the matrix between
    // fibers / for end-grain. Tuned to sit slightly cooler than the ivory
    // tile face so the bamboo body reads as a separate material.
    let fiber_light = vec3<f32>(0.93, 0.83, 0.55);
    let fiber_dark  = vec3<f32>(0.66, 0.50, 0.26);
    let end_base    = vec3<f32>(0.46, 0.32, 0.16);

    // ── Side-grain (±Y and ±Z faces) ────────────────────────────────────
    // Fibers run along local X. The "across-fiber" coordinate is whichever
    // tangent axis isn't X — Z on top/bottom faces, Y on left/right faces.
    let across = select(local_pos.y, local_pos.z, abs(local_n.y) > abs(local_n.z));

    // Stretch the noise *heavily* along the fiber direction so each
    // value-noise cell becomes a long thin streak — that's the parallel
    // fiber look. Two octaves give the fibers some thickness variance.
    let n1 = vnoise2(vec2<f32>(local_pos.x * 4.0, across * 70.0));
    let n2 = vnoise2(vec2<f32>(local_pos.x * 1.5, across * 140.0));
    let fiber_pattern = n1 * 0.65 + n2 * 0.35;

    // A faint sinusoidal striping reinforces the long-grain feel between
    // noise cells, especially under raking candlelight.
    let stripes = sin(across * 220.0) * 0.5 + 0.5;
    let side_t = clamp(fiber_pattern * 0.75 + stripes * 0.25, 0.0, 1.0);

    // Subtle along-length tonal variation (warmer/cooler bands).
    let length_var = vnoise2(vec2<f32>(local_pos.x * 6.0, across * 4.0));
    var side_rgb = mix(fiber_dark, fiber_light, side_t);
    side_rgb = side_rgb * (0.92 + 0.16 * length_var);

    // ── End-grain (±X faces) ────────────────────────────────────────────
    // Cross-section through the fibers: cluster of darker circular fiber
    // bundles in a lighter matrix. Use 2D noise on (Y, Z) at high
    // frequency, then sharpen with a power curve to make the dark cores
    // sparse and well-defined.
    let end_n = vnoise2(vec2<f32>(local_pos.y * 110.0, local_pos.z * 110.0));
    let end_n2 = vnoise2(vec2<f32>(local_pos.y * 45.0 + 7.3, local_pos.z * 45.0 - 2.1));
    let dots = pow(end_n, 2.6);
    var end_rgb = mix(end_base * 1.55, end_base * 0.55, dots);
    // Slight large-scale warmth variation across the end face.
    end_rgb = end_rgb * (0.90 + 0.20 * end_n2);

    // Blend the two based on how end-grain the surface is. Use a soft
    // smoothstep so beveled corners get a believable transition rather
    // than a hard seam.
    let end_grain = smoothstep(0.55, 0.92, abs(local_n.x));
    return mix(side_rgb, end_rgb, end_grain);
}

// ── Procedural tortoise shell (blonde bekko / hawksbill keratin) ───────
// Real bekko: translucent golden-amber ground with irregular coffee-dark
// “islands”, caramel halos where the horn thins at patch edges, fine growth
// veins, and a wax-polished top. Domain-warped FBM breaks axis-aligned smears;
// `local_n` adds subtle streak polish on upward-facing facets.
fn tortoise_albedo(local_pos: vec3<f32>, seed: f32, local_n: vec3<f32>) -> vec3<f32> {
    // Strong per-instance fingerprints — hash(seed) shifts thresholds and tone so
    // neighbouring ids (and large preview ids) don't read as copy-paste slabs.
    let seed_j = vec2<f32>(seed * 19.13, seed * 17.71);
    let sh = hash21(seed_j);
    let sh2 = hash21(seed_j + vec2<f32>(13.2, 8.9));
    let sh3 = hash21(seed_j + vec2<f32>(2.1, 31.4));
    let seed_offset = vec2<f32>(seed * 17.37, seed * 11.91);
    let p0 = vec2<f32>(local_pos.x, local_pos.z);

    // Domain warp — blobby organic islands instead of rectangular noise cells.
    let warp1 = vec2<f32>(
        vnoise2(p0 * 2.9 + seed_offset),
        vnoise2(p0 * 3.2 + seed_offset + vec2<f32>(41.2, 18.7)),
    ) - vec2<f32>(0.5, 0.5);
    let pw = p0 + warp1 * 0.12;
    let warp2 = vec2<f32>(
        vnoise2(pw * 6.0 + vec2<f32>(2.1, 9.4)),
        vnoise2(pw * 5.8 + vec2<f32>(27.0, 3.3)),
    ) - vec2<f32>(0.5, 0.5);
    let pw2 = pw + warp2 * 0.048;

    // FBM on warped coords — large honey fields with medium-scale breakup.
    let f1 = vnoise2(pw2 * vec2<f32>(4.2, 5.0));
    let f2 = vnoise2(pw2 * vec2<f32>(10.5, 12.5) + vec2<f32>(8.3, 2.1));
    let f3 = vnoise2(pw2 * vec2<f32>(22.0, 26.0) + vec2<f32>(1.7, 16.2));
    let fbm = f1 * 0.48 + f2 * 0.38 + f3 * 0.14;

    let macro_bias = vnoise2(p0 * 1.55 + seed_offset * 0.48);
    var field = clamp(mix(fbm, fbm * 0.65 + macro_bias * 0.35, 0.28), 0.0, 1.0);
    // Tile-unique low-frequency bias so blot layout isn't just a translated copy.
    field = clamp(field + (sh - 0.5) * 0.22 + (sh2 - 0.5) * 0.14, 0.0, 1.0);

    // Dark patch cores + softer outer rim (two thresholds).
    let thr_j = (sh - 0.5) * 0.09 + (sh2 - 0.5) * 0.06;
    let patch_soft = smoothstep(0.44 + thr_j, 0.76 + thr_j, field);
    let patch_dark = smoothstep(0.56 + thr_j * 0.85, 0.88 + thr_j * 0.85, field);

    // Amber ground (layered warm tones, not flat).
    let honey_hi = vec3<f32>(0.86, 0.62, 0.24);
    let honey_mid = vec3<f32>(0.72, 0.46, 0.16);
    let honey_lo = vec3<f32>(0.58, 0.34, 0.11);
    var rgb = mix(honey_lo, honey_hi, f1 * 0.55 + f2 * 0.35);
    rgb = mix(rgb, honey_mid, f3 * 0.22);

    let blot_outer = vec3<f32>(0.26, 0.11, 0.06);
    let blot_core = vec3<f32>(0.11, 0.045, 0.025);
    rgb = mix(rgb, blot_outer, patch_soft * 0.72);
    rgb = mix(rgb, blot_core, patch_dark * 0.78);

    // Per-tile amber cast — readable on the porcelain strip and top face.
    let warm_t = 0.94 + sh3 * 0.12;
    let cool_b = 1.02 - sh3 * 0.10;
    rgb = rgb * vec3<f32>(warm_t, mix(1.0, 1.05, sh3), cool_b);

    // Caramel “halos” at patch margins (thin horn reads lighter).
    let halo_band = smoothstep(0.38, 0.52, field) * (1.0 - smoothstep(0.68, 0.86, field));
    let halo_rgb = vec3<f32>(0.90, 0.64, 0.26);
    rgb = mix(rgb, halo_rgb, halo_band * 0.42);

    // Sparse fine veins / growth lines on top of the mottle.
    let vein_n = vnoise2(pw2 * vec2<f32>(52.0, 56.0));
    let veins = pow(clamp(vein_n, 0.0, 1.0), 11.0);
    rgb = rgb * (1.0 - veins * 0.28) + blot_core * veins * 0.85;

    // Through-thickness translucency: cap paler, base richer (local Y ~ slab thickness).
    let depth_t = smoothstep(-0.21, 0.17, local_pos.y);
    let pale_cap = vec3<f32>(0.93, 0.74, 0.36);
    rgb = mix(rgb, pale_cap, depth_t * 0.24);

    // Subtle polish streaks on faces that face +Y (tile top / bevel flats).
    let up_w = smoothstep(0.55, 0.95, local_n.y);
    let streak = sin(dot(p0, vec2<f32>(19.2, 23.8)) + seed * 1.9) * 0.5 + 0.5;
    rgb = mix(rgb, rgb * vec3<f32>(1.05, 1.03, 0.97), up_w * streak * 0.10);

    return clamp(rgb, vec3<f32>(0.02), vec3<f32>(1.0));
}

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) wn: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) local_pos: vec3<f32>,
    @location(3) local_n: vec3<f32>,
    @location(4) world_pos: vec3<f32>,
    /// World-space tangent (orthogonalized to vertex normal).
    @location(5) t_w: vec3<f32>,
    /// World-space bitangent (`cross(N, T) * handedness`).
    @location(6) b_w: vec3<f32>,
    /// UV for normal / metallic-roughness / emissive (glTF secondary TEXCOORD when present).
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

    // cam.base_color_factor.w — see `tile_body.rs`:
    //   0–2 = procedural tile body (`TileBodyShaderKind`),
    //   4 = sample bound base-color texture, no decal projection (shop room),
    //   5 = sample bound base-color per primitive + mahjong decal on **Face** material only.
    let body_kind = cam.base_color_factor.w;
    let use_textured_env = body_kind > 3.5 && body_kind < 4.5;
    let use_textured_tile_glb = body_kind > 4.5 && body_kind < 5.5;
    let use_textured_albedo = use_textured_env || use_textured_tile_glb;
    let is_plastic = !use_textured_albedo && body_kind > 0.5 && body_kind < 1.5;
    let is_tortoise = !use_textured_albedo && body_kind > 1.5 && body_kind < 2.5;

    let ivory_band_softness: f32 = 0.012; // smooth transition (~1 mm)

    var base_rgb: vec3<f32>;
    /// Fragment alpha (blend mode only for textured glTF paths).
    var out_alpha: f32 = 1.0;
    if (use_textured_albedo) {
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
    } else {
        // Real mahjong tiles are a thin ivory/bone face layer glued onto a
        // bamboo body — the ivory wraps around the top of the side bevels
        // for a couple of millimetres before the bamboo grain takes over.
        // Plastic caps are 40% of the slab thickness; tortoise shell has no cap.
        var ivory_layer_y: f32 = 0.172;
        if (is_plastic) { ivory_layer_y = 0.042; }
        if (is_tortoise) { ivory_layer_y = 0.172; }
        let ivory_band = smoothstep(
            ivory_layer_y - ivory_band_softness,
            ivory_layer_y + ivory_band_softness,
            in.local_pos.y,
        ) * select(0.0, 1.0, in.local_n.y > -0.25);

        // ── Bamboo & Ivory (mat 0) ──────────────────────────────────────
        let ivory = vec3<f32>(0.96, 0.93, 0.84);
        let bamboo = bamboo_albedo(in.local_pos, normalize(in.local_n));
        let bamboo_body = mix(bamboo, ivory, ivory_band);
        let bamboo_rgb = select(bamboo_body, ivory, is_front);

        // ── Plastic (mat 1) ─────────────────────────────────────────────
        let plastic_face = vec3<f32>(0.97, 0.97, 0.96);
        let kelly = vec3<f32>(0.0, 0.35, 0.18);
        let depth_t = smoothstep(-0.21, 0.17, in.local_pos.y);
        let translucent_lighten = vec3<f32>(0.18, 0.22, 0.14) * depth_t;
        let plastic_body_base = kelly + translucent_lighten;
        let pn = vnoise2(vec2<f32>(in.local_pos.x * 12.0, in.local_pos.z * 12.0));
        let plastic_body = plastic_body_base * (0.96 + 0.08 * pn);
        let plastic_side = mix(plastic_body, plastic_face, ivory_band);
        let plastic_rgb = select(plastic_side, plastic_face, is_front);

        // ── Tortoise shell (mat 2) ──────────────────────────────────────
        let tortoise_body = tortoise_albedo(in.local_pos, cam.tile_seed, normalize(in.local_n));
        let honey_mean = vec3<f32>(0.72, 0.48, 0.18);
        // Keep most of the shell mottle on the face — heavy flattening made every tile read identical.
        let tortoise_face = mix(tortoise_body, honey_mean, 0.06);
        let tortoise_side = mix(tortoise_body, tortoise_face, ivory_band);
        let tortoise_rgb = select(tortoise_side, tortoise_face, is_front);

        base_rgb = bamboo_rgb;
        if (is_plastic) { base_rgb = plastic_rgb; }
        if (is_tortoise) { base_rgb = tortoise_rgb; }
    }

    // Tile decal projection + groove normals — skipped for shop env (kind 4) only.
    var carve_dhdu = 0.0;
    var carve_dhdv = 0.0;
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
        let decal_uv = decal_uv_face * cam.decal_atlas_uv.zw + cam.decal_atlas_uv.xy;
        let decal = textureSample(decal_tex, base_sampler, decal_uv);
        let in_uv = decal_uv_face.x >= 0.0 && decal_uv_face.x <= 1.0 && decal_uv_face.y >= 0.0 && decal_uv_face.y <= 1.0;
        // Imported tile meshes: decal only on the authored **Face** material (`v_color.a`).
        // Do not fall back to procedural `is_front` on body / side-band primitives.
        let decal_face = select(is_front, use_glb_face, use_textured_tile_glb);
        let decal_a = select(0.0, decal.a, decal_face && in_uv);
        let decal_rgb = decal.rgb;

        // ── Carved-groove engraving (same technique as plaque text) ─────────
        // Treat decal alpha as a heightmap: 0 = flush ivory/plastic surface,
        // 1 = bottom of the carved channel. Finite-difference gradient gives
        // groove-wall normals that catch candlelight from one side and shadow
        // the other, exactly like CNC-routed tile faces.
        if (decal_face && in_uv) {
            let dim_d = vec2<f32>(textureDimensions(decal_tex, 0));
            let tx = vec2<f32>(1.0 / max(dim_d.x, 1.0), 1.0 / max(dim_d.y, 1.0));
            let a_l = textureSampleLevel(decal_tex, base_sampler, decal_uv + vec2<f32>(-tx.x, 0.0), 0.0).a;
            let a_r = textureSampleLevel(decal_tex, base_sampler, decal_uv + vec2<f32>( tx.x, 0.0), 0.0).a;
            let a_d = textureSampleLevel(decal_tex, base_sampler, decal_uv + vec2<f32>(0.0, -tx.y), 0.0).a;
            let a_u = textureSampleLevel(decal_tex, base_sampler, decal_uv + vec2<f32>(0.0,  tx.y), 0.0).a;
            let carve_bump = 3.0;
            carve_dhdu = (a_r - a_l) * carve_bump;
            carve_dhdv = (a_u - a_d) * carve_bump;
        }

        // Groove-floor darkening: the carved recess is slightly shadowed by
        // the groove walls before the paint/ink is laid in.
        let groove = smoothstep(0.05, 0.35, decal_a);
        rgb = mix(base_rgb, base_rgb * 0.55, groove);
        // Composite the decal colour on top of the darkened groove.
        rgb = mix(rgb, decal_rgb, decal_a);
    }

    // ── Point-light pass ────────────────────────────────────────────────
    // Accumulate candle / point-light contributions on top of the base
    // shade. Each light uses a smooth quadratic falloff plus a Lambertian
    // term against the world-space normal so the side bevel facing a candle
    // gets the most warmth. Lighting tints existing colour (rgb * contrib)
    // instead of overwriting it, so the tile's albedo still reads through.
    var n_world: vec3<f32>;
    if (use_textured_albedo) {
        // glTF / OpenGL tangent-space normal (+Y up in TS); RGB linear unpacked.
        let nm = textureSample(normal_tex, base_sampler, in.uv_emr).rgb * 2.0 - 1.0;
        var Ngeom = normalize(in.wn);
        if (!front_facing) {
            Ngeom = -Ngeom;
        }
        let T = normalize(in.t_w);
        let B = normalize(in.b_w);
        n_world = normalize(nm.x * T + nm.y * B + nm.z * Ngeom);
    } else {
        n_world = normalize(in.wn);
        if (!front_facing) {
            n_world = -n_world;
        }
        // Carved-groove normal perturbation from decal alpha gradient.
        let edge_mag = abs(carve_dhdu) + abs(carve_dhdv);
        if (edge_mag > 0.001) {
            let perturbed_local = normalize(vec3<f32>(-carve_dhdu, 1.0, -carve_dhdv));
            let perturbed_world = normalize((cam.model * vec4<f32>(perturbed_local, 0.0)).xyz);
            let blend_edge = clamp(edge_mag * 1.5, 0.0, 1.0);
            n_world = normalize(mix(n_world, perturbed_world, blend_edge));
        }
    }

    // Enhancement kind from base_color_factor.z:
    //   0 = none, 1 = pearl, 2 = gilded, 3 = polychrome.
    let enh = cam.base_color_factor.z;
    let has_enh = enh > 0.5;

    // View direction from the actual camera position passed via uniform.
    let view_dir = normalize(cam.cam_pos - in.world_pos);
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
        let lp = lights.lights[i].pos.xyz;
        let radius = lights.lights[i].pos.w;
        let kind = lights.lights[i].params.x;
        let lc = lights.lights[i].color.rgb * punc_rgb_mul;
        let intensity = lights.lights[i].color.a * select(1.0, lights.extras.w, kind > 0.5);
        let to_light = lp - in.world_pos;
        let dist = length(to_light);
        let inv_doc = cam.hdr_tonemap.w;
        let atten = select(
            scene_smooth_point_atten(dist, radius),
            punctual_attenuation_with_inv_doc_scale(dist, radius, inv_doc),
            kind > 0.5,
        );
        let l_dir = to_light / max(dist, 0.0001);
        let nl = max(dot(n_world, l_dir), 0.0);
        // 0.35 ambient floor so even back-facing fragments warm up a little
        // (matches how a real candle bounces off the table around a tile).
        let lambert = 0.35 + 0.65 * nl;
        let punc_vis = punctual_shadow_vis(i, in.world_pos);
        point_contrib = point_contrib + lc * intensity * atten * lambert * punc_vis;

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
                sheen_acc = sheen_acc + lc * intensity * atten * lobe * fresnel * pearl_tint;
            } else if (enh < 2.5) {
                // Gilded: metallic gold conductor — Schlick Fresnel tinted
                // by gold base so highlights read warm.
                let f0 = vec3<f32>(0.95, 0.75, 0.30);
                let f_gold = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vdh, 5.0);
                let lobe = pow(nh, 24.0) * 0.9 + broad * 0.08;
                sheen_acc = sheen_acc + lc * intensity * atten * lobe * f_gold;
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
                sheen_acc = sheen_acc + lc * intensity * atten * lobe * fresnel * holo_tint;
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
        let to_frag = in.world_pos - s.pos.xyz;
        let dist = length(to_frag);
        let radius = max(s.pos.w, 1.0);
        let t = clamp(1.0 - dist / radius, 0.0, 1.0);
        let atten = t * t;
        if (atten <= 0.0) {
            continue;
        }
        let to_light = -to_frag / max(dist, 0.0001);
        let frag_dir = to_frag / max(dist, 0.0001);
        let cos_a = dot(frag_dir, s.dir.xyz);
        let cos_outer = s.dir.w;
        let cos_inner = s.params.x;
        let spot_factor = khr_spot_angle_attenuation_scene(cos_a, cos_inner, cos_outer);
        if (spot_factor <= 0.0) {
            continue;
        }
        let nl = max(dot(n_world, to_light), 0.0);
        let lambert = 0.35 + 0.65 * nl;
        point_contrib = point_contrib
            + s.color.rgb * punc_rgb_mul * s.color.a * atten * spot_factor * lambert;
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

    // Tile albedo × candle/spot contribution, modulated by the mesh shadow
    // map so casters darken tiles on the table like the lit-mesh path.
    // `use_textured_env` = imported shop room (`shop.glb`): same table shadow map
    // mismatch as `room_glb.wgsl` — skip gameplay shadow for that path only.
    // Embedded glTF punctual rooms already carry their authored room/baked shadows; procedural
    // tiles use the candle/spot lights directly and skip the directional receiver map so they
    // do not self-shadow into hard black under the room frustum.
    let mesh_shadow_vis = sample_shadow_visibility(in.world_pos);
    let embedded_gltf_punctual = cam.hdr_tonemap.w > 1e-8;
    let mesh_shadow = select(mesh_shadow_vis, 1.0, use_textured_env || embedded_gltf_punctual);
    var lit_rgb = (rgb * point_contrib + sheen_acc) * mesh_shadow;

    // Tortoise shell: warm amber Fresnel rim at grazing angles.
    // Scaled by the local candle contribution so the rim only blooms
    // where a wick is actually lighting the tile — keeps dark tiles dark.
    if (is_tortoise) {
        let edge = 1.0 - ndv_global;
        let rim = pow(edge, 3.0) * 0.35;
        let rim_tint = vec3<f32>(0.95, 0.60, 0.22);
        lit_rgb = lit_rgb + rim_tint * rim * point_contrib * mesh_shadow;
    }

    // glTF metallic–roughness + emissive (linear), sampled on `uv_emr`.
    // `decal_atlas_uv.z` carries room emissive scale for imported shop/hallway only;
    // showcase `tile.glb` uses zw as decal atlas scale — keep multiplier at 1 there.
    var gltf_emissive_hdr = vec3<f32>(0.0);
    if (use_textured_albedo) {
        let mr_s = textureSample(metallic_roughness_tex, base_sampler, in.uv_emr);
        let metallic = clamp(mr_s.b * pbr.metallic_factor, 0.0, 1.0);
        let emissive_base = textureSample(emissive_tex, base_sampler, in.uv_emr).rgb * pbr.emissive_factor.rgb;
        let emissive_scale = select(1.0, cam.decal_atlas_uv.z, use_textured_env);
        let emissive = emissive_base * emissive_scale;
        gltf_emissive_hdr = emissive;
        lit_rgb = lit_rgb * (1.0 - metallic * 0.78);
        lit_rgb = lit_rgb + emissive;
    }

    // ── Blocked-tile dimming (solitaire) ───────────────────────────
    // base_color_factor.x: 1.0 = free/playable, <1.0 = blocked.
    // Desaturate toward luminance then scale down so blocked tiles
    // read as inert stone without becoming illegible.
    let brightness = cam.base_color_factor.x;
    if (brightness < 0.99) {
        let lum = dot(lit_rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        lit_rgb = mix(lit_rgb, vec3<f32>(lum), 0.35) * brightness;
    }

    // ── Hover / selection fresnel ───────────────────────────────────
    // base_color_factor.y: 0.0 = none, 0.5 = hovered, 1.0 = selected.
    // Hover: saturated electric-blue rim (thin, tight).
    // Selected: warm champagne-gold rim (wider, brighter).
    let sel = cam.base_color_factor.y;
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
    if (cam.hdr_tonemap.x > 0.5) {
        // Table / room linear HDR path: write the un-tonemapped HDR into
        // `scene_color`. `tonemap_composite.wgsl` applies the single ACES
        // pass + sRGB encode; the per-shader `lights.extras.x` gamma slider
        // is intentionally a no-op here.
        //
        // `hdr_tonemap.y` crushes punctual-lit albedo (gameplay feel).
        // glTF emissive is authored as outgoing radiance — if it goes through the same multiplier,
        // bright point lights on the same mesh (e.g. hallway lamp bulbs) swamp it and changing
        // emissive scale is invisible. Keep emissive out of that multiply (same idea as
        // `room_glb.wgsl`: emissive is not scaled by `tile_seed`).
        let hem = cam.hdr_tonemap.z * rgb * vec3<f32>(0.08);
        var hdr = (lit_rgb - gltf_emissive_hdr + hem) * cam.hdr_tonemap.y;
        hdr = hdr + gltf_emissive_hdr;
        out_rgb = hdr;
    } else {
        // Legacy non-HDR scenes still apply the user gamma slider in-shader.
        out_rgb = pow(lit_rgb, vec3<f32>(inv_g));
    }
    return vec4<f32>(out_rgb, out_alpha);
}
