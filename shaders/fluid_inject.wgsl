// Fluid injection compute shader — Gaussian splat of velocity + colored density.

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

struct InjectionPoint {
    pos_radius: vec4<f32>,   // (grid_x, grid_y, radius, strength)
    vel_density: vec4<f32>,  // (vel_x, vel_y, _, _)
    color_pad: vec4<f32>,    // (density_r, density_g, density_b, _)
};

struct InjectionParams {
    points: array<InjectionPoint, 8>,
    active_count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var<uniform> fluid: FluidUniforms;
@group(0) @binding(1) var<uniform> injection: InjectionParams;
@group(0) @binding(2) var<storage, read_write> velocity_x: array<f32>;
@group(0) @binding(3) var<storage, read_write> velocity_y: array<f32>;
@group(0) @binding(4) var<storage, read_write> density_r: array<f32>;
@group(0) @binding(5) var<storage, read_write> density_g: array<f32>;
@group(0) @binding(6) var<storage, read_write> density_b: array<f32>;

fn idx(x: u32, y: u32) -> u32 {
    return y * u32(fluid.grid_w) + x;
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

    var dvx = 0.0;
    var dvy = 0.0;
    var dr = 0.0;
    var dg = 0.0;
    var db = 0.0;

    for (var p = 0u; p < injection.active_count; p = p + 1u) {
        let pt = injection.points[p];
        let center = vec2<f32>(pt.pos_radius.x, pt.pos_radius.y);
        let radius = pt.pos_radius.z;
        let strength = pt.pos_radius.w;

        let diff = cell - center;
        let dist2 = dot(diff, diff);
        let r2 = radius * radius;
        let gauss = strength * exp(-dist2 / (2.0 * r2));

        dvx += pt.vel_density.x * gauss;
        dvy += pt.vel_density.y * gauss;
        dr += pt.color_pad.x * gauss;
        dg += pt.color_pad.y * gauss;
        db += pt.color_pad.z * gauss;
    }

    velocity_x[i] += dvx;
    velocity_y[i] += dvy;
    density_r[i] += dr;
    density_g[i] += dg;
    density_b[i] += db;
}
