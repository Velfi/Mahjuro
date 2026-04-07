// Fluid advection compute shader — semi-Lagrangian advection with bilinear interpolation.

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
// Source buffers (read)
@group(0) @binding(1) var<storage, read> src_vx: array<f32>;
@group(0) @binding(2) var<storage, read> src_vy: array<f32>;
@group(0) @binding(3) var<storage, read> src_dr: array<f32>;
@group(0) @binding(4) var<storage, read> src_dg: array<f32>;
@group(0) @binding(5) var<storage, read> src_db: array<f32>;
// Destination buffers (write)
@group(0) @binding(6) var<storage, read_write> dst_vx: array<f32>;
@group(0) @binding(7) var<storage, read_write> dst_vy: array<f32>;
@group(0) @binding(8) var<storage, read_write> dst_dr: array<f32>;
@group(0) @binding(9) var<storage, read_write> dst_dg: array<f32>;
@group(0) @binding(10) var<storage, read_write> dst_db: array<f32>;

fn idx(x: u32, y: u32) -> u32 {
    return y * u32(fluid.grid_w) + x;
}

fn bilerp_weights(pos: vec2<f32>) -> vec4<f32> {
    let gw = fluid.grid_w;
    let gh = fluid.grid_h;
    let p = clamp(pos - vec2<f32>(0.5, 0.5), vec2<f32>(0.0), vec2<f32>(gw - 1.001, gh - 1.001));
    let fx = p.x - floor(p.x);
    let fy = p.y - floor(p.y);
    return vec4<f32>(floor(p.x), floor(p.y), fx, fy);
}

fn bilerp(v00: f32, v10: f32, v01: f32, v11: f32, fx: f32, fy: f32) -> f32 {
    return mix(mix(v00, v10, fx), mix(v01, v11, fx), fy);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= u32(fluid.grid_w) || y >= u32(fluid.grid_h)) {
        return;
    }

    let i = idx(x, y);
    let cell = vec2<f32>(f32(x) + 0.5, f32(y) + 0.5);

    // Trace backward along velocity field.
    let vx = src_vx[i];
    let vy = src_vy[i];
    let back = cell - vec2<f32>(vx, vy) * fluid.dt;

    // Compute bilinear interpolation coordinates.
    let bw = bilerp_weights(back);
    let x0 = u32(bw.x);
    let y0 = u32(bw.y);
    let x1 = x0 + 1u;
    let y1 = y0 + 1u;
    let fx = bw.z;
    let fy = bw.w;
    let w = u32(fluid.grid_w);

    // Sample source fields at traced position.
    dst_vx[i] = bilerp(src_vx[y0*w+x0], src_vx[y0*w+x1], src_vx[y1*w+x0], src_vx[y1*w+x1], fx, fy) * fluid.velocity_dissipation;
    dst_vy[i] = bilerp(src_vy[y0*w+x0], src_vy[y0*w+x1], src_vy[y1*w+x0], src_vy[y1*w+x1], fx, fy) * fluid.velocity_dissipation;
    dst_dr[i] = bilerp(src_dr[y0*w+x0], src_dr[y0*w+x1], src_dr[y1*w+x0], src_dr[y1*w+x1], fx, fy) * fluid.density_dissipation;
    dst_dg[i] = bilerp(src_dg[y0*w+x0], src_dg[y0*w+x1], src_dg[y1*w+x0], src_dg[y1*w+x1], fx, fy) * fluid.density_dissipation;
    dst_db[i] = bilerp(src_db[y0*w+x0], src_db[y0*w+x1], src_db[y1*w+x0], src_db[y1*w+x1], fx, fy) * fluid.density_dissipation;
}
