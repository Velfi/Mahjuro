// Generic lit-mesh shader used by procedural scene props (candles, table).
//
// One pipeline, one shader; the `material_kind` field of MeshUniform selects
// the per-fragment look:
//
//   0.0 = plain     — lit base color
//   1.0 = wax       — pale beeswax with a high ambient floor (fake SSS)
//   2.0 = wick      — dark, no specular
//   3.0 = lacquered wood — procedural ring grain + Blinn-Phong specular
//
// All material variants share the candle/spot point-light loop from the tile
// shader so the new geometry catches the same warm pools as the hand tiles.

struct MeshUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    base_color: vec4<f32>,
    // x = material_kind, y = specular_strength, z = specular_power, w = unused
    material_params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> mesh: MeshUniform;
@group(0) @binding(1) var albedo_tex: texture_2d<f32>;
@group(0) @binding(2) var albedo_samp: sampler;

struct PointLight {
    pos: vec4<f32>,   // xyz = world position, w = radius
    color: vec4<f32>, // rgb = color, a = intensity
};

struct PointLights {
    count: vec4<u32>,
    lights: array<PointLight, 8>,
};

@group(1) @binding(0) var<uniform> lights: PointLights;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_n: vec3<f32>,
    @location(2) local_pos: vec3<f32>,
    @location(3) uv: vec2<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
) -> VsOut {
    var displaced = pos;
    var world_normal = normalize((mesh.model * vec4<f32>(n, 0.0)).xyz);

    // Lacquered wood: displace along the local +Z (table normal in
    // local space) using the same procedural field the FS uses for
    // shading, then rebuild the normal from a finite-difference of the
    // height field. The model matrix has wildly non-uniform scale
    // (X/Y are huge, Z is 1.0), so we evaluate the gradient in WORLD
    // units by multiplying the local epsilon by the model's column
    // lengths — otherwise the slopes would be off by 1000× and the
    // surface would look mirror-flat.
    if (mesh.material_params.x > 2.5) {
        let scale_x = length(mesh.model[0].xyz); // local +X axis world length
        let scale_y = length(mesh.model[1].xyz); // local +Y axis world length
        // Local Z scale is 1 in the table model matrix, so a unit of
        // height here is a unit in world Y. Tune amplitude in world units.
        let amp = 1.6;
        let eps = 1.0 / 200.0; // matches build_table_mesh segments

        let h_c = wood_height(pos.xy);
        let h_x = wood_height(pos.xy + vec2<f32>(eps, 0.0));
        let h_y = wood_height(pos.xy + vec2<f32>(0.0, eps));

        displaced.z = pos.z + h_c * amp;

        // Rotation Rx(-90°) maps local +X → world +X, local +Y → world -Z,
        // local +Z → world +Y. So the world-space tangent vectors are:
        let dh_x = (h_x - h_c) * amp;
        let dh_y = (h_y - h_c) * amp;
        let t_x = vec3<f32>(eps * scale_x, dh_x, 0.0);
        let t_y = vec3<f32>(0.0,            dh_y, -eps * scale_y);
        world_normal = normalize(cross(t_x, t_y));
    }

    let world = mesh.model * vec4<f32>(displaced, 1.0);
    var o: VsOut;
    o.clip_pos = mesh.view_proj * world;
    o.world_pos = world.xyz;
    o.world_n = world_normal;
    // Pass the *undisplaced* local position so the FS samples the wood
    // basis at the same surface coordinate the VS displaced from.
    o.local_pos = pos;
    o.uv = uv;
    return o;
}

// Cheap value-noise hash. Used by the wood material for grain rings.
fn hash11(p: f32) -> f32 {
    return fract(sin(p * 127.1) * 43758.5453);
}
fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// Smooth value noise on a 2D lattice. Bilinear-blends 4 hashed corners
// with a smoothstep interpolant — cheap, no derivatives required.
fn vnoise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i + vec2<f32>(0.0, 0.0));
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// 4-octave fractal value noise.
fn fbm2(p_in: vec2<f32>) -> f32 {
    var p = p_in;
    var f = 0.0;
    var amp = 0.5;
    for (var i = 0; i < 4; i = i + 1) {
        f = f + amp * vnoise2(p);
        p = p * 2.03 + vec2<f32>(17.0, 9.0);
        amp = amp * 0.5;
    }
    return f;
}

