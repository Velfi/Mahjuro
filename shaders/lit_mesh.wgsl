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
    // extras.x = display gamma exponent; rest reserved.
    extras: vec4<f32>,
    lights: array<PointLight, 16>,
};

@group(1) @binding(0) var<uniform> lights: PointLights;

// ── Tile occluders (analytic AABB shadows for the candle pools) ──────
// Each entry is one hand tile's world-space AABB. The fragment shader
// casts a ray from each candle to the shaded fragment and discards a
// light's contribution if the ray pierces any occluder before reaching
// the surface. This is what gives the table its tile-shaped shadow
// pools — the directional shadow map only handles the (near-vertical)
// key light.
struct TileOccluder {
    center: vec4<f32>,       // xyz = AABB center, w unused
    half_extents: vec4<f32>, // xyz = AABB half-extents, w unused
};
struct TileOccluders {
    count: vec4<u32>, // count.x = number of active occluders
    boxes: array<TileOccluder, 16>,
};
@group(1) @binding(1) var<uniform> occluders: TileOccluders;

// Jorge Jimenez's interleaved gradient noise. Cheap, low-discrepancy,
// stable in screen space — perfect for jittering shadow taps without
// the swimming you'd get from white noise. Returns a value in [0, 1).
fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(0.06711056 * p.x + 0.00583715 * p.y));
}

// Slab test: returns true if the segment from light_pos along `dir` (with
// `dir = frag_pos - light_pos`) is interrupted by the AABB strictly between
// the candle (t≈0) and the fragment (t≈1). The bias keeps tiles from
// self-shadowing on the lit side and candles from blocking their own bases.
fn segment_hits_aabb(
    light_pos: vec3<f32>,
    inv_dir: vec3<f32>,
    c: vec3<f32>,
    h: vec3<f32>,
) -> bool {
    let t1 = (c - h - light_pos) * inv_dir;
    let t2 = (c + h - light_pos) * inv_dir;
    let tmin = min(t1, t2);
    let tmax = max(t1, t2);
    let near_t = max(max(tmin.x, tmin.y), tmin.z);
    let far_t  = min(min(tmax.x, tmax.y), tmax.z);
    return far_t > near_t && near_t > 0.02 && near_t < 0.985;
}

// Soft analytic shadow from a candle modeled as a small disc area light.
// Casts `TAPS` rays from jittered points on the disc to `frag_pos`; each
// ray is tested against every tile AABB. Returns visibility in [0,1].
//
// The IGN seed is the framebuffer pixel coord — adjacent pixels get
// decorrelated rotations, which produces a stable, gradient-friendly
// penumbra without temporal animation.
fn candle_occlusion(light_pos: vec3<f32>, frag_pos: vec3<f32>, frag_xy: vec2<f32>) -> f32 {
    let n = occluders.count.x;
    if (n == 0u) {
        return 1.0;
    }
    // Disc radius in world units. The candles in this scene are about
    // 30–40 units across; treating the flame as a ~3-unit disc gives a
    // believable soft penumbra without making the shadow vanish entirely.
    let disc_radius = 3.0;
    // Per-fragment IGN-derived rotation so each pixel sees a different
    // tap orientation. Two octaves of IGN — one rotates the rosette,
    // the other phase-shifts the radii — keeps the pattern incoherent
    // across both axes for any tile alignment.
    let rot = ign(frag_xy) * 6.2831853;
    let rad_phase = ign(frag_xy + vec2<f32>(37.0, 91.0));
    let cos_r = cos(rot);
    let sin_r = sin(rot);

    // Four taps in a rotated rosette around the candle center, in the
    // table plane (XZ — world Y is the vertical axis here). The base
    // pattern is a 4-vertex square scaled by `rad_phase` so adjacent
    // pixels also vary in disc coverage, not just orientation.
    let r0 = disc_radius * mix(0.55, 1.0, rad_phase);
    var visible = 0.0;
    let taps = array<vec2<f32>, 4>(
        vec2<f32>( 1.0,  0.0),
        vec2<f32>( 0.0,  1.0),
        vec2<f32>(-1.0,  0.0),
        vec2<f32>( 0.0, -1.0),
    );
    for (var ti: i32 = 0; ti < 4; ti = ti + 1) {
        let base = taps[ti] * r0;
        let off = vec2<f32>(
            base.x * cos_r - base.y * sin_r,
            base.x * sin_r + base.y * cos_r,
        );
        let lp = light_pos + vec3<f32>(off.x, 0.0, off.y);
        let dir = frag_pos - lp;
        let safe = dir + vec3<f32>(1e-6, 1e-6, 1e-6);
        let inv = vec3<f32>(1.0) / safe;
        // Iterate the constant slot count and skip empty slots via the
        // sentinel half-extent. Looping on a uniform `n` was unreliable —
        // some naga versions appeared to fold the bound to 1, which is
        // why earlier builds only ever produced a single tile shadow.
        var blocked = false;
        for (var k: u32 = 0u; k < 16u; k = k + 1u) {
            let h = occluders.boxes[k].half_extents.xyz;
            if (h.x <= 0.0) {
                continue;
            }
            let c = occluders.boxes[k].center.xyz;
            if (segment_hits_aabb(lp, inv, c, h)) {
                blocked = true;
            }
        }
        if (!blocked) {
            visible = visible + 1.0;
        }
    }
    return visible * 0.25;
}

