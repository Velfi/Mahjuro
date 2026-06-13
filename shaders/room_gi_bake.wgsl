const PI: f32 = 3.141592653589793;
const GOLDEN: f32 = 2.399963229728653;
const BVH_STACK_MAX: u32 = 96u;

struct BakeParams {
    counts: vec4<u32>,
    grid: vec4<u32>,
    world_min: vec4<f32>,
    world_extent: vec4<f32>,
    trace_params: vec4<f32>,
    lighting_params: vec4<f32>,
}

struct BakeTriangle {
    p0: vec4<f32>,
    p1: vec4<f32>,
    p2: vec4<f32>,
    n0: vec4<f32>,
    n1: vec4<f32>,
    n2: vec4<f32>,
    uv0_uv1: vec4<f32>,
    uv2_uvemr0: vec4<f32>,
    uvemr1_uvemr2: vec4<f32>,
    color0: vec4<f32>,
    color1: vec4<f32>,
    color2: vec4<f32>,
    tags: vec4<u32>,
}

struct BakeMaterial {
    albedo_rect: vec4<u32>,
    mr_rect: vec4<u32>,
    emissive_rect: vec4<u32>,
    factors: vec4<f32>,
    emissive_factor: vec4<f32>,
    flags: vec4<u32>,
}

struct BakeLight {
    pos_range: vec4<f32>,
    color_intensity: vec4<f32>,
    params: vec4<f32>,
}

struct BakeBvhNode {
    bounds_min: vec4<f32>,
    bounds_max: vec4<f32>,
    info: vec4<u32>,
}

struct LightmapTexel {
    tri: vec4<u32>,
    bary: vec4<f32>,
}

struct Hit {
    tri: u32,
    t: f32,
    bary: vec3<f32>,
    point: vec3<f32>,
    normal: vec3<f32>,
}

struct SurfaceSample {
    albedo: vec3<f32>,
    alpha: f32,
    metallic: f32,
    roughness: f32,
    emissive: vec3<f32>,
}

struct AtlasSampleCoords {
    p00: vec2<i32>,
    p10: vec2<i32>,
    p01: vec2<i32>,
    p11: vec2<i32>,
    w: vec2<f32>,
}

@group(0) @binding(0) var<uniform> params: BakeParams;
@group(0) @binding(1) var<storage, read> tris: array<BakeTriangle>;
@group(0) @binding(2) var<storage, read> materials: array<BakeMaterial>;
@group(0) @binding(3) var<storage, read> lights: array<BakeLight>;
@group(0) @binding(4) var albedo_atlas: texture_2d<f32>;
@group(0) @binding(5) var mr_atlas: texture_2d<f32>;
@group(0) @binding(6) var emissive_atlas: texture_2d<f32>;
@group(0) @binding(8) var<storage, read> bvh_nodes: array<BakeBvhNode>;
@group(0) @binding(9) var<storage, read> bvh_indices: array<u32>;
@group(0) @binding(10) var<storage, read> lightmap_texels: array<LightmapTexel>;
@group(0) @binding(11) var<storage, read_write> out_lightmap: array<vec4<f32>>;
@group(0) @binding(12) var<storage, read_write> tmp_lightmap: array<vec4<f32>>;

fn srgb_to_linear(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn srgb3_to_linear(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        srgb_to_linear(v.r),
        srgb_to_linear(v.g),
        srgb_to_linear(v.b),
    );
}

fn wrap_coord(v: f32, mode: u32) -> f32 {
    if (mode == 0u) {
        return clamp(v, 0.0, 1.0);
    }
    if (mode == 2u) {
        let t = v - floor(v / 2.0) * 2.0;
        return select(t, 2.0 - t, t > 1.0);
    }
    return fract(v);
}

fn wrap_texel_index(i: i32, size_u: u32, mode: u32) -> i32 {
    let size = max(i32(size_u), 1);
    if (mode == 0u) {
        return clamp(i, 0, size - 1);
    }
    if (mode == 2u) {
        let period = max(size * 2, 1);
        var m = i % period;
        if (m < 0) {
            m = m + period;
        }
        return select(m, period - 1 - m, m >= size);
    }
    var r = i % size;
    if (r < 0) {
        r = r + size;
    }
    return r;
}