struct WoodSample {
    albedo: vec3<f32>,
    // Grain weight in [0,1] — high on early-wood ring peaks. Used to
    // modulate the lacquer specular so highlights pool over polished
    // grain rather than darker pores.
    grain: f32,
    // Pore mask in [0,1] — high inside open-pore voids that should
    // suppress specular and slightly darken the diffuse term.
    pore: f32,
};

// Shared evaluation of the procedural wood field. Returns the structural
// quantities (ring weights, pore mask, tone) at a given local-XY surface
// coordinate. Both `wood_height` (used by the vertex shader for geometric
// displacement) and `wood_sample` (used by the fragment shader for color)
// derive from this so the bumps and the grain colors stay locked to the
// same physical surface.
struct WoodBasis {
    late_wood: f32,
    early_wood: f32,
    pore: f32,
    tone: f32,
    fiber: f32,
};

fn wood_basis(local_xy: vec2<f32>) -> WoodBasis {
    // Stretch X harder than Y so rings are tall and sweeping, the way a
    // long board would look. Scale matches the ring count we want across
    // the visible slab — large because the table model matrix scales the
    // local [-0.5, 0.5] range up to thousands of world units.
    let p = vec2<f32>(local_xy.x * 14.0, local_xy.y * 4.0);

    // Distort the ring axis with two octaves of fbm so rings curve
    // organically and occasionally pinch / fork. Amplitude is low so
    // rings stay recognizable as rings rather than smearing into marble.
    let warp = vec2<f32>(
        fbm2(p * 0.35 + vec2<f32>(2.7, 0.0)),
        fbm2(p * 0.35 + vec2<f32>(0.0, 5.1)),
    ) - 0.5;
    let wp = p + warp * 0.6;

    // Distance from the off-screen "pith" of the log times a ring
    // frequency, perturbed so spacing is not perfectly periodic.
    let ring_freq = 1.6;
    let pith_x = -8.0;
    let r = (wp.x - pith_x) * ring_freq + sin(wp.y * 0.4) * 0.35;
    let r_jitter = fbm2(wp * 0.25) * 0.8;
    let ring = fract(r + r_jitter);

    // Asymmetric ring profile: narrow dark late-wood stripe followed by
    // a broader warm early-wood band.
    let tri = 1.0 - abs(ring * 2.0 - 1.0);
    let late_wood = pow(tri, 9.0);
    let early_wood = pow(tri, 1.6);

    // High-freq fibrous streaks running along the grain axis.
    let fiber_p = vec2<f32>(wp.x * 80.0, wp.y * 4.0);
    let fiber = (vnoise2(fiber_p) - 0.5) * 0.18;

    // Sparse open pores concentrated in early-wood bands.
    let pore_p = vec2<f32>(wp.x * 55.0, wp.y * 28.0);
    let pore_n = vnoise2(pore_p) * vnoise2(pore_p * 1.7 + 3.1);
    let pore = smoothstep(0.55, 0.78, pore_n) * (0.4 + 0.6 * early_wood);

    let tone = fbm2(p * 0.18) * 0.5 + 0.5;

    var b: WoodBasis;
    b.late_wood = late_wood;
    b.early_wood = early_wood;
    b.pore = pore;
    b.tone = tone;
    b.fiber = fiber;
    return b;
}

// Signed surface height from the wood basis. Positive on early-wood
// ridges, negative inside late-wood lines and pores. Returned in the
// same units as the model's local Z so the vertex shader can scale it
// directly into world units.
fn wood_height(local_xy: vec2<f32>) -> f32 {
    let b = wood_basis(local_xy);
    // Early-wood bulges up; late-wood and pores recess. The mean is
    // close to zero so displacement does not visibly raise the slab.
    return b.early_wood * 0.55 - b.late_wood * 0.85 - b.pore * 1.6;
}

