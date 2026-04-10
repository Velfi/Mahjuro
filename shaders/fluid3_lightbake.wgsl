// Smoke pre-lighting bake.
//
// One compute thread per voxel. After the projection step has finalised
// `vd[0]` for the frame, this pass walks every cell in the grid, samples
// all candle point lights at the cell's world-space center using the
// SAME falloff/clamp/Reinhard model the original raymarch fragment shader
// used per step, and writes the resulting smoke colour into a sibling
// `lit_density` 3D texture: `(rgb = pre-lit colour, a = density)`.
//
// The volumetric raymarch then samples this texture instead of the raw
// `vd[0]`, dropping the per-step lighting loop entirely. The math:
//
//   Old per fragment: 28 steps × 8 lights = 224 light-evals × 2M pixels
//                   = ~450M evals per frame
//   New per voxel:    1 eval × 8 lights × 64·32·64 cells
//                   = ~1.0M evals per frame (~450× fewer)
//
// Voxel-rate lighting is correct here because:
//   1. The per-voxel size (~2 world units) is much smaller than the
//      candle radii (~30-60 units), so the lighting field is smooth at
//      the grid scale and doesn't need to be re-evaluated per ray sample.
//   2. The bilinear sampler used by the raymarch already interpolates
//      between neighbouring voxels, so the lighting transitions smoothly
//      along the ray.
//   3. The lit colour is written for every voxel regardless of density,
//      so bilinear interpolation at plume edges blends correctly between
//      "lit but empty" and "lit and dense" cells.

const MAX_LIGHTS: u32 = 16u;

struct FluidUniforms {
    grid_size:    vec4<f32>,
    grid_min:     vec4<f32>,
    grid_max:     vec4<f32>,
    inv_extent:   vec4<f32>,
    params:       vec4<f32>,
};

struct VolumeCamera {
    inv_view_proj: mat4x4<f32>,
    view_proj:     mat4x4<f32>,
    cam_pos:       vec4<f32>,
    grid_min:      vec4<f32>,
    grid_max:      vec4<f32>,
    // x=max_alpha, y=step_count, z=light_strength, w=ambient.
    // We only consume z and w here.
    params:        vec4<f32>,
    mode:          vec4<f32>,
};

struct PointLight {
    pos:   vec4<f32>,   // xyz position, w radius
    color: vec4<f32>,   // rgb color, a intensity
};
struct PointLights {
    count:  vec4<u32>,
    extras: vec4<f32>,
    lights: array<PointLight, 16>,
};

@group(0) @binding(0) var<uniform> u: FluidUniforms;
@group(0) @binding(1) var src_vd: texture_3d<f32>;
@group(0) @binding(2) var dst_lit: texture_storage_3d<rgba16float, write>;
@group(0) @binding(3) var<uniform> cam: VolumeCamera;
@group(0) @binding(4) var<uniform> lights: PointLights;

fn cell_to_world(c: vec3<f32>) -> vec3<f32> {
    let uvw = (c + vec3<f32>(0.5)) / u.grid_size.xyz;
    return mix(u.grid_min.xyz, u.grid_max.xyz, uvw);
}

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec3<u32>(u32(u.grid_size.x), u32(u.grid_size.y), u32(u.grid_size.z));
    if (gid.x >= dims.x || gid.y >= dims.y || gid.z >= dims.z) {
        return;
    }
    let coord = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));

    // Density lives in the W channel of the velocity-density texture.
    let vd_sample = textureLoad(src_vd, coord, 0);
    let density = max(vd_sample.w, 0.0);

    // World-space center of the voxel — same `cell_to_world` convention
    // every other fluid pass uses, so the lighting matches the actual
    // smoke geometry exactly.
    let pos = cell_to_world(vec3<f32>(f32(gid.x), f32(gid.y), f32(gid.z)));

    let light_strength = cam.params.z;
    let ambient = cam.params.w;
    let lcount = lights.count.x;

    // ── Lighting (verbatim from the original fragment shader) ──────────
    // Two non-obvious deviations from a naïve `(1 - dist/radius)²`
    // attenuation that the raymarch had — both retained here so the
    // pre-lit voxels look identical to the per-step lighting they
    // replace:
    //
    //   1. Minimum distance clamp at 28% of the light radius. The
    //      smoke plume from a wick sits *right next to* its own light
    //      source — without a floor on the effective distance, every
    //      voxel near the candle gets `falloff ≈ 1` and the column
    //      blows out into a vertical white shaft.
    //   2. Smoke albedo = (0.42, 0.40, 0.37). Real smoke scatters only
    //      a fraction of incoming light forward; the dark grey base
    //      keeps even brightly-lit cells reading as smoke rather than
    //      glowing fog.
    var lit = vec3<f32>(ambient);
    for (var li: u32 = 0u; li < lcount; li = li + 1u) {
        let l = lights.lights[li];
        let to_light = l.pos.xyz - pos;
        let dist = sqrt(max(dot(to_light, to_light), 1.0));
        let radius = max(l.pos.w, 1.0);
        let min_dist = radius * 0.28;
        let eff_dist = max(dist, min_dist);
        let falloff = clamp(1.0 - eff_dist / radius, 0.0, 1.0);
        lit = lit + l.color.rgb * l.color.a * falloff * falloff * light_strength;
    }
    // Reinhard so overlapping candle radii don't push past the smoke's
    // own albedo and clip to white.
    lit = lit / (vec3<f32>(1.0) + lit * 0.6);
    let smoke_color = vec3<f32>(0.42, 0.40, 0.37) * lit;

    // Always write smoke_color, even when density is zero — the bilinear
    // sampler in the raymarch interpolates between neighbouring voxels,
    // and zeroing the colour at empty cells would darken the plume edges
    // toward black during the lerp.
    textureStore(dst_lit, coord, vec4<f32>(smoke_color, density));
}