// ── Shadow sampling (group 2, shared frame-wide) ─────────────────────
struct ShadowGlobals {
    light_view_proj: mat4x4<f32>,
    // x = enabled (0/1), y = depth bias, z = texel size, w = unused
    params: vec4<f32>,
};
@group(2) @binding(0) var<uniform> shadow_globals: ShadowGlobals;
@group(2) @binding(1) var shadow_map: texture_depth_2d;
@group(2) @binding(2) var shadow_samp: sampler_comparison;

// ── SSR globals (group 3, shared frame-wide) ─────────────────────────
// The lacquered-floor branch marches reflection rays in screen space
// against the previous frame's composited colour + depth. The camera
// is fixed in this game, so a one-frame stale snapshot is effectively
// current. Disabled (params.x < 0.5) → analytic clearcoat only.
struct SsrGlobals {
    inv_view_proj: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
    // x = enabled (0/1), y = max_distance (world units),
    // z = stride (world units / step), w = max_steps
    params: vec4<f32>,
};
@group(3) @binding(0) var<uniform> ssr_globals: SsrGlobals;
@group(3) @binding(1) var ssr_scene_prev: texture_2d<f32>;
@group(3) @binding(2) var ssr_depth: texture_depth_2d;
@group(3) @binding(3) var ssr_samp: sampler;

// Project a world-space point to screen-space UV (0..1, top-left origin
// to match wgpu/webgpu texture sampling) plus its NDC z. Returns w<0 if
// the point is behind the camera.
fn ssr_project(world: vec3<f32>) -> vec4<f32> {
    let clip = ssr_globals.view_proj * vec4<f32>(world, 1.0);
    if (clip.w <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }
    let inv_w = 1.0 / clip.w;
    let ndc = clip.xyz * inv_w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, ndc.y * -0.5 + 0.5);
    return vec4<f32>(uv, ndc.z, 1.0);
}

// Sample the SSR depth texture at a UV (clamped) and unproject the
// resulting NDC point back to world space. Returns world Y of the
// nearest opaque surface at that screen position.
fn ssr_world_at(uv: vec2<f32>) -> vec3<f32> {
    let dim = vec2<f32>(textureDimensions(ssr_depth, 0));
    let px = vec2<i32>(clamp(uv * dim, vec2<f32>(0.0), dim - vec2<f32>(1.0)));
    let d = textureLoad(ssr_depth, px, 0);
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0, d);
    let world = ssr_globals.inv_view_proj * vec4<f32>(ndc, 1.0);
    return world.xyz / max(world.w, 1e-6);
}

