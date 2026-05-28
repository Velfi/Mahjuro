// Half-res: trilinear SH probe interpolation + diffuse irradiance (cosine kernel on L2 SH).

struct ProbeGiFrameUniform {
    inv_view_proj: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    world_min: vec4<f32>,
    world_max: vec4<f32>,
    grid_dims: vec4<u32>,
    screen_march: vec4<f32>,
    cam_pos: vec4<f32>,
    sample_params: vec4<u32>,
}

struct ProbeSh {
    sh: array<vec4<f32>, 9>,
}

@group(0) @binding(0) var<uniform> p: ProbeGiFrameUniform;
@group(0) @binding(1) var<storage, read> probes: array<ProbeSh>;
@group(0) @binding(2) var depth_tex: texture_2d<f32>;

const MAX_PROBES: u32 = 256u;

fn sh9_basis(d: vec3<f32>) -> array<f32, 9> {
    let x = d.x;
    let y = d.y;
    let z = d.z;
    var b: array<f32, 9>;
    b[0] = 0.282095;
    b[1] = -0.488603 * y;
    b[2] = 0.488603 * z;
    b[3] = -0.488603 * x;
    b[4] = 1.092548 * x * y;
    b[5] = -1.092548 * y * z;
    b[6] = 0.315392 * (3.0 * z * z - 1.0);
    b[7] = -1.092548 * x * z;
    b[8] = 0.546274 * (x * x - y * y);
    return b;
}

fn band_scale(i: u32) -> f32 {
    if (i == 0u) {
        return 3.14159265;
    }
    if (i < 4u) {
        return 2.0943951;
    }
    return 0.785398163;
}

fn eval_irradiance(n: vec3<f32>, L: array<vec3<f32>, 9>) -> vec3<f32> {
    let B = sh9_basis(n);
    var rgb = vec3<f32>(0.0);
    for (var k: u32 = 0u; k < 9u; k = k + 1u) {
        rgb = rgb + L[k] * B[k] * band_scale(k);
    }
    return max(rgb, vec3<f32>(0.0));
}

fn world_at_uv_depth(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0, depth);
    let world = p.inv_view_proj * vec4<f32>(ndc, 1.0);
    return world.xyz / max(world.w, 1e-6);
}

fn probe_at(ix: u32, iy: u32, iz: u32, nx: u32, ny: u32) -> u32 {
    return ix + nx * (iy + ny * iz);
}

fn lerp3(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    return a * (1.0 - t) + b * t;
}

fn lerp_sh(a: array<vec3<f32>, 9>, b: array<vec3<f32>, 9>, t: f32) -> array<vec3<f32>, 9> {
    var o: array<vec3<f32>, 9>;
    for (var k: u32 = 0u; k < 9u; k = k + 1u) {
        o[k] = lerp3(a[k], b[k], t);
    }
    return o;
}

fn read_probe_sh(pi: u32, count: u32) -> array<vec3<f32>, 9> {
    var o: array<vec3<f32>, 9>;
    if (pi >= count || pi >= MAX_PROBES) {
        for (var k: u32 = 0u; k < 9u; k = k + 1u) {
            o[k] = vec3<f32>(0.0);
        }
        return o;
    }
    for (var k: u32 = 0u; k < 9u; k = k + 1u) {
        o[k] = probes[pi].sh[k].xyz;
    }
    return o;
}

