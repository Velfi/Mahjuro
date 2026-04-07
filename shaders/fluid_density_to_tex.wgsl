// Convert density storage buffers (R, G, B) into an RGBA texture for rendering.

struct FluidUniforms {
    grid_w: f32,
    grid_h: f32,
    inv_grid_w: f32,
    inv_grid_h: f32,
    dt: f32,
    density_dissipation: f32,
    velocity_dissipation: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var<storage, read> density_r: array<f32>;
@group(0) @binding(2) var<storage, read> density_g: array<f32>;
@group(0) @binding(3) var<storage, read> density_b: array<f32>;
@group(0) @binding(4) var out_tex: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= u32(fluid.grid_w) || y >= u32(fluid.grid_h)) {
        return;
    }

    let i = y * u32(fluid.grid_w) + x;
    let r = clamp(density_r[i], 0.0, 1.0);
    let g = clamp(density_g[i], 0.0, 1.0);
    let b = clamp(density_b[i], 0.0, 1.0);

    textureStore(out_tex, vec2<u32>(x, y), vec4<f32>(r, g, b, 1.0));
}