fn atlas_sample_coords(rect: vec4<u32>, uv: vec2<f32>, wrap_s: u32, wrap_t: u32) -> AtlasSampleCoords {
    let rw = max(rect.z, 1u);
    let rh = max(rect.w, 1u);
    let u = wrap_coord(uv.x, wrap_s);
    let v = wrap_coord(uv.y, wrap_t);
    let sx = u * f32(rw) - 0.5;
    let sy = v * f32(rh) - 0.5;
    let fx = floor(sx);
    let fy = floor(sy);
    let ix0 = i32(fx);
    let iy0 = i32(fy);
    let ix1 = ix0 + 1;
    let iy1 = iy0 + 1;
    let x0 = i32(rect.x) + wrap_texel_index(ix0, rw, wrap_s);
    let x1 = i32(rect.x) + wrap_texel_index(ix1, rw, wrap_s);
    let y0 = i32(rect.y) + wrap_texel_index(iy0, rh, wrap_t);
    let y1 = i32(rect.y) + wrap_texel_index(iy1, rh, wrap_t);
    return AtlasSampleCoords(
        vec2<i32>(x0, y0),
        vec2<i32>(x1, y0),
        vec2<i32>(x0, y1),
        vec2<i32>(x1, y1),
        vec2<f32>(fract(sx), fract(sy)),
    );
}

fn bilerp_rgba(c00: vec4<f32>, c10: vec4<f32>, c01: vec4<f32>, c11: vec4<f32>, w: vec2<f32>) -> vec4<f32> {
    return mix(mix(c00, c10, w.x), mix(c01, c11, w.x), w.y);
}

fn sample_albedo(m: BakeMaterial, uv: vec2<f32>) -> vec4<f32> {
    let s = atlas_sample_coords(m.albedo_rect, uv, m.flags.y, m.flags.z);
    let c = bilerp_rgba(
        textureLoad(albedo_atlas, s.p00, 0),
        textureLoad(albedo_atlas, s.p10, 0),
        textureLoad(albedo_atlas, s.p01, 0),
        textureLoad(albedo_atlas, s.p11, 0),
        s.w,
    );
    return vec4<f32>(srgb3_to_linear(c.rgb), c.a);
}

fn sample_mr(m: BakeMaterial, uv: vec2<f32>) -> vec4<f32> {
    let s = atlas_sample_coords(m.mr_rect, uv, m.flags.y, m.flags.z);
    return bilerp_rgba(
        textureLoad(mr_atlas, s.p00, 0),
        textureLoad(mr_atlas, s.p10, 0),
        textureLoad(mr_atlas, s.p01, 0),
        textureLoad(mr_atlas, s.p11, 0),
        s.w,
    );
}

fn sample_emissive(m: BakeMaterial, uv: vec2<f32>) -> vec3<f32> {
    let s = atlas_sample_coords(m.emissive_rect, uv, m.flags.y, m.flags.z);
    let c = bilerp_rgba(
        textureLoad(emissive_atlas, s.p00, 0),
        textureLoad(emissive_atlas, s.p10, 0),
        textureLoad(emissive_atlas, s.p01, 0),
        textureLoad(emissive_atlas, s.p11, 0),
        s.w,
    );
    return srgb3_to_linear(c.rgb);
}

fn tri_uv0(t: BakeTriangle) -> vec2<f32> {
    return t.uv0_uv1.xy;
}

fn tri_uv1(t: BakeTriangle) -> vec2<f32> {
    return t.uv0_uv1.zw;
}

fn tri_uv2(t: BakeTriangle) -> vec2<f32> {
    return t.uv2_uvemr0.xy;
}

fn tri_uvemr0(t: BakeTriangle) -> vec2<f32> {
    return t.uv2_uvemr0.zw;
}

fn tri_uvemr1(t: BakeTriangle) -> vec2<f32> {
    return t.uvemr1_uvemr2.xy;
}

fn tri_uvemr2(t: BakeTriangle) -> vec2<f32> {
    return t.uvemr1_uvemr2.zw;
}

fn interp2(a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, bary: vec3<f32>) -> vec2<f32> {
    return a * bary.x + b * bary.y + c * bary.z;
}

fn interp4(a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, bary: vec3<f32>) -> vec4<f32> {
    return a * bary.x + b * bary.y + c * bary.z;
}

fn bake_finite3(v: vec3<f32>) -> bool {
    return v.x == v.x
        && v.y == v.y
        && v.z == v.z
        && abs(v.x) < 1e20
        && abs(v.y) < 1e20
        && abs(v.z) < 1e20;
}