fn wood_sample_basis(b: WoodBasis) -> WoodSample {
    // True walnut palette — deep, low-value, narrow dynamic range. Real
    // walnut linear-space albedo barely exceeds ~0.10 even on the
    // brightest fibers; the brown reads dark in any reasonable lighting.
    let dark = mix(
        vec3<f32>(0.018, 0.0085, 0.0040),
        vec3<f32>(0.030, 0.0140, 0.0065),
        b.tone,
    );
    let mid = mix(
        vec3<f32>(0.055, 0.028, 0.013),
        vec3<f32>(0.075, 0.038, 0.018),
        b.tone,
    );
    let light = mix(
        vec3<f32>(0.090, 0.050, 0.025),
        vec3<f32>(0.110, 0.062, 0.030),
        b.tone,
    );

    // Late-wood is the dominant *darker* state; early-wood is a modest
    // lift. The previous version had this inverted in feel because the
    // mid value was already too high.
    var c = mix(dark, mid, 1.0 - b.late_wood);
    c = mix(c, light, b.early_wood * 0.55);
    c = c + vec3<f32>(b.fiber) * vec3<f32>(0.6, 0.45, 0.30);
    c = mix(c, c * 0.40, b.pore);

    var s: WoodSample;
    s.albedo = max(c, vec3<f32>(0.0));
    s.grain = clamp(b.early_wood * 0.7 + 0.3, 0.0, 1.0);
    s.pore = b.pore;
    return s;
}