// Returns visibility in [0,1]: 1 = fully lit by the key direction,
// 0 = fully occluded. Uses 3×3 PCF on the hardware comparison sampler.
fn sample_shadow_visibility(world_pos: vec3<f32>) -> f32 {
    if (shadow_globals.params.x < 0.5) {
        return 1.0;
    }
    let lp = shadow_globals.light_view_proj * vec4<f32>(world_pos, 1.0);
    let proj = lp.xyz / lp.w;
    let uv = vec2<f32>(proj.x * 0.5 + 0.5, proj.y * -0.5 + 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || proj.z < 0.0 || proj.z > 1.0) {
        return 1.0;
    }
    let bias = shadow_globals.params.y;
    let depth_ref = proj.z - bias;
    let texel = shadow_globals.params.z;
    var sum = 0.0;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let off = vec2<f32>(f32(dx), f32(dy)) * texel;
            sum = sum + textureSampleCompare(shadow_map, shadow_samp, uv + off, depth_ref);
        }
    }
    return sum / 9.0;
}

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
    // walnut linear-space albedo barely exceeds ~0.06 even on the
    // brightest fibers; the brown reads dark in any reasonable lighting.
    // We keep the dynamic range tight so that the candle pools lifting
    // the diffuse band don't push the table into a milky tan — the
    // brightness on the surface should come from the clearcoat lobe
    // (the glossy lacquer) rather than from raw albedo + light.
    // Strongly red-skewed to keep the surface reading as "brown" rather
    // than "tan" once the candle pools lift the values. The G/B channels
    // are pushed down so even at peak diffuse the chroma stays warm.
    let dark = mix(
        vec3<f32>(0.0120, 0.0034, 0.0010),
        vec3<f32>(0.0190, 0.0058, 0.0018),
        b.tone,
    );
    let mid = mix(
        vec3<f32>(0.0320, 0.0105, 0.0035),
        vec3<f32>(0.0440, 0.0150, 0.0055),
        b.tone,
    );
    let light = mix(
        vec3<f32>(0.0540, 0.0200, 0.0078),
        vec3<f32>(0.0680, 0.0260, 0.0100),
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
fn fs_main(
    in: VsOut,
    @builtin(front_facing) front_facing: bool,
) -> @location(0) vec4<f32> {
    let kind = mesh.material_params.x;
    let spec_strength = mesh.material_params.y;
    let spec_power = max(mesh.material_params.z, 1.0);

    // Sample the albedo texture unconditionally — material kind is uniform
    // across the draw, but hoisting the sample keeps naga's uniform-control-
    // flow analysis happy regardless of how it inlines the branch below.
    let tex_sample = textureSample(albedo_tex, albedo_samp, in.uv);
    let tex_rgb = tex_sample.rgb;
    // material_params.w doubles as a "this instance has an engraved decal"
    // flag. When >0.5 the texture is treated as a transparent overlay
    // (engraved label) composited *over* the procedural base material rather
    // than multiplied with it. The yaku/wood tablet pass sets this flag.
    let has_decal = mesh.material_params.w > 0.5;
    var albedo = mesh.base_color.rgb * tex_rgb;
    if (has_decal) {
        // Start from the flat base colour, ignore the texture multiply —
        // the procedural branch below may overwrite it for wood, and the
        // decal composite at the end will lay the engraved glyphs on top.
        albedo = mesh.base_color.rgb;
    }
    var wood_grain = 0.0;
    var wood_pore = 0.0;
    if (kind > 2.5) {
        // Lacquered wood: procedural grain overrides the (white) albedo tex.
        let w = wood_sample(in.local_pos);
        albedo = w.albedo;
        wood_grain = w.grain;
        wood_pore = w.pore;
    }
    if (has_decal) {
        albedo = mix(albedo, tex_rgb, tex_sample.a);
    }

    // Candle-only lighting: there is no ambient floor and no directional
    // key light. Every photon on the table comes from a wick PointLight
    // accumulated in the per-light loop below. `rgb` therefore starts at
    // pure black; fragments outside any candle pool stay dark.
    var n = normalize(in.world_n);
    if (!front_facing) {
        n = -n;
    }
    var rgb = vec3<f32>(0.0);

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
    // glassy layer rather than just "more highlight". The deep walnut
    // base is dark on its own — the lacquer coat is what makes the
    // table read as polished rather than matte chocolate.
    let coat_strength = select(0.0, 0.55, is_wood);
    let coat_power = 380.0;
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
        // Analytic AABB occlusion: tiles between the candle and this
        // fragment block the light's diffuse + specular contribution.
        // Floor at 0.18 so shadowed regions still pick up a soft bounce
        // term — fully black shadows look painted on.
        let cand_vis = mix(0.18, 1.0, candle_occlusion(lp, in.world_pos, in.clip_pos.xy));
        lit = lit + lc * intensity * atten * lambert * cand_vis;

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
            sss_acc = sss_acc + lc * intensity * atten * sss_band * sss_strength * sss_mask * cand_vis;
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
                + lc * intensity * atten * back * wax_thinness * wax_back_scale * cand_vis;
        }

        if (spec_strength > 0.001) {
            let h = normalize(l_dir + view_dir);
            let nh = max(dot(n, h), 0.0);
            // Two-lobe Blinn-Phong: a sharp grain highlight plus a
            // softer underlying sheen. Pores knock both lobes down so
            // open-grain voids stay matte.
            let sharp = pow(nh, spec_power);
            let soft  = pow(nh, max(spec_power * 0.5, 1.0)) * 0.10;
            var s = (sharp + soft) * spec_strength;
            if (is_wood) {
                s = s * mix(0.55, 1.15, wood_grain) * (1.0 - wood_pore * 0.85);
            }
            spec_acc = spec_acc + lc * intensity * atten * s * cand_vis;
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
            coat_acc = coat_acc + lc * intensity * atten * coat * cand_vis;
        }
    }

    // Compose: diffuse light multiplies albedo; sss adds a tinted glow;
    // base specular and clearcoat add on top. For wood we Fresnel-fade
    // the diffuse so energy flows into the coat at glancing angles.
    var diffuse_scale = 1.0;
    if (is_wood) {
        let f_view = coat_f0 + (1.0 - coat_f0) * pow(1.0 - ndv_view, 5.0);
        diffuse_scale = 1.0 - f_view * 0.6;
        // Soft-cap the wood's diffuse light gather. Without this, a
        // fragment near several candle pools accumulates `lit` past 5–6
        // and the resulting `albedo * lit` lifts the brown into milky
        // tan after sRGB encoding. Real lacquered walnut returns very
        // little diffuse — most of the candlelight bounces off the
        // glossy coat instead. Reinhard-style soft knee preserves the
        // ratio between channels so the chroma stays warm.
        lit = lit / (vec3<f32>(1.0) + lit * 0.55);
    }
    // No directional shadow gating now that there's no directional light;
    // analytic candle AABB occlusion (`cand_vis` above) already darkens
    // fragments shadowed from the wicks themselves.
    let lit_shadowed = lit;
    // Reinhard-knee the coat accumulator on wood: with many candles
    // contributing additive white highlights, the lacquer lobe was
    // piling up past 1.0 and milkifying the deep walnut. The knee
    // preserves the *shape* of the highlight (sharpest bits still
    // dominate) while clamping the broad lift that comes from many
    // distant pools all hitting the same fragment.
    var coat_final = coat_acc;
    var spec_final = spec_acc;

    // ── Screen-space reflections (lacquered wood only) ─────────────
    // The analytic clearcoat lobe above gives the table a glassy
    // highlight pinpoint, but a real polished tabletop also reflects
    // the *content* of the scene above it — most importantly, the
    // candle flames smear into vertical pillars pointing toward the
    // viewer. We march a reflection ray in world space against the
    // previous frame's depth + colour to capture that.
    if (is_wood && ssr_globals.params.x > 0.5) {
        let cam_pos_ssr = ssr_globals.view_pos.xyz;
        let v_ssr = normalize(cam_pos_ssr - in.world_pos);
        let r = reflect(-v_ssr, n);
        // Only march rays that point upward away from the table.
        // Reject grazing/down rays — they'd just hit the floor itself.
        if (r.y > 0.02) {
            let max_dist = ssr_globals.params.y;
            let stride = ssr_globals.params.z;
            let max_steps = i32(ssr_globals.params.w);
            // Start a hair above the surface to avoid self-intersection.
            let origin = in.world_pos + n * (stride * 0.5);
            var t_prev = 0.0;
            var t_hit = -1.0;
            var hit_uv = vec2<f32>(0.0, 0.0);
            for (var i: i32 = 1; i <= 64; i = i + 1) {
                if (i > max_steps) { break; }
                let t = f32(i) * stride;
                if (t > max_dist) { break; }
                let p = origin + r * t;
                let proj = ssr_project(p);
                if (proj.w < 0.0) { break; }
                let uv = proj.xy;
                if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
                    break;
                }
                let scene_world = ssr_world_at(uv);
                // The march and the depth texture both encode "how
                // far above the table". A hit happens when the ray
                // first dips at-or-below the recorded scene point's
                // world Y at the same screen pixel.
                if (p.y <= scene_world.y + stride * 0.6
                    && scene_world.y > in.world_pos.y + 0.1) {
                    // Binary refinement between the previous and
                    // current step for a sharper contact point.
                    var lo = t_prev;
                    var hi = t;
                    for (var k: i32 = 0; k < 4; k = k + 1) {
                        let mid = (lo + hi) * 0.5;
                        let pm = origin + r * mid;
                        let pmp = ssr_project(pm);
                        if (pmp.w < 0.0) { break; }
                        let uvm = pmp.xy;
                        let sw = ssr_world_at(uvm);
                        if (pm.y <= sw.y && sw.y > in.world_pos.y + 0.1) {
                            hi = mid;
                            hit_uv = uvm;
                        } else {
                            lo = mid;
                        }
                    }
                    t_hit = hi;
                    if (hit_uv.x == 0.0 && hit_uv.y == 0.0) {
                        hit_uv = uv;
                    }
                    break;
                }
                t_prev = t;
            }
            if (t_hit > 0.0) {
                // Schlick Fresnel against the actual view (not the
                // half-vector) — at glancing angles the reflection
                // should swell toward the lacquer's full intensity.
                let f0 = 0.04;
                let f_view = f0 + (1.0 - f0) * pow(1.0 - max(dot(n, v_ssr), 0.0), 5.0);
                // Fade against screen edges so the reflection doesn't
                // pop when it walks off the framebuffer.
                let edge = min(min(hit_uv.x, 1.0 - hit_uv.x), min(hit_uv.y, 1.0 - hit_uv.y));
                let edge_fade = smoothstep(0.0, 0.08, edge);
                // Distance fade so far reflections don't dominate.
                let dist_fade = 1.0 - clamp(t_hit / ssr_globals.params.y, 0.0, 1.0);
                let refl = textureSampleLevel(ssr_scene_prev, ssr_samp, hit_uv, 0.0).rgb;
                coat_final = coat_final + refl * (f_view * 0.55 * edge_fade * dist_fade);
            }
        }
    }

    if (is_wood) {
        coat_final = coat_final / (vec3<f32>(1.0) + coat_final * 0.7);
    }
    rgb = rgb
        + albedo * lit_shadowed * diffuse_scale
        + sss_acc * sss_tint
        + wax_back_acc * wax_tint
        + spec_final
        + coat_final;

    let inv_g = 1.0 / max(lights.extras.x, 0.01);
    let out_rgb = pow(rgb, vec3<f32>(inv_g));
    return vec4<f32>(out_rgb, mesh.base_color.a);
}
