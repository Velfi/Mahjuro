// 3D pressure projection — subtract pressure gradient from velocity to enforce ∇·v = 0.
// Reads vd_src, writes vd_dst (density passes through unchanged).

struct FluidUniforms {
    grid_size:    vec4<f32>,
    grid_min:     vec4<f32>,
    grid_max:     vec4<f32>,
    inv_extent:   vec4<f32>,
    params:       vec4<f32>,
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var src_vd: texture_3d<f32>;
@group(0) @binding(2) var pressure: texture_3d<f32>;
@group(0) @binding(3) var dst_vd: texture_storage_3d<rgba16float, write>;

fn p(c: vec3<i32>) -> f32 {
    return textureLoad(pressure, c, 0).x;
}

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec3<i32>(i32(fluid.grid_size.x), i32(fluid.grid_size.y), i32(fluid.grid_size.z));
    if (i32(gid.x) >= dims.x || i32(gid.y) >= dims.y || i32(gid.z) >= dims.z) {
        return;
    }
    let c = vec3<i32>(i32(gid.x), i32(gid.y), i32(gid.z));

    let xl = vec3<i32>(max(c.x - 1, 0), c.y, c.z);
    let xr = vec3<i32>(min(c.x + 1, dims.x - 1), c.y, c.z);
    let yl = vec3<i32>(c.x, max(c.y - 1, 0), c.z);
    let yr = vec3<i32>(c.x, min(c.y + 1, dims.y - 1), c.z);
    let zl = vec3<i32>(c.x, c.y, max(c.z - 1, 0));
    let zr = vec3<i32>(c.x, c.y, min(c.z + 1, dims.z - 1));

    let grad = vec3<f32>(
        0.5 * (p(xr) - p(xl)),
        0.5 * (p(yr) - p(yl)),
        0.5 * (p(zr) - p(zl)),
    );

    var vd = textureLoad(src_vd, c, 0);
    vd = vec4<f32>(vd.xyz - grad, vd.w);

    // Solid floor at min Y: clamp downward velocity, hold smoke up.
    if (c.y == 0) {
        vd.y = max(vd.y, 0.0);
    }

    textureStore(dst_vd, c, vd);
}
