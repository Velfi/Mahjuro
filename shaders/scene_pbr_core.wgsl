// Binding-free scene lighting helpers shared by room, tile, prop, and offline
// bake shaders.
const SCENE_PI: f32 = 3.141592653589793;
const SCENE_INV_PI: f32 = 0.3183098861837907;

fn khr_spot_angle_attenuation_scene(cos_a: f32, cos_inner: f32, cos_outer: f32) -> f32 {
    let den = max(cos_inner - cos_outer, 1e-3);
    let scale = 1.0 / den;
    let offset = -cos_outer * scale;
    let angular = clamp(cos_a * scale + offset, 0.0, 1.0);
    return angular * angular;
}

/// `KHR_lights_punctual` distance attenuation (inverse square x smooth range window).
fn punctual_attenuation_khr(distance: f32, range_max: f32) -> f32 {
    let d = max(distance, 1e-4);
    var att = 1.0 / (d * d);
    if (range_max > 1e-5) {
        let x = min(d / range_max, 1.0);
        let window = max(1.0 - pow(x, 4.0), 0.0);
        att = att * window;
    }
    return att;
}

/// Document-space scaling for room GLB: `inv_doc_scale = 1/world_scale` when non-zero.
fn punctual_attenuation_with_inv_doc_scale(
    dist_world: f32,
    range_world: f32,
    inv_doc_scale: f32,
) -> f32 {
    let d = select(dist_world, dist_world * inv_doc_scale, inv_doc_scale > 1e-8);
    let r = select(range_world, range_world * inv_doc_scale, inv_doc_scale > 1e-8);
    return punctual_attenuation_khr(d, r);
}

/// Gameplay / synthetic point lights (quadratic window by radius).
fn scene_smooth_point_atten(dist: f32, radius: f32) -> f32 {
    let t = clamp(1.0 - dist / max(radius, 1.0), 0.0, 1.0);
    return t * t;
}

fn scene_safe_normalize(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let len2 = dot(v, v);
    return select(fallback, v * inverseSqrt(len2), len2 > 1e-8);
}

fn scene_luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// Receiver-side direct diffuse weight for punctual lights.
/// Candle softness belongs to source shape/material effects, not a separate
/// diffuse model.
fn scene_punctual_diffuse_weight(n_dot_l_raw: f32) -> f32 {
    return max(n_dot_l_raw, 0.0) * SCENE_INV_PI;
}

fn scene_fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    let ct = clamp(cos_theta, 0.0, 1.0);
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - ct, 5.0);
}

fn scene_distribution_ggx(ndh: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = ndh * ndh * (a2 - 1.0) + 1.0;
    return a2 / max(SCENE_PI * denom * denom, 1e-8);
}

fn scene_geometry_schlick_ggx(ndx: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    return ndx / max(ndx * (1.0 - k) + k, 1e-8);
}

fn scene_geometry_smith(ndv: f32, ndl: f32, roughness: f32) -> f32 {
    return scene_geometry_schlick_ggx(ndv, roughness) * scene_geometry_schlick_ggx(ndl, roughness);
}

/// Diffuse weight with a small floor so punctual-only scenes do not black out at grazing
/// view (Schlick `kD -> 0`) -- approximates Blender World + EEVEE ambient/indirect on dielectrics.
fn scene_dielectric_kd(ks: vec3<f32>, metallic: f32) -> vec3<f32> {
    let dielectric = 1.0 - metallic;
    let kd = (vec3<f32>(1.0) - ks) * dielectric;
    return max(kd, vec3<f32>(0.04 * dielectric));
}

struct ScenePbrDirectSurface {
    albedo: vec3<f32>,
    normal: vec3<f32>,
    view_dir: vec3<f32>,
    metallic: f32,
    roughness: f32,
}

struct ScenePbrDirectLight {
    direction: vec3<f32>,
    radiance: vec3<f32>,
    visibility: f32,
}

struct ScenePbrDirectContribution {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
    total: vec3<f32>,
    n_dot_l: f32,
}

struct ScenePbrPointLightSample {
    direction: vec3<f32>,
    radiance: vec3<f32>,
    distance: f32,
    attenuation: f32,
}

fn scene_pbr_direct_surface(
    albedo: vec3<f32>,
    normal: vec3<f32>,
    view_dir: vec3<f32>,
    metallic: f32,
    roughness: f32,
) -> ScenePbrDirectSurface {
    return ScenePbrDirectSurface(albedo, normal, view_dir, metallic, roughness);
}