fn bake_sanitize_radiance(v: vec3<f32>) -> vec3<f32> {
    if (!bake_finite3(v)) {
        return vec3<f32>(0.0);
    }
    return clamp(v, vec3<f32>(0.0), vec3<f32>(1.0e5));
}

fn bake_sanitize_lightmap_pixel(px: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(bake_sanitize_radiance(px.rgb), select(0.0, 1.0, px.a > 0.5));
}

fn bake_triangle_geom_normal(t: BakeTriangle) -> vec3<f32> {
    return scene_safe_normalize(cross(t.p1.xyz - t.p0.xyz, t.p2.xyz - t.p0.xyz), vec3<f32>(0.0, 0.0, 1.0));
}

fn bake_triangle_normal(t: BakeTriangle, bary: vec3<f32>) -> vec3<f32> {
    let face_n = bake_triangle_geom_normal(t);
    return scene_safe_normalize(t.n0.xyz * bary.x + t.n1.xyz * bary.y + t.n2.xyz * bary.z, face_n);
}

fn surface_sample(hit: Hit) -> SurfaceSample {
    let t = tris[hit.tri];
    let m = materials[t.tags.x];
    let uv = interp2(tri_uv0(t), tri_uv1(t), tri_uv2(t), hit.bary);
    let uv_emr = interp2(tri_uvemr0(t), tri_uvemr1(t), tri_uvemr2(t), hit.bary);
    let vertex_color = max(interp4(t.color0, t.color1, t.color2, hit.bary), vec4<f32>(0.0));
    let tex = sample_albedo(m, uv);
    var tex_rgb = tex.rgb;
    let tex_lum = dot(tex_rgb, vec3<f32>(0.299, 0.587, 0.114));
    if ((m.flags.x == 1u || m.flags.x == 2u) && tex_lum < 1e-4 && tex.a > 1e-4) {
        tex_rgb = vec3<f32>(tex.a);
    }
    let mr = sample_mr(m, uv_emr);
    let em = sample_emissive(m, uv_emr);
    return SurfaceSample(
        clamp(tex_rgb * vertex_color.rgb, vec3<f32>(0.0), vec3<f32>(32.0)),
        clamp(tex.a * vertex_color.a, 0.0, 1.0),
        clamp(mr.b * m.factors.x, 0.0, 1.0),
        clamp(mr.g * m.factors.y, 0.02, 1.0),
        em * m.emissive_factor.rgb * m.factors.w,
    );
}

fn material_accepts_hit(hit: Hit) -> bool {
    let t = tris[hit.tri];
    let m = materials[t.tags.x];
    if (m.flags.x == 0u) {
        return true;
    }
    let sample = surface_sample(hit);
    if (m.flags.x == 1u) {
        return sample.alpha >= m.factors.z;
    }
    return sample.alpha >= 0.5;
}

fn intersect_triangle(i: u32, origin: vec3<f32>, dir: vec3<f32>, t_min: f32, t_max: f32) -> Hit {
    let t = tris[i];
    let p0 = t.p0.xyz;
    let p1 = t.p1.xyz;
    let p2 = t.p2.xyz;
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let pvec = cross(dir, e2);
    let det = dot(e1, pvec);
    if (abs(det) < 1e-8) {
        return Hit(0u, -1.0, vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0));
    }
    let inv_det = 1.0 / det;
    let tvec = origin - p0;
    let u = dot(tvec, pvec) * inv_det;
    if (u < 0.0 || u > 1.0) {
        return Hit(0u, -1.0, vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0));
    }
    let qvec = cross(tvec, e1);
    let v = dot(dir, qvec) * inv_det;
    if (v < 0.0 || u + v > 1.0) {
        return Hit(0u, -1.0, vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0));
    }
    let d = dot(e2, qvec) * inv_det;
    if (d < t_min || d > t_max) {
        return Hit(0u, -1.0, vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0));
    }
    let bary = vec3<f32>(1.0 - u - v, u, v);
    var n = bake_triangle_normal(t, bary);
    if (dot(n, dir) > 0.0) {
        n = -n;
    }
    return Hit(i, d, bary, origin + dir * d, n);
}

fn hit_from_triangle_bary(i: u32, bary: vec3<f32>) -> Hit {
    let t = tris[i];
    let p0 = t.p0.xyz;
    let p1 = t.p1.xyz;
    let p2 = t.p2.xyz;
    let n = bake_triangle_normal(t, bary);
    return Hit(i, 0.0, bary, p0 * bary.x + p1 * bary.y + p2 * bary.z, n);
}

