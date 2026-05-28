// Volumetric irradiance probes: project emissive-only radiance into L2 SH via screen-space ray march.
// One thread per probe; Fibonacci sphere directions; uniform sphere Monte Carlo weights.

const PI: f32 = 3.14159265358979323846;
const MAX_PROBES: u32 = 256u;
const GOLDEN: f32 = 2.39996322972865332; // π * (3 - √5)

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
@group(0) @binding(1) var emissive_tex: texture_2d<f32>;
@group(0) @binding(2) var depth_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var<storage, read_write> probes: array<ProbeSh>;

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

fn world_at_uv_depth(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0, depth);
    let world = p.inv_view_proj * vec4<f32>(ndc, 1.0);
    return world.xyz / max(world.w, 1e-6);
}

fn march_emissive(origin: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    let full_w = max(p.screen_march.x, 1.0);
    let full_h = max(p.screen_march.y, 1.0);
    let march_max = p.screen_march.z;
    let n_steps = max(p.sample_params.y, 1u);
    let step_len = march_max / f32(n_steps);
    let bias = 0.0015;

    for (var s: u32 = 1u; s <= n_steps; s = s + 1u) {
        let t = step_len * f32(s);
        let q = origin + dir * t;
        let clip = p.view_proj * vec4<f32>(q, 1.0);
        if (clip.w <= 0.0) {
            break;
        }
        let inv_w = 1.0 / clip.w;
        let ndc = clip.xyz * inv_w;
        if (ndc.z <= 0.0 || ndc.z >= 1.0) {
            break;
        }
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, ndc.y * -0.5 + 0.5);
        if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
            break;
        }
        let px = vec2<i32>(
            i32(clamp(uv.x * full_w - 0.5, 0.0, full_w - 1.0)),
            i32(clamp(uv.y * full_h - 0.5, 0.0, full_h - 1.0)),
        );
        let d_s = textureLoad(depth_tex, px, 0);
        if (d_s >= 0.9999) {
            continue;
        }
        if (abs(d_s - ndc.z) < bias) {
            return textureSampleLevel(emissive_tex, samp, uv, 0.0).rgb;
        }
    }
    return vec3<f32>(0.0);
}

fn fib_dir(i: u32, n: u32) -> vec3<f32> {
    let m = f32(i) + 0.5;
    let nd = max(f32(n), 1.0);
    let y = 1.0 - 2.0 * m / nd;
    let r = sqrt(max(1.0 - y * y, 0.0));
    let phi = GOLDEN * m;
    return normalize(vec3<f32>(cos(phi) * r, sin(phi) * r, y));
}

@compute @workgroup_size(64)
fn update_probes(@builtin(global_invocation_id) gid: vec3<u32>) {
    let pid = gid.x;
    let count = p.grid_dims.w;
    if (pid >= count || pid >= MAX_PROBES) {
        return;
    }

    let nx = max(p.grid_dims.x, 1u);
    let ny = max(p.grid_dims.y, 1u);
    let nz = max(p.grid_dims.z, 1u);
    let iz = pid / (nx * ny);
    let rem = pid - iz * nx * ny;
    let iy = rem / nx;
    let ix = rem - iy * nx;

    let mn = p.world_min.xyz;
    let mx = p.world_max.xyz;
    let ext = max(mx - mn, vec3<f32>(1e-4));
    let cell = vec3<f32>(
        (f32(ix) + 0.5) / f32(max(nx, 1u)),
        (f32(iy) + 0.5) / f32(max(ny, 1u)),
        (f32(iz) + 0.5) / f32(max(nz, 1u)),
    );
    let origin = mn + cell * ext;

    let n_dir = max(p.sample_params.x, 4u);
    let w = (4.0 * PI) / f32(n_dir);

    var acc: array<vec3<f32>, 9>;
    for (var k: u32 = 0u; k < 9u; k = k + 1u) {
        acc[k] = vec3<f32>(0.0);
    }

    for (var j: u32 = 0u; j < n_dir; j = j + 1u) {
        let dir = fib_dir(j, n_dir);
        let L = march_emissive(origin, dir);
        if (dot(L, L) < 1e-12) {
            continue;
        }
        let B = sh9_basis(dir);
        for (var k: u32 = 0u; k < 9u; k = k + 1u) {
            acc[k] = acc[k] + L * B[k] * w;
        }
    }

    var out: ProbeSh;
    for (var k: u32 = 0u; k < 9u; k = k + 1u) {
        out.sh[k] = vec4<f32>(acc[k], 0.0);
    }
    probes[pid] = out;
}