fn scene_pbr_sample_point_light(
    surface_pos: vec3<f32>,
    light_pos: vec3<f32>,
    range_world: f32,
    color_intensity: vec4<f32>,
    attenuation_kind: f32,
    inv_doc_scale: f32,
) -> ScenePbrPointLightSample {
    let to_light = light_pos - surface_pos;
    let dist = length(to_light);
    if (dist <= 1e-4) {
        return ScenePbrPointLightSample(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), dist, 0.0);
    }
    let direction = to_light / dist;
    let atten = select(
        scene_smooth_point_atten(dist, range_world),
        punctual_attenuation_with_inv_doc_scale(dist, range_world, inv_doc_scale),
        attenuation_kind > 0.5,
    );
    let radiance = max(color_intensity.rgb, vec3<f32>(0.0)) * max(color_intensity.a, 0.0) * atten;
    return ScenePbrPointLightSample(direction, radiance, dist, atten);
}

fn scene_pbr_sample_spot_light(
    surface_pos: vec3<f32>,
    light_pos: vec3<f32>,
    range_world: f32,
    light_dir: vec3<f32>,
    cos_inner: f32,
    cos_outer: f32,
    color_intensity: vec4<f32>,
    attenuation_kind: f32,
    inv_doc_scale: f32,
) -> ScenePbrPointLightSample {
    let point = scene_pbr_sample_point_light(
        surface_pos,
        light_pos,
        range_world,
        color_intensity,
        attenuation_kind,
        inv_doc_scale,
    );
    if (point.distance <= 1e-4 || length(point.radiance) <= 0.0) {
        return point;
    }
    let spot_dir = scene_safe_normalize(light_dir, vec3<f32>(0.0, 0.0, -1.0));
    let angular = khr_spot_angle_attenuation_scene(dot(-point.direction, spot_dir), cos_inner, cos_outer);
    return ScenePbrPointLightSample(
        point.direction,
        point.radiance * angular,
        point.distance,
        point.attenuation * angular,
    );
}

fn scene_room_pbr_f0(albedo: vec3<f32>, metallic: f32) -> vec3<f32> {
    let albedo_lum = dot(albedo, vec3<f32>(0.299, 0.587, 0.114));
    let metal_f0_floor = vec3<f32>(0.52, 0.42, 0.24);
    let metal_ramp = smoothstep(0.45, 0.65, metallic);
    let dark_ramp = 1.0 - smoothstep(0.04, 0.16, albedo_lum);
    let f0_boost = metal_ramp * dark_ramp;
    let f0_base = mix(albedo, max(albedo, metal_f0_floor), f0_boost);
    return mix(vec3<f32>(0.04), f0_base, metallic);
}

fn scene_pbr_surface_brdf(surface: ScenePbrDirectSurface, light_dir: vec3<f32>) -> vec3<f32> {
    let n = scene_safe_normalize(surface.normal, vec3<f32>(0.0, 0.0, 1.0));
    let v = scene_safe_normalize(surface.view_dir, n);
    let l = scene_safe_normalize(light_dir, n);
    let n_dot_l = max(dot(n, l), 0.0);
    if (n_dot_l <= 0.0) {
        return vec3<f32>(0.0);
    }
    let n_dot_v = max(dot(n, v), 1e-4);
    let h = scene_safe_normalize(v + l, n);
    let n_dot_h = max(dot(n, h), 0.0);
    let v_dot_h = max(dot(v, h), 0.0);
    let roughness = clamp(surface.roughness, 0.04, 1.0);
    let metallic = clamp(surface.metallic, 0.0, 1.0);
    let f0 = scene_room_pbr_f0(max(surface.albedo, vec3<f32>(0.0)), metallic);
    let f = scene_fresnel_schlick(v_dot_h, f0);
    let kd = scene_dielectric_kd(f, metallic);
    let diffuse = kd * max(surface.albedo, vec3<f32>(0.0)) * SCENE_INV_PI;
    let d = scene_distribution_ggx(n_dot_h, roughness);
    let g = scene_geometry_smith(n_dot_v, n_dot_l, roughness);
    let specular = d * g * f / max(4.0 * n_dot_v * n_dot_l, 1e-6);
    return diffuse + specular;
}