fn sample_sh_trilinear(world_pos: vec3<f32>) -> array<vec3<f32>, 9> {
    let nx = max(p.grid_dims.x, 1u);
    let ny = max(p.grid_dims.y, 1u);
    let nz = max(p.grid_dims.z, 1u);
    let count = p.grid_dims.w;

    let mn = p.world_min.xyz;
    let mx = p.world_max.xyz;
    let ext = max(mx - mn, vec3<f32>(1e-4));
    let t = (world_pos - mn) / ext;
    let tc = clamp(t, vec3<f32>(0.0), vec3<f32>(1.0));

    let fx = tc.x * f32(max(nx, 1u) - 1u);
    let fy = tc.y * f32(max(ny, 1u) - 1u);
    let fz = tc.z * f32(max(nz, 1u) - 1u);

    let ix0 = u32(clamp(floor(fx), 0.0, f32(nx - 1u)));
    let iy0 = u32(clamp(floor(fy), 0.0, f32(ny - 1u)));
    let iz0 = u32(clamp(floor(fz), 0.0, f32(nz - 1u)));
    let ix1 = min(ix0 + 1u, nx - 1u);
    let iy1 = min(iy0 + 1u, ny - 1u);
    let iz1 = min(iz0 + 1u, nz - 1u);

    let tx = fract(fx);
    let ty = fract(fy);
    let tz = fract(fz);

    let p000 = read_probe_sh(probe_at(ix0, iy0, iz0, nx, ny), count);
    let p100 = read_probe_sh(probe_at(ix1, iy0, iz0, nx, ny), count);
    let p010 = read_probe_sh(probe_at(ix0, iy1, iz0, nx, ny), count);
    let p110 = read_probe_sh(probe_at(ix1, iy1, iz0, nx, ny), count);
    let p001 = read_probe_sh(probe_at(ix0, iy0, iz1, nx, ny), count);
    let p101 = read_probe_sh(probe_at(ix1, iy0, iz1, nx, ny), count);
    let p011 = read_probe_sh(probe_at(ix0, iy1, iz1, nx, ny), count);
    let p111 = read_probe_sh(probe_at(ix1, iy1, iz1, nx, ny), count);

    let x0 = lerp_sh(p000, p100, tx);
    let x1 = lerp_sh(p010, p110, tx);
    let x2 = lerp_sh(p001, p101, tx);
    let x3 = lerp_sh(p011, p111, tx);
    let y0 = lerp_sh(x0, x1, ty);
    let y1 = lerp_sh(x2, x3, ty);
    return lerp_sh(y0, y1, tz);
}

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    let pp = pos[vid];
    var out: VsOut;
    out.clip_pos = vec4<f32>(pp, 0.0, 1.0);
    out.uv = vec2<f32>(pp.x * 0.5 + 0.5, 1.0 - (pp.y * 0.5 + 0.5));
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let full_w = max(p.screen_march.x, 1.0);
    let full_h = max(p.screen_march.y, 1.0);
    let half_w = max(i32(full_w * 0.5), 1);
    let half_h = max(i32(full_h * 0.5), 1);

    let hx = i32(clamp(in.uv.x * f32(half_w), 0.0, f32(half_w - 1)));
    let hy = i32(clamp(in.uv.y * f32(half_h), 0.0, f32(half_h - 1)));
    let pxf = f32(hx * 2);
    let pyf = f32(hy * 2);
    let center_uv = vec2<f32>((pxf + 0.5) / full_w, (pyf + 0.5) / full_h);
    let center_px = vec2<i32>(
        i32(clamp(pxf, 0.0, full_w - 1.0)),
        i32(clamp(pyf, 0.0, full_h - 1.0)),
    );

    let d_c = textureLoad(depth_tex, center_px, 0).x;
    if (d_c >= 0.9999) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    let pw = world_at_uv_depth(center_uv, d_c);
    let p_dx = dpdx(pw);
    let p_dy = dpdy(pw);
    let cp = cross(p_dx, p_dy);
    let ln = length(cp);
    if (ln < 1e-8) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    var n = normalize(cp);
    let vdir = normalize(p.cam_pos.xyz - pw);
    if (dot(n, vdir) < 0.0) {
        n = -n;
    }

    let L = sample_sh_trilinear(pw);
    let irr = eval_irradiance(n, L);
    let str = p.screen_march.w;
    return vec4<f32>(max(irr * str, vec3<f32>(0.0)), 1.0);
}
