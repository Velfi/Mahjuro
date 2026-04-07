// Fluid pressure solver — divergence, Jacobi iteration, and velocity projection.
// Mode is selected via uniform: 0 = divergence, 1 = Jacobi, 2 = projection.

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

struct PressureParams {
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var<uniform> params: PressureParams;
@group(0) @binding(2) var<storage, read_write> velocity_x: array<f32>;
@group(0) @binding(3) var<storage, read_write> velocity_y: array<f32>;
@group(0) @binding(4) var<storage, read> pressure_src: array<f32>;
@group(0) @binding(5) var<storage, read_write> pressure_dst: array<f32>;
@group(0) @binding(6) var<storage, read_write> divergence: array<f32>;

fn idx(x: u32, y: u32) -> u32 {
    return y * u32(fluid.grid_w) + x;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    let w = u32(fluid.grid_w);
    let h = u32(fluid.grid_h);
    if (x >= w || y >= h) {
        return;
    }

    let i = idx(x, y);

    // Clamped neighbor indices for boundary handling.
    let xl = select(x - 1u, 0u, x == 0u);
    let xr = select(x + 1u, w - 1u, x >= w - 1u);
    let yb = select(y - 1u, 0u, y == 0u);
    let yt = select(y + 1u, h - 1u, y >= h - 1u);

    if (params.mode == 0u) {
        // Divergence: div = 0.5 * (vx[right] - vx[left] + vy[top] - vy[bottom])
        let div = 0.5 * (
            velocity_x[idx(xr, y)] - velocity_x[idx(xl, y)] +
            velocity_y[idx(x, yt)] - velocity_y[idx(x, yb)]
        );
        divergence[i] = div;
        pressure_dst[i] = 0.0; // Clear pressure for first Jacobi iteration.
    } else if (params.mode == 1u) {
        // Jacobi iteration: p = (p_left + p_right + p_bottom + p_top - div) / 4
        let p = (
            pressure_src[idx(xl, y)] + pressure_src[idx(xr, y)] +
            pressure_src[idx(x, yb)] + pressure_src[idx(x, yt)] -
            divergence[i]
        ) * 0.25;
        pressure_dst[i] = p;
    } else {
        // Projection: subtract pressure gradient from velocity.
        let dp_dx = 0.5 * (pressure_src[idx(xr, y)] - pressure_src[idx(xl, y)]);
        let dp_dy = 0.5 * (pressure_src[idx(x, yt)] - pressure_src[idx(x, yb)]);
        velocity_x[i] -= dp_dx;
        velocity_y[i] -= dp_dy;
    }
}
