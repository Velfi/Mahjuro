// 3D Jacobi iteration: p_new = (Σ p_neighbors - div) / 6

struct FluidUniforms {
    grid_size:    vec4<f32>,
    grid_min:     vec4<f32>,
    grid_max:     vec4<f32>,
    inv_extent:   vec4<f32>,
    params:       vec4<f32>,
    force_params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var p_src: texture_3d<f32>;
@group(0) @binding(2) var div_src: texture_3d<f32>;
@group(0) @binding(3) var p_dst: texture_storage_3d<r32float, write>;

fn p(c: vec3<i32>) -> f32 {
    return textureLoad(p_src, c, 0).x;
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
    let inv_cell = fluid.grid_size.xyz * fluid.inv_extent.xyz;
    let ax = inv_cell.x * inv_cell.x;
    let ay = inv_cell.y * inv_cell.y;
    let az = inv_cell.z * inv_cell.z;

    let div = textureLoad(div_src, c, 0).x;
    let p_new = (
        (p(xl) + p(xr)) * ax +
        (p(yl) + p(yr)) * ay +
        (p(zl) + p(zr)) * az -
        div
    ) / (2.0 * (ax + ay + az));
    textureStore(p_dst, c, vec4<f32>(p_new, 0.0, 0.0, 0.0));
}