fn scene_pbr_direct_punctual_radiance(
    surface: ScenePbrDirectSurface,
    light: ScenePbrDirectLight,
) -> ScenePbrDirectContribution {
    let n = scene_safe_normalize(surface.normal, vec3<f32>(0.0, 0.0, 1.0));
    let l = scene_safe_normalize(light.direction, n);
    let n_dot_l_raw = dot(n, l);
    let n_dot_l = max(n_dot_l_raw, 0.0);
    let visibility = clamp(light.visibility, 0.0, 1.0);
    let radiance = max(light.radiance, vec3<f32>(0.0)) * visibility;
    if (n_dot_l <= 0.0 || length(radiance) <= 0.0) {
        return ScenePbrDirectContribution(vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), n_dot_l);
    }

    let v = scene_safe_normalize(surface.view_dir, n);
    let n_dot_v = max(dot(n, v), 1e-4);
    let h = scene_safe_normalize(v + l, n);
    let n_dot_h = max(dot(n, h), 0.0);
    let v_dot_h = max(dot(v, h), 0.0);
    let roughness = clamp(surface.roughness, 0.04, 1.0);
    let metallic = clamp(surface.metallic, 0.0, 1.0);
    let albedo = max(surface.albedo, vec3<f32>(0.0));
    let f0 = scene_room_pbr_f0(albedo, metallic);
    let f = scene_fresnel_schlick(v_dot_h, f0);
    let kd = scene_dielectric_kd(f, metallic);
    let diffuse = kd * albedo * radiance * scene_punctual_diffuse_weight(n_dot_l_raw);
    let d = scene_distribution_ggx(n_dot_h, roughness);
    let g = scene_geometry_smith(n_dot_v, n_dot_l, roughness);
    let spec_brdf = d * g * f / max(4.0 * n_dot_v * n_dot_l, 1e-6);
    let specular = spec_brdf * radiance * n_dot_l;
    return ScenePbrDirectContribution(diffuse, specular, diffuse + specular, n_dot_l);
}

fn scene_pbr_direct_sampled_light(
    surface: ScenePbrDirectSurface,
    sample: ScenePbrPointLightSample,
    visibility: f32,
) -> ScenePbrDirectContribution {
    return scene_pbr_direct_punctual_radiance(
        surface,
        ScenePbrDirectLight(sample.direction, sample.radiance, visibility),
    );
}

fn scene_environment_radiance(
    dir: vec3<f32>,
    roughness: f32,
    ambient_scale: f32,
    exposure: f32,
) -> vec3<f32> {
    let scale = max(ambient_scale, 0.0);
    if (scale <= 0.0) {
        return vec3<f32>(0.0);
    }
    let d = scene_safe_normalize(dir, vec3<f32>(0.0, 0.0, 1.0));
    let up = clamp(d.z * 0.5 + 0.5, 0.0, 1.0);
    let horizon = vec3<f32>(0.43, 0.37, 0.30);
    let sky = vec3<f32>(0.62, 0.58, 0.51);
    let ground = vec3<f32>(0.22, 0.18, 0.14);
    let hemi = mix(ground, sky, smoothstep(0.0, 1.0, up));
    let horizon_fill = horizon * (1.0 - abs(up * 2.0 - 1.0));
    let blur = clamp(roughness * 0.35, 0.0, 0.35);
    let blurred = mix(hemi + horizon_fill * 0.25, vec3<f32>(scene_luminance(hemi)), blur);
    return scale * blurred / max(exposure, 1e-5);
}

fn scene_world_hemisphere_lighting(
    n: vec3<f32>,
    albedo: vec3<f32>,
    metallic: f32,
    ambient_scale: f32,
    exposure: f32,
) -> vec3<f32> {
    let env = scene_environment_radiance(n, 1.0, ambient_scale, exposure);
    let dielectric = (1.0 - metallic) * albedo * env * 0.09;
    let albedo_lum = dot(albedo, vec3<f32>(0.299, 0.587, 0.114));
    let metal_tint = mix(
        vec3<f32>(0.58, 0.50, 0.40),
        albedo,
        clamp(albedo_lum * 22.0, 0.0, 1.0),
    ) * metallic * 0.14;
    return dielectric + metal_tint * env;
}

// Jorge Jimenez's interleaved gradient noise: stable screen-space jitter for
// punctual occlusion rays and anisotropic material variation.
fn scene_ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(0.06711056 * p.x + 0.00583715 * p.y));
}

// Slab test: segment from `light_pos` to `frag_pos` pierces the AABB between t ~= 0 and t ~= 1.
fn scene_segment_hits_aabb(
    light_pos: vec3<f32>,
    inv_dir: vec3<f32>,
    c: vec3<f32>,
    h: vec3<f32>,
    near_bias: f32,
    far_bias: f32,
) -> bool {
    let t1 = (c - h - light_pos) * inv_dir;
    let t2 = (c + h - light_pos) * inv_dir;
    let tmin = min(t1, t2);
    let tmax = max(t1, t2);
    let near_t = max(max(tmin.x, tmin.y), tmin.z);
    let far_t = min(min(tmax.x, tmax.y), tmax.z);
    return far_t > near_t && near_t > near_bias && near_t < far_bias;
}