fn inv_ray_component(v: f32) -> f32 {
    if (abs(v) > 1e-8) {
        return 1.0 / v;
    }
    return select(-1e30, 1e30, v >= 0.0);
}

fn inv_ray_dir(dir: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        inv_ray_component(dir.x),
        inv_ray_component(dir.y),
        inv_ray_component(dir.z),
    );
}

fn ray_hits_aabb(
    origin: vec3<f32>,
    inv_dir: vec3<f32>,
    bounds_min: vec3<f32>,
    bounds_max: vec3<f32>,
    t_min: f32,
    t_max: f32,
) -> bool {
    let t1 = (bounds_min - origin) * inv_dir;
    let t2 = (bounds_max - origin) * inv_dir;
    let lo = min(t1, t2);
    let hi = max(t1, t2);
    let near_t = max(max(lo.x, lo.y), lo.z);
    let far_t = min(min(hi.x, hi.y), hi.z);
    return far_t >= max(near_t, t_min) && near_t <= t_max;
}

fn trace_scene(origin: vec3<f32>, dir: vec3<f32>, t_min: f32, t_max: f32) -> Hit {
    var best = Hit(0u, -1.0, vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0));
    var best_t = t_max;
    if (params.counts.x == 0u) {
        return best;
    }

    let inv_dir = inv_ray_dir(dir);
    var stack: array<u32, 96>;
    var stack_len = 1u;
    stack[0] = 0u;
    loop {
        if (stack_len == 0u) {
            break;
        }
        stack_len = stack_len - 1u;
        let node = bvh_nodes[stack[stack_len]];
        if (!ray_hits_aabb(origin, inv_dir, node.bounds_min.xyz, node.bounds_max.xyz, t_min, best_t)) {
            continue;
        }

        if (node.info.z != 0u) {
            let end = node.info.x + node.info.y;
            for (var k = node.info.x; k < end; k = k + 1u) {
                let tri_idx = bvh_indices[k];
                let h = intersect_triangle(tri_idx, origin, dir, t_min, best_t);
                if (h.t < 0.0) {
                    continue;
                }
                if (!material_accepts_hit(h)) {
                    continue;
                }
                best = h;
                best_t = h.t;
            }
            continue;
        }

        if (stack_len + 2u <= BVH_STACK_MAX) {
            stack[stack_len] = node.info.x;
            stack_len = stack_len + 1u;
            stack[stack_len] = node.info.y;
            stack_len = stack_len + 1u;
        }
    }
    return best;
}

fn world_environment_radiance(dir: vec3<f32>, roughness: f32) -> vec3<f32> {
    return scene_environment_radiance(
        dir,
        roughness,
        params.lighting_params.x,
        params.lighting_params.y,
    );
}

fn world_hemisphere_radiance(n: vec3<f32>, albedo: vec3<f32>, metallic: f32) -> vec3<f32> {
    return scene_world_hemisphere_lighting(
        n,
        albedo,
        metallic,
        params.lighting_params.x,
        params.lighting_params.y,
    );
}

fn tangent_basis(n: vec3<f32>) -> mat3x3<f32> {
    let up = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.z) > 0.92);
    let t = scene_safe_normalize(cross(up, n), vec3<f32>(1.0, 0.0, 0.0));
    let b = cross(n, t);
    return mat3x3<f32>(t, b, n);
}

fn cosine_hemi_dir(n: vec3<f32>, i: u32, count: u32, seed: u32) -> vec3<f32> {
    let c = f32(max(count, 1u));
    let m = f32(i) + 0.5;
    let r = sqrt(m / c);
    let phi = GOLDEN * (m + f32(seed & 1023u) * 0.61803398875);
    let local = vec3<f32>(cos(phi) * r, sin(phi) * r, sqrt(max(1.0 - r * r, 0.0)));
    return scene_safe_normalize(tangent_basis(n) * local, n);
}

fn low_discrepancy_2d(i: u32, seed: u32) -> vec2<f32> {
    let s = f32(seed & 4095u);
    let a = fract((f32(i) + 0.5 + s * 0.61803398875) * 0.754877666);
    let b = fract((f32(i) + 0.5 + s * 0.41421356237) * 0.569840296);
    return vec2<f32>(a, max(b, 1e-5));
}

