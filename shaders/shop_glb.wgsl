// Shop.glb environment — glTF-style punctual lights + metallic–roughness + ACES.
// Separate from `tile_3d.wgsl` (candle pools + artistic lambert floor).
//
// Uniform hacks (same `CameraUniform` layout as tiles; shop writer only):
// - `tile_seed`     = linear HDR exposure multiplier before tonemap
// - `decal_atlas_uv.x` = ambient scale (0 = punctual-only interior)
//
// Point / spot `pos.w` = max light distance in **world units** (`KHR_lights_punctual` range),
// or `0` for infinite range (pure inverse-square with a minimum distance clamp).

const PI: f32 = 3.14159265358979323846;

struct CameraUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    base_color_factor: vec4<f32>,
    cam_pos: vec3<f32>,
    tile_seed: f32,
    decal_atlas_uv: vec4<f32>,
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

struct PointLight {
    pos: vec4<f32>,
    color: vec4<f32>,
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

/// `KHR_lights_punctual` distance attenuation (inverse square × smooth range window).
fn punctual_attenuation(distance: f32, range_max: f32) -> f32 {
    let d = max(distance, 1e-4);
    var att = 1.0 / (d * d);
    if range_max > 1e-5 {
        let x = min(d / range_max, 1.0);
        let window = max(1.0 - pow(x, 4.0), 0.0);
        att = att * window;
    }
    return att;
}

fn aces_fitted(color: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp(
        (color * (a * color + b)) / (color * (c * color + d) + e),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
}

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
    let world = cam.model * vec4<f32>(pos, 1.0);
    let N = normalize((cam.model * vec4<f32>(n, 0.0)).xyz);
    let Tw = (cam.model * vec4<f32>(tangent.xyz, 0.0)).xyz;
    let Torth = normalize(Tw - N * dot(N, Tw));
    let Borth = normalize(cross(N, Torth)) * tangent.w;

    var o: VsOut;
    o.clip_pos = cam.view_proj * world;
    o.wn = N;
    o.uv = uv;
    o.local_pos = pos;
    o.local_n = n;
    o.world_pos = world.xyz;
    o.t_w = Torth;
    o.b_w = Borth;
    o.uv_emr = uv_emr_in;
    o.v_color = v_color_in;
    return o;
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let base_s = textureSample(base_color, base_sampler, in.uv);
    let tex_a = base_s.a * in.v_color.a;
    if (pbr.alpha_mode == 1u) {
        if (tex_a < pbr.alpha_cutoff) {
            discard;
        }
    }
    var out_alpha: f32 = 1.0;
    if (pbr.alpha_mode == 2u) {
        out_alpha = tex_a;
    }
    let albedo = base_s.rgb * in.v_color.rgb;

    let nm = textureSample(normal_tex, base_sampler, in.uv_emr).rgb * 2.0 - 1.0;
    var Ngeom = normalize(in.wn);
    if (!front_facing) {
        Ngeom = -Ngeom;
    }
    let T = normalize(in.t_w);
    let B = normalize(in.b_w);
    let n_world = normalize(nm.x * T + nm.y * B + nm.z * Ngeom);

    let mr_s = textureSample(metallic_roughness_tex, base_sampler, in.uv_emr);
    let metallic = clamp(mr_s.b * pbr.metallic_factor, 0.0, 1.0);
    let roughness = clamp(mr_s.g * pbr.roughness_factor, 0.04, 1.0);
    let emissive =
        textureSample(emissive_tex, base_sampler, in.uv_emr).rgb * pbr.emissive_factor.rgb;

    let V = normalize(cam.cam_pos - in.world_pos);
    let NdotV = max(dot(n_world, V), 1e-4);
    let F0 = mix(vec3<f32>(0.04), albedo, metallic);

    var Lo = vec3<f32>(0.0);

    let light_count = lights.count.x;
    for (var i: u32 = 0u; i < light_count; i = i + 1u) {
        let pl = lights.lights[i];
        let light_pos = pl.pos.xyz;
        let range_w = pl.pos.w;
        let to_light = light_pos - in.world_pos;
        let dist = length(to_light);
        let L = to_light / max(dist, 1e-4);
        let atten = punctual_attenuation(dist, range_w);
        let radiance = pl.color.rgb * pl.color.a * atten;
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
        let atten_spot = punctual_attenuation(dist, range_spot);
        if (atten_spot <= 0.0) {
            continue;
        }
        let L = -to_frag / max(dist, 1e-4);
        let frag_dir = to_frag / max(dist, 1e-4);
        let cos_a = dot(frag_dir, s.dir.xyz);
        let cos_outer = s.dir.w;
        let cos_inner = s.params.x;
        let spot_factor = smoothstep(cos_outer, cos_inner, cos_a);
        if (spot_factor <= 0.0) {
            continue;
        }
        let radiance = s.color.rgb * s.color.a * atten_spot * spot_factor;
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
    let ambient = ambient_scale * albedo * (1.0 - metallic) * vec3<f32>(0.08);

    var hdr = ambient + Lo + emissive;
    hdr = hdr * cam.tile_seed;

    // Gameplay shadow map is ortho-fit around the mahjong table near the origin.
    // Shop.glb uses the same Z-up frame but geometry is scaled by `window_h`, so
    // light-space coverage is meaningless here and PCF often zeros the room.
    let gameplay_shadow = sample_shadow_visibility(in.world_pos);
    // `clamp(gs+1,1,2) - gs` is always 1.0 but keeps the shadow sample live for the layout.
    let shadow_neutral =
        clamp(gameplay_shadow + 1.0, 1.0, 2.0) - gameplay_shadow;
    hdr = hdr * shadow_neutral;

    let mapped = aces_fitted(hdr);
    let inv_g = 1.0 / max(lights.extras.x, 0.01);
    let out_rgb = pow(mapped, vec3<f32>(inv_g));
    return vec4<f32>(out_rgb, out_alpha);
}