fn wood_sample(local_pos: vec3<f32>) -> WoodSample {
    return wood_sample_basis(wood_basis(local_pos.xy));
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let kind = mesh.material_params.x;
    let spec_strength = mesh.material_params.y;
    let spec_power = max(mesh.material_params.z, 1.0);

    // Sample the albedo texture unconditionally — material kind is uniform
    // across the draw, but hoisting the sample keeps naga's uniform-control-
    // flow analysis happy regardless of how it inlines the branch below.
    let tex_rgb = textureSample(albedo_tex, albedo_samp, in.uv).rgb;
    var albedo = mesh.base_color.rgb * tex_rgb;
    var wood_grain = 0.0;
    var wood_pore = 0.0;
    if (kind > 2.5) {
        // Lacquered wood: procedural grain overrides the (white) albedo tex.
        let w = wood_sample(in.local_pos);
        albedo = w.albedo;
        wood_grain = w.grain;
        wood_pore = w.pore;
    }

    // Key-light ambient + soft top diffuse (matches the tile shader's mood).
    var n = normalize(in.world_n);
    if (!front_facing) {
        n = -n;
    }
    let key_dir = normalize(vec3<f32>(0.25, 1.0, 0.35));
    let nk = max(dot(n, key_dir), 0.0);
    var ambient = 0.55;
    var diffuse_strength = 0.35;
    if (kind > 0.5 && kind < 1.5) {
        // Wax: moderate ambient — the per-light back-transmission term
        // below now does the heavy lifting that the old high ambient
        // floor (0.78) was faking. Lower ambient lets the wax react
        // dynamically to the candle pools instead of looking flat.
        ambient = 0.42;
        diffuse_strength = 0.28;
    } else if (kind > 1.5 && kind < 2.5) {
        // Wick: dark, very low ambient.
        ambient = 0.25;
        diffuse_strength = 0.20;
    } else if (kind > 2.5) {
        // Lacquered wood: very low ambient — walnut is dark, and we
        // want the candle pools to be the dominant light source.
        ambient = 0.14;
        diffuse_strength = 0.20;
    }
    let key_shade = ambient + diffuse_strength * nk;
    var rgb = albedo * key_shade;

    // ── Point-light pass ─────────────────────────────────────────────────
    // Distance attenuation + Lambertian, with optional Blinn-Phong specular,
    // optional fake subsurface wrap (wood/wax), and an optional dielectric
    // clearcoat lobe (lacquered wood).
    var lit = vec3<f32>(0.0);          // diffuse-band light (multiplied by albedo)
    var sss_acc = vec3<f32>(0.0);      // wrap-diffuse subsurface (front-lit)
    var wax_back_acc = vec3<f32>(0.0); // wax back-transmission (Penner-style)
    var spec_acc = vec3<f32>(0.0);     // base specular accumulator
    var coat_acc = vec3<f32>(0.0);     // clearcoat accumulator (white, untinted)

    let cam_pos = vec3<f32>(0.0, 0.0, 4000.0); // approximate; only direction matters
    let view_dir = normalize(cam_pos - in.world_pos);
    let ndv_view = clamp(dot(n, view_dir), 0.0, 1.0);

    let is_wood = kind > 2.5;
    let is_wax  = (kind > 0.5 && kind < 1.5);

    // Wrap-diffuse subsurface: softens the terminator past 90° so the
    // shaded side picks up a tinted bleed. Wood gets a tiny amount,
    // wax gets a much stronger wrap because beeswax really *does*
    // scatter light through its surface millimeters.
    var wrap = 0.0;
    var sss_strength = 0.0;
    var sss_tint = vec3<f32>(0.0);
    if (is_wood) {
        wrap = 0.40;
        sss_strength = 0.18;
        sss_tint = vec3<f32>(0.18, 0.085, 0.035);
    } else if (is_wax) {
        wrap = 0.55;
        sss_strength = 0.55;
        sss_tint = vec3<f32>(1.00, 0.78, 0.42);
    }

    // ── Wax back-transmission (Penner SSS) tunables ─────────────────
    // The classic cheap translucency model: the effective light
    // direction is bent toward the surface normal by `distortion`,
    // then we measure how much of that bent light vector points
    // *toward* the camera (i.e. through the back of the object).
    // The result is heavily concentrated on the silhouette of an
    // object that is back-lit, exactly where real wax glows.
    let wax_distortion = 0.45;
    let wax_back_power = 4.0;
    let wax_back_scale = select(0.0, 1.4, is_wax);
    let wax_tint = vec3<f32>(1.00, 0.72, 0.32);

    // Thickness proxy from local geometry. The candle mesh is built
    // with the wax body running from y≈0 (base) to y≈0.555 (wax pool
    // around the wick). Thinness is high near the top of the votive
    // (the rim flare and the wax pool itself are thin slabs of wax)
    // and at silhouette edges (where the camera sees a glancing slice
    // of wax). Both are the places real candle wax glows from inside.
    var wax_thinness = 0.0;
    if (is_wax) {
        let height01 = clamp(in.local_pos.y / 0.56, 0.0, 1.0);
        let edge = 1.0 - max(dot(n, view_dir), 0.0);
        let top_thin = smoothstep(0.30, 0.92, height01);
        let edge_thin = pow(edge, 2.0);
        wax_thinness = clamp(top_thin * 0.85 + edge_thin * 0.65, 0.0, 1.6);
    }

    // Clearcoat tunables. F0 = 0.04 (dielectric ~IOR 1.5). The coat is
    // narrower than the wood's grain specular so it reads as a separate
    // glassy layer rather than just "more highlight".
    let coat_strength = select(0.0, 0.55, is_wood);
    let coat_power = 320.0;
    let coat_f0 = 0.04;

    let count = lights.count.x;
    for (var i: u32 = 0u; i < count; i = i + 1u) {
        let lp = lights.lights[i].pos.xyz;
        let radius = lights.lights[i].pos.w;
        let lc = lights.lights[i].color.rgb;
        let intensity = lights.lights[i].color.a;
        let to_light = lp - in.world_pos;
        let dist = length(to_light);
        let t = clamp(1.0 - dist / max(radius, 1.0), 0.0, 1.0);
        let atten = t * t;
        let l_dir = to_light / max(dist, 0.0001);
        let ndl_raw = dot(n, l_dir);
        let nl = max(ndl_raw, 0.0);
        let lambert = 0.35 + 0.65 * nl;
        lit = lit + lc * intensity * atten * lambert;

        // Wrap-diffuse SSS: pushes the terminator past 90° so the
        // shaded side of the surface picks up a soft tinted bleed,
        // mimicking light scattering through the top millimeter of
        // wood or wax. Wood pores damp the effect (open grain absorbs
        // more than it scatters); wax has no equivalent term.
        if (sss_strength > 0.001) {
            let wrapped = max((ndl_raw + wrap) / (1.0 + wrap), 0.0);
            // Take the wrap energy *beyond* the normal Lambert term so
            // we don't double-count light on the lit hemisphere.
            let sss_band = max(wrapped - nl, 0.0);
            let sss_mask = select(1.0, 1.0 - wood_pore * 0.7, is_wood);
            sss_acc = sss_acc + lc * intensity * atten * sss_band * sss_strength * sss_mask;
        }

        // Wax back-transmission (Penner SSS). Bend the light direction
        // toward the surface normal, then measure how much of that
        // bent vector points *toward* the viewer (i.e. comes through
        // the back of the wax). The term peaks when a candle is
        // back-lit by another candle's flame and you see its near
        // side glow — exactly the look real wax has in a dim room.
        if (wax_back_scale > 0.001) {
            let lt = normalize(l_dir + n * wax_distortion);
            let back = pow(max(dot(view_dir, -lt), 0.0), wax_back_power);
            wax_back_acc = wax_back_acc
                + lc * intensity * atten * back * wax_thinness * wax_back_scale;
        }

        if (spec_strength > 0.001) {
            let h = normalize(l_dir + view_dir);
            let nh = max(dot(n, h), 0.0);
            // Two-lobe Blinn-Phong: a sharp grain highlight plus a
            // softer underlying sheen. Pores knock both lobes down so
            // open-grain voids stay matte.
            let sharp = pow(nh, spec_power);
            let soft  = pow(nh, max(spec_power * 0.25, 1.0)) * 0.35;
            var s = (sharp + soft) * spec_strength;
            if (is_wood) {
                s = s * mix(0.55, 1.15, wood_grain) * (1.0 - wood_pore * 0.85);
            }
            spec_acc = spec_acc + lc * intensity * atten * s;
        }

        // Clearcoat lobe — Schlick Fresnel against the half-vector,
        // narrow Blinn-Phong, white (the coat is a clear dielectric so
        // it does not pick up the wood's color). Pores get less coat
        // because they are physically below the lacquer surface.
        if (coat_strength > 0.001) {
            let h = normalize(l_dir + view_dir);
            let nh = max(dot(n, h), 0.0);
            let vdh = max(dot(view_dir, h), 0.0);
            let f_schlick = coat_f0 + (1.0 - coat_f0) * pow(1.0 - vdh, 5.0);
            let d = pow(nh, coat_power) * (coat_power + 2.0) / 8.0;
            let coat = d * f_schlick * coat_strength * (1.0 - wood_pore * 0.6);
            coat_acc = coat_acc + lc * intensity * atten * coat;
        }
    }

    // Compose: diffuse light multiplies albedo; sss adds a tinted glow;
    // base specular and clearcoat add on top. For wood we Fresnel-fade
    // the diffuse so energy flows into the coat at glancing angles.
    var diffuse_scale = 1.0;
    if (is_wood) {
        let f_view = coat_f0 + (1.0 - coat_f0) * pow(1.0 - ndv_view, 5.0);
        diffuse_scale = 1.0 - f_view * 0.6;
    }
    rgb = rgb
        + albedo * lit * diffuse_scale
        + sss_acc * sss_tint
        + wax_back_acc * wax_tint
        + spec_acc
        + coat_acc;

    if (is_wood) {
        // Ambient Fresnel rim: a *very* subtle cool kiss only at the
        // most glancing angles. The previous value blew out the whole
        // far edge of the table into a milky band.
        let fres = pow(1.0 - ndv_view, 6.0);
        rgb = rgb + vec3<f32>(0.025, 0.030, 0.045) * fres * (1.0 - wood_pore);
    }

    return vec4<f32>(rgb, mesh.base_color.a);
}