fn sample_ggx_reflection_dir(n: vec3<f32>, v: vec3<f32>, i: u32, count: u32, seed: u32, roughness: f32) -> vec3<f32> {
    let xi = low_discrepancy_2d(i, seed + count * 13u);
    let a = roughness * roughness;
    let a2 = max(a * a, 1e-5);
    let phi = 2.0 * PI * xi.x;
    let cos_theta = sqrt((1.0 - xi.y) / max(1.0 + (a2 - 1.0) * xi.y, 1e-5));
    let sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 0.0));
    let h = scene_safe_normalize(
        tangent_basis(n) * vec3<f32>(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta),
        n,
    );
    let wi = reflect(-v, h);
    return scene_safe_normalize(wi, n);
}

fn diffuse_pdf(n: vec3<f32>, wi: vec3<f32>) -> f32 {
    return scene_punctual_diffuse_weight(dot(n, wi));
}

fn ggx_reflection_pdf(n: vec3<f32>, v: vec3<f32>, wi: vec3<f32>, roughness: f32) -> f32 {
    if (dot(n, wi) <= 0.0) {
        return 0.0;
    }
    let h = scene_safe_normalize(v + wi, n);
    let ndh = max(dot(n, h), 0.0);
    let vdh = max(dot(v, h), 1e-5);
    return scene_distribution_ggx(ndh, roughness) * ndh / max(4.0 * vdh, 1e-5);
}

fn surface_bsdf(sample: SurfaceSample, n: vec3<f32>, v: vec3<f32>, wi: vec3<f32>) -> vec3<f32> {
    return scene_pbr_surface_brdf(
        scene_pbr_direct_surface(sample.albedo, n, v, sample.metallic, sample.roughness),
        wi,
    );
}

fn mis_mixture_pdf(n: vec3<f32>, v: vec3<f32>, wi: vec3<f32>, roughness: f32) -> f32 {
    return 0.5 * diffuse_pdf(n, wi) + 0.5 * ggx_reflection_pdf(n, v, wi, roughness);
}

fn surface_receiver_indirect_base(hit: Hit, sample: SurfaceSample) -> vec3<f32> {
    return world_hemisphere_radiance(hit.normal, sample.albedo, sample.metallic);
}

fn surface_bounce_source_radiance_sampled(hit: Hit, sample: SurfaceSample, view_dir: vec3<f32>) -> vec3<f32> {
    let V = scene_safe_normalize(view_dir, hit.normal);
    let pbr_surface = scene_pbr_direct_surface(
        sample.albedo,
        hit.normal,
        V,
        sample.metallic,
        sample.roughness,
    );
    var out = sample.emissive + world_hemisphere_radiance(hit.normal, sample.albedo, sample.metallic);
    for (var i: u32 = 0u; i < params.counts.z; i = i + 1u) {
        let lgt = lights[i];
        let point_sample = scene_pbr_sample_point_light(
            hit.point,
            lgt.pos_range.xyz,
            lgt.pos_range.w,
            lgt.color_intensity,
            lgt.params.x,
            params.trace_params.x,
        );
        if (point_sample.distance <= params.trace_params.y) {
            continue;
        }
        if (length(point_sample.radiance) <= 0.0) {
            continue;
        }
        // Runtime room lighting does not trace imported punctual lights through
        // the full static GLB. Keep the bounce source on the same contract so
        // enclosed authoring lights, like stair lanterns, can still contribute
        // indirect energy to the lightmap.
        let direct = scene_pbr_direct_sampled_light(pbr_surface, point_sample, 1.0);
        out = out + direct.total;
    }
    return bake_sanitize_radiance(out);
}

fn surface_bounce_source_radiance(hit: Hit, view_dir: vec3<f32>) -> vec3<f32> {
    return surface_bounce_source_radiance_sampled(hit, surface_sample(hit), view_dir);
}

fn surface_path_radiance_sample(hit: Hit, view_dir: vec3<f32>, seed: u32, sample_idx: u32) -> vec3<f32> {
    let sample = surface_sample(hit);
    let base = surface_receiver_indirect_base(hit, sample);
    let secondary_count = u32(max(params.lighting_params.z, 0.0));
    if (secondary_count == 0u) {
        return bake_sanitize_radiance(base);
    }
    let v = scene_safe_normalize(view_dir, hit.normal);
    let bias = params.trace_params.y;
    var bounce_sum = vec3<f32>(0.0);
    for (var bi: u32 = 0u; bi < secondary_count; bi = bi + 1u) {
        let bounce_idx = sample_idx * secondary_count + bi;
        let diffuse_sample = (bounce_idx & 1u) == 0u;
        let bounce_dir = select(
            sample_ggx_reflection_dir(hit.normal, v, bounce_idx, secondary_count, seed + bi * 1597334677u, sample.roughness),
            cosine_hemi_dir(hit.normal, bounce_idx, secondary_count, seed + bi * 3812015801u),
            diffuse_sample,
        );
        let ndl = max(dot(hit.normal, bounce_dir), 0.0);
        if (ndl <= 0.0) {
            continue;
        }
        let pdf = mis_mixture_pdf(hit.normal, v, bounce_dir, sample.roughness);
        if (pdf <= 1e-8) {
            continue;
        }
        let h2 = trace_scene(hit.point + hit.normal * bias, bounce_dir, bias, params.trace_params.z);
        let bsdf = surface_bsdf(sample, hit.normal, v, bounce_dir);
        if (h2.t < 0.0) {
            let env = world_environment_radiance(bounce_dir, sample.roughness);
            bounce_sum = bounce_sum + env * bsdf * (ndl / pdf);
        } else {
            bounce_sum = bounce_sum + surface_bounce_source_radiance(h2, -bounce_dir) * bsdf * (ndl / pdf);
        }
    }
    let bounce = bounce_sum / f32(max(secondary_count, 1u));
    return bake_sanitize_radiance(base + bounce);
}

fn adaptive_surface_radiance(hit: Hit, view_dir: vec3<f32>, texel_seed: u32) -> vec3<f32> {
    let max_samples = max(params.counts.w, 1u);
    let min_samples = min(max(u32(params.trace_params.w), 1u), max_samples);
    let rel_stderr_target = max(params.lighting_params.w, 0.0);
    var mean = vec3<f32>(0.0);
    var mean_luma = 0.0;
    var m2_luma = 0.0;
    var sample_count = 0u;
    for (var si: u32 = 0u; si < max_samples; si = si + 1u) {
        let seed = texel_seed + si * 747796405u + 2891336453u;
        let sample = surface_path_radiance_sample(hit, view_dir, seed, si);
        sample_count = sample_count + 1u;
        let count_f = f32(sample_count);
        let delta = sample - mean;
        mean = mean + delta / count_f;

        let luma = scene_luminance(sample);
        let dl = luma - mean_luma;
        mean_luma = mean_luma + dl / count_f;
        m2_luma = m2_luma + dl * (luma - mean_luma);

        if (sample_count >= min_samples && sample_count > 1u && rel_stderr_target > 0.0) {
            let variance = max(m2_luma / f32(sample_count - 1u), 0.0);
            let stderr = sqrt(variance / count_f);
            let rel_stderr = stderr / max(abs(mean_luma), 1e-4);
            if (rel_stderr <= rel_stderr_target) {
                break;
            }
        }
    }
    return bake_sanitize_radiance(mean);
}

@compute @workgroup_size(8, 8, 1)
fn lightmap_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.grid.x || gid.y >= params.grid.y) {
        return;
    }
    let idx = gid.x + gid.y * params.grid.x;
    let lm = lightmap_texels[idx];
    if (lm.tri.x == 0xffffffffu) {
        out_lightmap[idx] = vec4<f32>(0.0);
        return;
    }
    let hit = hit_from_triangle_bary(lm.tri.x, lm.bary.xyz);
    if (!material_accepts_hit(hit)) {
        out_lightmap[idx] = vec4<f32>(0.0);
        return;
    }
    let radiance = adaptive_surface_radiance(hit, hit.normal, idx);
    out_lightmap[idx] = vec4<f32>(radiance, 1.0);
}

fn lightmap_texel_visible_idx(idx: u32) -> bool {
    let lm = lightmap_texels[idx];
    if (lm.tri.x == 0xffffffffu) {
        return false;
    }
    let hit = hit_from_triangle_bary(lm.tri.x, lm.bary.xyz);
    return material_accepts_hit(hit);
}

@compute @workgroup_size(8, 8, 1)
fn lightmap_alpha_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.grid.x || gid.y >= params.grid.y) {
        return;
    }
    let idx = gid.x + gid.y * params.grid.x;
    let visible = lightmap_texel_visible_idx(idx) && out_lightmap[idx].a > 0.5;
    let rgb = bake_sanitize_radiance(out_lightmap[idx].rgb);
    out_lightmap[idx] = vec4<f32>(rgb, select(0.0, 1.0, visible));
}

fn lightmap_pixel_idx(x: i32, y: i32) -> u32 {
    return u32(x) + u32(y) * params.grid.x;
}

fn lightmap_pixel_visible(px: vec4<f32>) -> bool {
    return px.a > 0.5 && bake_finite3(px.rgb);
}

@compute @workgroup_size(8, 8, 1)
fn lightmap_denoise_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.grid.x || gid.y >= params.grid.y) {
        return;
    }
    let idx = gid.x + gid.y * params.grid.x;
    let center = out_lightmap[idx];
    if (!lightmap_pixel_visible(center)) {
        tmp_lightmap[idx] = bake_sanitize_lightmap_pixel(center);
        return;
    }
    let center_luma = scene_luminance(center.rgb);
    var sum = center.rgb * 4.0;
    var weight = 4.0;
    for (var oy: i32 = -1; oy <= 1; oy = oy + 1) {
        for (var ox: i32 = -1; ox <= 1; ox = ox + 1) {
            if (ox == 0 && oy == 0) {
                continue;
            }
            let nx = i32(gid.x) + ox;
            let ny = i32(gid.y) + oy;
            if (nx < 0 || ny < 0 || nx >= i32(params.grid.x) || ny >= i32(params.grid.y)) {
                continue;
            }
            let neighbor = out_lightmap[lightmap_pixel_idx(nx, ny)];
            if (!lightmap_pixel_visible(neighbor)) {
                continue;
            }
            let luma = scene_luminance(neighbor.rgb);
            let rel = abs(luma - center_luma) / max(max(center_luma, luma), 1e-4);
            let w = clamp(1.0 - rel, 0.0, 1.0);
            sum = sum + neighbor.rgb * w;
            weight = weight + w;
        }
    }
    tmp_lightmap[idx] = vec4<f32>(bake_sanitize_radiance(sum / max(weight, 1e-5)), 1.0);
}

@compute @workgroup_size(8, 8, 1)
fn lightmap_dilate_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.grid.x || gid.y >= params.grid.y) {
        return;
    }
    let idx = gid.x + gid.y * params.grid.x;
    let center = out_lightmap[idx];
    if (lightmap_pixel_visible(center)) {
        tmp_lightmap[idx] = bake_sanitize_lightmap_pixel(center);
        return;
    }
    var sum = vec3<f32>(0.0);
    var count = 0.0;
    for (var oy: i32 = -1; oy <= 1; oy = oy + 1) {
        for (var ox: i32 = -1; ox <= 1; ox = ox + 1) {
            if (ox == 0 && oy == 0) {
                continue;
            }
            let nx = i32(gid.x) + ox;
            let ny = i32(gid.y) + oy;
            if (nx < 0 || ny < 0 || nx >= i32(params.grid.x) || ny >= i32(params.grid.y)) {
                continue;
            }
            let neighbor = out_lightmap[lightmap_pixel_idx(nx, ny)];
            if (!lightmap_pixel_visible(neighbor)) {
                continue;
            }
            sum = sum + neighbor.rgb;
            count = count + 1.0;
        }
    }
    if (count > 0.0) {
        tmp_lightmap[idx] = vec4<f32>(bake_sanitize_radiance(sum / count), 1.0);
    } else {
        tmp_lightmap[idx] = vec4<f32>(bake_sanitize_radiance(center.rgb), 0.0);
    }
}

@compute @workgroup_size(8, 8, 1)
fn lightmap_copy_tmp_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.grid.x || gid.y >= params.grid.y) {
        return;
    }
    let idx = gid.x + gid.y * params.grid.x;
    out_lightmap[idx] = tmp_lightmap[idx];
}

@compute @workgroup_size(8, 8, 1)
fn lightmap_finalize_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.grid.x || gid.y >= params.grid.y) {
        return;
    }
    let idx = gid.x + gid.y * params.grid.x;
    let visible = lightmap_texel_visible_idx(idx);
    let rgb = bake_sanitize_radiance(out_lightmap[idx].rgb);
    out_lightmap[idx] = vec4<f32>(rgb, select(0.0, 1.0, visible));
}
