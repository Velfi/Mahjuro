// Generic lit-mesh shader used by procedural scene props (candles, table).
//
// One pipeline, one shader; the `material_kind` field of MeshUniform selects
// the per-fragment look:
//
//   0.0 = plain     — lit base color
//   1.0 = wax       — pale beeswax with a high ambient floor (fake SSS)
//   2.0 = wick      — dark, no specular
//   3.0 = lacquered wood — procedural ring grain + Blinn-Phong specular
//   5.0 = metal     — tinted-Fresnel conductor (gold coins)
//   7.0 = talisman  — dielectric with heightmap normal perturbation (jade tablets)
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
    // extras.x = display gamma exponent.
    // extras.y = wall-clock time in seconds (used by the water material
    //            branch to scroll the surface and animate foam crests).
    // extras.z/.w reserved.
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

    // Single center ray from the candle to the fragment. This replaces
    // the previous 4-tap rotated rosette (4× cheaper) at the cost of
    // harder shadow edges. The IGN jitter offsets the ray slightly in
    // the table plane so adjacent pixels don't alias identically.
    let jitter_r = 1.5;
    let rot = ign(frag_xy) * 6.2831853;
    let off = vec2<f32>(cos(rot), sin(rot)) * jitter_r;
    let lp = light_pos + vec3<f32>(off.x, 0.0, off.y);
    let dir = frag_pos - lp;
    let safe = dir + vec3<f32>(1e-6, 1e-6, 1e-6);
    let inv = vec3<f32>(1.0) / safe;

    for (var k: u32 = 0u; k < n; k = k + 1u) {
        let c = occluders.boxes[k].center.xyz;
        let h = occluders.boxes[k].half_extents.xyz;
        if (segment_hits_aabb(lp, inv, c, h)) {
            return 0.0;
        }
    }
    return 1.0;
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
    var world_normal = normalize((mesh.model * vec4<f32>(n, 0.0)).xyz);
    var world_pos_out = (mesh.model * vec4<f32>(pos, 1.0)).xyz;

    // Lacquered wood (kind 3 — the table): evaluate the procedural wood
    // field in WORLD XZ coordinates rather than local mesh coordinates.
    // This decouples the grain density from the model's local-to-world
    // scale, so the table can be enlarged to "infinite plane" extents
    // without the rings stretching with it. The displacement is applied
    // directly in world Y (the table normal), and the normal is rebuilt
    // from a finite-difference of the world-space height field.
    //
    // Kind 3 only — kind 4 (LacqueredWoodFlat) shares the wood albedo
    // branch in the FS but skips this displacement because the table-
    // tuned 1.6-world-unit amplitude blows through the thin slab
    // thickness of small upright wood meshes (the score plaque), and
    // because that slab is not horizontal — its surface coords aren't
    // world XZ.
    if (mesh.material_params.x > 2.5 && mesh.material_params.x < 3.5) {
        let amp = 1.6;
        let eps_w = 1.0; // 1 world unit step for the gradient
        let wxz = world_pos_out.xz;
        let h_c = wood_height_world(wxz);
        let h_x = wood_height_world(wxz + vec2<f32>(eps_w, 0.0));
        let h_z = wood_height_world(wxz + vec2<f32>(0.0, eps_w));

        world_pos_out.y = world_pos_out.y + h_c * amp;

        let dh_x = (h_x - h_c) * amp;
        let dh_z = (h_z - h_c) * amp;
        // World-space tangent vectors along +X and +Z. Cross(t_z, t_x)
        // produces a normal with a positive Y component (table-up).
        let t_x = vec3<f32>(eps_w, dh_x, 0.0);
        let t_z = vec3<f32>(0.0,    dh_z, eps_w);
        world_normal = normalize(cross(t_z, t_x));
    }

    var o: VsOut;
    o.clip_pos = mesh.view_proj * vec4<f32>(world_pos_out, 1.0);
    o.world_pos = world_pos_out;
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

// World-space scale factors that map (world_x, world_z) into the wood
// field's pre-multiplied coordinate space. Tuned so that rings sweep
// across the table at the same density as the original 3.5×screen
// table on a 1280×720 layout, but now driven by world units instead
// of local mesh coords — so the table can be enlarged to "infinite
// plane" extents without stretching the grain.
const TABLE_WOOD_SCALE_X: f32 = 0.003125;
const TABLE_WOOD_SCALE_Z: f32 = 0.001587;

// Inner wood-basis evaluator: takes coordinates already in the field's
// pre-scaled space (the `p` the rest of this function uses). Both the
// local-space wrapper (for upright slabs like the score plaque) and
// the world-space wrapper (for the horizontal table) feed this.
fn wood_basis_p(p_in: vec2<f32>) -> WoodBasis {
    let p = p_in;

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

// Local-space wrapper used by upright wood slabs (the score plaque).
// Maps the slab's local [-0.5, 0.5] surface coords into the wood field's
// pre-scaled space using the original tuning constants.
fn wood_basis(local_xy: vec2<f32>) -> WoodBasis {
    return wood_basis_p(vec2<f32>(local_xy.x * 14.0, local_xy.y * 4.0));
}

// World-space wrapper used by the horizontal lacquered table. Tiles the
// wood field at a fixed world-space frequency regardless of how big the
// table model has been scaled.
fn wood_basis_world(world_xz: vec2<f32>) -> WoodBasis {
    return wood_basis_p(vec2<f32>(
        world_xz.x * TABLE_WOOD_SCALE_X,
        world_xz.y * TABLE_WOOD_SCALE_Z,
    ));
}

// Signed surface height from the wood basis. Positive on early-wood
// ridges, negative inside late-wood lines and pores. Returned in
// world units so the vertex shader can apply it directly to world Y.
fn wood_height(local_xy: vec2<f32>) -> f32 {
    let b = wood_basis(local_xy);
    // Early-wood bulges up; late-wood and pores recess. The mean is
    // close to zero so displacement does not visibly raise the slab.
    return b.early_wood * 0.55 - b.late_wood * 0.85 - b.pore * 1.6;
}

fn wood_height_world(world_xz: vec2<f32>) -> f32 {
    let b = wood_basis_world(world_xz);
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

fn wood_sample_world(world_xz: vec2<f32>) -> WoodSample {
    return wood_sample_basis(wood_basis_world(world_xz));
}

@fragment
fn fs_main(
    in: VsOut,
    @builtin(front_facing) front_facing: bool,
) -> @location(0) vec4<f32> {
    let kind = mesh.material_params.x;
    let spec_strength = mesh.material_params.y;
    let spec_power = max(mesh.material_params.z, 1.0);

    // ── Material kind booleans ──────────────────────────────────────
    // Derived once from the float `kind` and used everywhere below.
    // Keep these in sync with MaterialKind in lit_mesh.rs.
    //   0 = Plain, 1 = Wax, 2 = (unused), 3 = LacqueredWood,
    //   4 = LacqueredWoodFlat, 5 = Metal, 6 = Water, 7 = Talisman,
    //   8 = Foil
    let is_wax       = (kind > 0.5 && kind < 1.5);
    let is_wood      = (kind > 2.5 && kind < 4.5);
    let is_metal     = (kind > 4.5 && kind < 5.5);
    let is_water_mat = (kind > 5.5 && kind < 6.5);
    let is_talisman  = (kind > 6.5 && kind < 7.5);
    let is_foil      = (kind > 7.5 && kind < 8.5);

    // Sample the albedo texture unconditionally — material kind is uniform
    // across the draw, but hoisting the sample keeps naga's uniform-control-
    // flow analysis happy regardless of how it inlines the branch below.
    let tex_sample = textureSample(albedo_tex, albedo_samp, in.uv);
    let tex_rgb = tex_sample.rgb;
    // material_params.w doubles as a "this instance has an engraved decal"
    // flag for non-talisman materials. When >0.5 the texture is treated as
    // a transparent overlay (engraved label) composited *over* the procedural
    // base material rather than multiplied with it. For talismans (kind>6.5),
    // .w carries the sub-kind index (0=jade, 1=pearl, 2=gilded, 3=polychrome).
    let has_decal = mesh.material_params.w > 0.5 && !is_talisman && !is_foil;
    var albedo = mesh.base_color.rgb * tex_rgb;
    if (has_decal) {
        // Start from the flat base colour, ignore the texture multiply —
        // the procedural branch below may overwrite it for wood, and the
        // decal composite at the end will lay the engraved glyphs on top.
        albedo = mesh.base_color.rgb;
    }
    if (is_metal) {
        // Metal: the bound texture is a heightmap, not an albedo. Use the
        // raw base colour and let the height contribute later via the
        // normal-perturbation block below.
        albedo = mesh.base_color.rgb;
    }
    if (is_foil) {
        // Foil: the bound texture IS the full-colour pack art; multiply it
        // with the base colour (typically white) so the art shows through.
        // The metallic foil sheen is layered on top in the per-light loop.
        albedo = mesh.base_color.rgb * tex_rgb;
    }
    var wood_grain = 0.0;
    var wood_pore = 0.0;
    if (is_wood) {
        // Lacquered wood: procedural grain overrides the (white) albedo tex.
        // Kind 3 (the horizontal table) samples in world XZ so the grain
        // tiles at a fixed world-space frequency regardless of the model's
        // scale — this is what lets the table extend to the horizon as an
        // "infinite plane" without stretching the rings. Kind 4 (the
        // upright score plaque) keeps using local surface coords because
        // its slab face isn't aligned with world XZ.
        var w: WoodSample;
        if (kind < 3.5) {
            w = wood_sample_world(in.world_pos.xz);
        } else {
            w = wood_sample(in.local_pos);
        }
        albedo = w.albedo;
        wood_grain = w.grain;
        wood_pore = w.pore;
    }
    // ── Carved & gold-leafed decal compositing ───────────────────────
    // Where the decal has alpha, the text is treated as gold paint sitting
    // inside a shallow carved channel. We perturb the surface normal at
    // alpha edges so the channel walls catch and shadow the candlelight,
    // darken the groove floor, then lay the gold colour on top. A metallic
    // specular boost is stored for the lighting loop below.
    var decal_metallic = 0.0;
    var decal_f0 = vec3<f32>(0.0);
    // Alpha-gradient samples for normal perturbation — hoisted so they're
    // visible to both the albedo-composite block and the normal block.
    var decal_dhdu = 0.0;
    var decal_dhdv = 0.0;
    if (has_decal) {
        let da = tex_sample.a;

        // Finite-difference normal perturbation from the decal alpha field.
        // The alpha acts as a heightmap: 0 = flush wood surface, 1 = bottom
        // of the carved channel. Edges where alpha changes rapidly become
        // the groove walls, catching light from one side and shadowing the
        // other — exactly how CNC-routed or chisel-cut lettering reads
        // under raking light.
        let dim_d = vec2<f32>(textureDimensions(albedo_tex, 0));
        let tx = vec2<f32>(1.0 / max(dim_d.x, 1.0), 1.0 / max(dim_d.y, 1.0));
        let a_l = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>(-tx.x, 0.0), 0.0).a;
        let a_r = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>( tx.x, 0.0), 0.0).a;
        let a_d = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>(0.0, -tx.y), 0.0).a;
        let a_u = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>(0.0,  tx.y), 0.0).a;
        let carve_bump = 3.5;
        decal_dhdu = (a_r - a_l) * carve_bump;
        decal_dhdv = (a_u - a_d) * carve_bump;

        // Groove-floor darkening: inside the carved channel the wood is
        // slightly shadowed by the groove walls — this reads as the dark
        // recess before the gold paint is laid in. Smoothstep avoids
        // touching fragments outside the glyph boundary.
        let groove = smoothstep(0.05, 0.35, da);
        albedo = mix(albedo, albedo * 0.55, groove);

        // Composite the gold paint *inside* the groove. Full alpha paints
        // the gold body; semi-transparent edges blend into the darkened
        // groove for a natural transition.
        albedo = mix(albedo, tex_rgb, da);

        // Gold leaf is a conductor — its specular response is tinted by the
        // paint's own colour (no white plastic highlight). Store the F0 and
        // a metallic flag so the lighting loop picks up a gold Fresnel lobe
        // on these fragments only.
        decal_metallic = smoothstep(0.15, 0.50, da);
        decal_f0 = tex_rgb * vec3<f32>(1.05, 0.92, 0.65);
    }

    // Candle-only lighting: there is no ambient floor and no directional
    // key light. Every photon on the table comes from a wick PointLight
    // accumulated in the per-light loop below. `rgb` therefore starts at
    // pure black; fragments outside any candle pool stay dark.
    var n = normalize(in.world_n);
    if (!front_facing) {
        n = -n;
    }

    // Apply carved-groove normal perturbation from the decal alpha gradient.
    // The derivatives were computed above; here we push the flat-face normal
    // sideways so the groove walls catch the candlelight. The perturbation
    // is in tangent space (UV-aligned), re-oriented into world space via the
    // model matrix's upper 3×3 (plaques/tablets only translate + uniformly
    // scale, so this is exact). Fades to zero outside the glyph boundary.
    if (has_decal) {
        let edge_mag = abs(decal_dhdu) + abs(decal_dhdv);
        if (edge_mag > 0.001) {
            let perturbed_local = normalize(vec3<f32>(-decal_dhdu, 1.0, -decal_dhdv));
            let perturbed_world = normalize((mesh.model * vec4<f32>(perturbed_local, 0.0)).xyz);
            // Blend based on edge strength so flat-alpha interiors keep
            // their original normal (the paint surface inside the groove
            // is still flat — only the walls tilt).
            let blend_edge = clamp(edge_mag * 1.5, 0.0, 1.0);
            n = normalize(mix(n, perturbed_world, blend_edge));
        }
    }

    // ── Discard-river material ─────────────────────────────────────────
    // Single mesh draws both the stone trough and the water surface; the
    // per-vertex `uv.y` channel switches between them. Stone fragments
    // (`uv.y < 0.5`) get a dark, slightly speckled rock shade; water
    // fragments (`uv.y > 0.5`) get scrolling normal perturbation, an
    // indigo→teal gradient with foam crests, and a Blinn-Phong specular
    // pool from the candle lights. The branch returns immediately so we
    // skip the wood/wax/metal lighting path entirely.
    if (is_water_mat) {
        let time = lights.extras.y;
        let is_water = in.uv.y > 0.5;
        var water_n = n;
        var water_albedo: vec3<f32>;
        var water_spec_strength = 0.0;
        var water_spec_power = 1.0;
        if (is_water) {
            // Use local-space XZ as a stable surface coordinate. Two
            // scrolling noise layers at different speeds + scales perturb
            // the normal, the sum acts as a pseudo-foam mask.
            let p = vec2<f32>(in.local_pos.x, in.local_pos.z) * 12.0;
            let f1 = fbm2(p + vec2<f32>(time * 0.45, time * 0.10));
            let f2 = fbm2(p * 1.7 + vec2<f32>(time * 0.18, -time * 0.30));
            let crest = smoothstep(0.55, 0.85, f1 * 0.6 + f2 * 0.6);
            // Cheap finite-difference normal from the noise sum.
            let eps = 0.6;
            let h_c = f1 + f2;
            let h_x = fbm2(p + vec2<f32>(eps + time * 0.45, time * 0.10))
                    + fbm2(p * 1.7 + vec2<f32>(eps * 1.7 + time * 0.18, -time * 0.30));
            let h_z = fbm2(p + vec2<f32>(time * 0.45, eps + time * 0.10))
                    + fbm2(p * 1.7 + vec2<f32>(time * 0.18, eps * 1.7 - time * 0.30));
            let bump = 0.55;
            let dhdu = (h_x - h_c) * bump;
            let dhdv = (h_z - h_c) * bump;
            // Surface tangent basis is local +X / +Z (the water plane is
            // a flat horizontal quad in local space). Build a perturbed
            // normal pointing mostly +Y with the gradient subtracted.
            let n_w = normalize(vec3<f32>(-dhdu, 1.0, -dhdv));
            // Re-orient the perturbed normal into world space using the
            // model matrix's upper 3x3 (the trough only translates +
            // uniformly scales, so passing through is a fine
            // approximation).
            water_n = normalize((mesh.model * vec4<f32>(n_w, 0.0)).xyz);
            // Indigo deep water → teal lift in shallow noise valleys, plus
            // bright foam where crests pile up. The Midnight Gold palette
            // hint sits on the cool indigo side; the foam pops it.
            let deep = vec3<f32>(0.018, 0.030, 0.075);
            let mid  = vec3<f32>(0.045, 0.085, 0.155);
            water_albedo = mix(deep, mid, clamp(f2 * 1.2, 0.0, 1.0))
                         + vec3<f32>(crest) * vec3<f32>(0.55, 0.62, 0.78);
            water_spec_strength = 1.4;
            water_spec_power = 220.0;
        } else {
            // Stone trough: dark slate with a tiny per-pixel speckle so
            // the walls don't read as plastic-flat. Spec is low — the
            // stone should not compete with the water highlight.
            let p = vec2<f32>(in.local_pos.x, in.local_pos.z) * 22.0;
            let speckle = vnoise2(p) * 0.08;
            water_albedo = vec3<f32>(0.038, 0.044, 0.052) + vec3<f32>(speckle);
            water_spec_strength = 0.10;
            water_spec_power = 24.0;
        }

        var lit_water = vec3<f32>(0.0);
        var spec_water = vec3<f32>(0.0);
        let cam_pos_w = vec3<f32>(0.0, 0.0, 4000.0);
        let view_dir_w = normalize(cam_pos_w - in.world_pos);
        let count_w = lights.count.x;
        for (var i: u32 = 0u; i < count_w; i = i + 1u) {
            let lp = lights.lights[i].pos.xyz;
            let radius = lights.lights[i].pos.w;
            let lc = lights.lights[i].color.rgb;
            let intensity = lights.lights[i].color.a;
            let to_light = lp - in.world_pos;
            let dist = length(to_light);
            let t = clamp(1.0 - dist / max(radius, 1.0), 0.0, 1.0);
            let atten = t * t;
            let l_dir = to_light / max(dist, 0.0001);
            let nl = max(dot(water_n, l_dir), 0.0);
            // Lift the ambient floor on water so the trough silhouette
            // reads even when no candle pool is overhead.
            let lambert = select(0.30, 0.45, is_water) + 0.55 * nl;
            lit_water = lit_water + lc * intensity * atten * lambert;

            if (water_spec_strength > 0.001) {
                let h = normalize(l_dir + view_dir_w);
                let nh = max(dot(water_n, h), 0.0);
                let s = pow(nh, water_spec_power) * water_spec_strength;
                spec_water = spec_water + lc * intensity * atten * s;
            }
        }
        // A constant cool ambient term so the river is visible across the
        // whole table even between candle pools — the surface is the
        // discard target, players should never lose track of where it is.
        let ambient = select(
            vec3<f32>(0.020, 0.024, 0.034),
            vec3<f32>(0.032, 0.046, 0.090),
            is_water,
        );
        var rgb_w = water_albedo * (lit_water + ambient) + spec_water;
        let inv_g_w = 1.0 / max(lights.extras.x, 0.01);
        let out_w = pow(rgb_w, vec3<f32>(inv_g_w));
        return vec4<f32>(out_w, mesh.base_color.a);
    }

    // ── Metal heightmap perturbation ─────────────────────────────────────
    // For metal kind we treat the bound texture as a grayscale heightfield
    // (the engraved Chinese cash-coin face). Central differences along U
    // and V give an approximate gradient; we lift it into world space using
    // the coin's flat-face tangent basis (UV maps to local XZ on the top
    // and bottom of the coin) and rotate the normal toward the gradient.
    // Only flat-ish faces are perturbed — the rim's UVs wrap once around
    // the cylinder and the gradient there would be meaningless.
    if (is_metal) {
        let face_flat = abs(n.y);
        if (face_flat > 0.6) {
            let dim = vec2<f32>(textureDimensions(albedo_tex, 0));
            let texel = vec2<f32>(1.0 / max(dim.x, 1.0), 1.0 / max(dim.y, 1.0));
            let h_l = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>(-texel.x, 0.0), 0.0).r;
            let h_r = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>( texel.x, 0.0), 0.0).r;
            let h_d = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>(0.0, -texel.y), 0.0).r;
            let h_u = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>(0.0,  texel.y), 0.0).r;
            // Bump strength — small enough that the engraving reads as
            // shallow strike depth, not a relief sculpture.
            let bump = 2.4;
            let dhdu = (h_r - h_l) * bump;
            let dhdv = (h_u - h_d) * bump;
            // Coin top/bottom tangent basis: tangent = +X, bitangent = +Z,
            // surface normal = ±Y. The heightmap perturbation pushes the
            // normal *away* from the gradient direction.
            let sgn = sign(n.y);
            let perturbed = normalize(vec3<f32>(-dhdu, sgn, -dhdv));
            // Re-orient the perturbed normal so it sits on the actual face
            // (pointing the same way the original normal does).
            var n_face = vec3<f32>(perturbed.x, perturbed.y * sgn, perturbed.z);
            n_face = normalize(n_face);
            // Fade between the original normal (rim and bevel) and the
            // perturbed normal (flat faces) by `face_flat` so the seam
            // between disc and rim doesn't pop.
            let blend = smoothstep(0.6, 0.95, face_flat);
            n = normalize(mix(n, n_face, blend));
        }
    }

    // ── Talisman heightmap perturbation ──────────────────────────────────
    // Same finite-difference approach as metal, but uses screen-space
    // derivatives to build the tangent frame so it works regardless of
    // the tablet's orientation (upright on the wall or laid flat in the
    // tray). Only the flat front/back faces are perturbed — detected via
    // local_pos.z proximity to the half-thickness (±0.09).
    if (is_talisman) {
        let face_flat = abs(abs(in.local_pos.z) - 0.09);
        if (face_flat < 0.02) {
            let dim = vec2<f32>(textureDimensions(albedo_tex, 0));
            let texel = vec2<f32>(1.0 / max(dim.x, 1.0), 1.0 / max(dim.y, 1.0));
            let h_l = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>(-texel.x, 0.0), 0.0).r;
            let h_r = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>( texel.x, 0.0), 0.0).r;
            let h_d = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>(0.0, -texel.y), 0.0).r;
            let h_u = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>(0.0,  texel.y), 0.0).r;
            // Bump strength tuned for shallow carved relief on jade.
            let bump = 3.0;
            let dhdu = (h_r - h_l) * bump;
            let dhdv = (h_u - h_d) * bump;
            // Screen-space derivative tangent basis: works for any
            // orientation without needing explicit tangent attributes.
            let dp_dx = dpdx(in.world_pos);
            let dp_dy = dpdy(in.world_pos);
            let tangent = normalize(dp_dx);
            let bitangent = normalize(cross(n, tangent));
            let perturbed = normalize(n - tangent * dhdu - bitangent * dhdv);
            n = perturbed;
        }
    }
    var rgb = vec3<f32>(0.0);

    // ── Point-light pass ─────────────────────────────────────────────────
    // Distance attenuation + Lambertian, with optional Blinn-Phong specular,
    // optional fake subsurface wrap (wood/wax), and an optional dielectric
    // clearcoat lobe (lacquered wood).
    var lit = vec3<f32>(0.0);          // diffuse-band light (multiplied by albedo)
    var sss_acc = vec3<f32>(0.0);      // wrap-diffuse subsurface (front-lit)
    var back_acc = vec3<f32>(0.0); // wax back-transmission (Penner-style)
    var spec_acc = vec3<f32>(0.0);     // base specular accumulator
    var coat_acc = vec3<f32>(0.0);     // clearcoat accumulator (white, untinted)
    var sheen_acc = vec3<f32>(0.0);    // talisman sheen accumulator

    let cam_pos = vec3<f32>(0.0, 0.0, 4000.0); // approximate; only direction matters
    let view_dir = normalize(cam_pos - in.world_pos);
    let ndv_view = clamp(dot(n, view_dir), 0.0, 1.0);


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
    } else if (is_talisman) {
        // Talisman SSS: per-kind tint so each material scatters light
        // through its surface with the right hue. The sub-kind index
        // lives in material_params.w (0=jade, 1=pearl, 2=gilded, 3=poly).
        let talisman_sub = mesh.material_params.w;
        wrap = 0.45;
        sss_strength = 0.40;
        if (talisman_sub < 0.5) {
            // Jade: warm green transmission.
            sss_tint = vec3<f32>(0.55, 0.92, 0.60);
        } else if (talisman_sub < 1.5) {
            // Pearl: cool pink-white glow.
            sss_tint = vec3<f32>(0.90, 0.85, 0.95);
            sss_strength = 0.30;
        } else if (talisman_sub < 2.5) {
            // Gilded: warm amber transmission.
            sss_tint = vec3<f32>(0.95, 0.80, 0.40);
            sss_strength = 0.20;
        } else {
            // Polychrome: cool violet-pink glow.
            sss_tint = vec3<f32>(0.75, 0.55, 0.95);
            sss_strength = 0.30;
        }
    }

    // ── Back-transmission (Penner SSS) tunables ──────────────────────
    // The classic cheap translucency model: the effective light
    // direction is bent toward the surface normal by `distortion`,
    // then we measure how much of that bent light vector points
    // *toward* the camera (i.e. through the back of the object).
    // The result is heavily concentrated on the silhouette of an
    // object that is back-lit, exactly where real wax glows.
    // Jade uses the same model — nephrite/jadeite tablets are thin
    // enough that back-lit edges glow with a warm green.
    let back_distortion = 0.45;
    let back_power = select(4.0, 6.0, is_talisman); // jade is denser, tighter lobe
    var back_scale = 0.0;
    var back_tint = vec3<f32>(0.0);
    if (is_wax) {
        back_scale = 1.4;
        back_tint = vec3<f32>(1.00, 0.72, 0.32);
    } else if (is_talisman) {
        let talisman_sub_bt = mesh.material_params.w;
        back_scale = 0.9;
        if (talisman_sub_bt < 0.5) {
            back_tint = vec3<f32>(0.50, 0.88, 0.55); // jade green
        } else if (talisman_sub_bt < 1.5) {
            back_tint = vec3<f32>(0.88, 0.82, 0.92); // pearl pink-white
        } else if (talisman_sub_bt < 2.5) {
            back_tint = vec3<f32>(0.92, 0.75, 0.35); // gilded amber
            back_scale = 0.5; // metal is less translucent
        } else {
            back_tint = vec3<f32>(0.70, 0.50, 0.90); // polychrome violet
        }
    }

    // Thickness proxy from local geometry. For wax: high near the top
    // of the votive and at silhouette edges. For jade: the tablet is a
    // thin flat slab, so thinness is driven by the silhouette edge
    // angle (where the camera sees through a thin cross-section of
    // stone) and slightly by the rim faces (which are geometrically
    // thinner than the flat faces).
    var back_thinness = 0.0;
    if (is_wax) {
        let height01 = clamp(in.local_pos.y / 0.56, 0.0, 1.0);
        let edge = 1.0 - max(dot(n, view_dir), 0.0);
        let top_thin = smoothstep(0.30, 0.92, height01);
        let edge_thin = pow(edge, 2.0);
        back_thinness = clamp(top_thin * 0.85 + edge_thin * 0.65, 0.0, 1.6);
    } else if (is_talisman) {
        // Silhouette edge: camera sees through a thin slice of jade.
        let edge = 1.0 - max(dot(n, view_dir), 0.0);
        let edge_thin = pow(edge, 2.5);
        // Rim faces (local_pos.z near 0, between front/back) are
        // geometrically thinner cross-sections of the tablet.
        let rim_thin = 1.0 - smoothstep(0.0, 0.06, abs(in.local_pos.z));
        back_thinness = clamp(edge_thin * 0.8 + rim_thin * 0.4, 0.0, 1.2);
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
        // Skip lights whose attenuation has fallen to zero — no point
        // computing occlusion, diffuse, specular, or SSS for a light
        // that contributes nothing.
        if (atten < 0.001) {
            continue;
        }
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
        if (back_scale > 0.001) {
            let lt = normalize(l_dir + n * back_distortion);
            let back = pow(max(dot(view_dir, -lt), 0.0), back_power);
            back_acc = back_acc
                + lc * intensity * atten * back * back_thinness * back_scale * cand_vis;
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
            if (is_metal) {
                // Conductor: Schlick Fresnel against the half-vector with
                // F0 = base colour. The reflected light then takes on the
                // metal's tint (no white "plastic" highlight) and swells
                // toward full reflectivity at glancing angles.
                let vdh = max(dot(view_dir, h), 0.0);
                let f0 = mesh.base_color.rgb;
                let f_metal = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vdh, 5.0);
                spec_acc = spec_acc + lc * intensity * atten * s * cand_vis * f_metal;
            } else {
                spec_acc = spec_acc + lc * intensity * atten * s * cand_vis;
            }
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

        // ── Talisman sheen lobes ──────────────────────────────────────
        // Per-kind view-dependent sheen layered on top of the base
        // dielectric specular. Broad lobes so the effect is visible
        // across the whole tablet, not just at perfect mirror angles.
        //   Jade (0)    — subtle waxy green luster
        //   Pearl (1)   — soft pearlescent pink/blue color shift
        //   Gilded (2)  — warm metallic gold Fresnel
        //   Polychrome (3) — holographic thin-film rainbow
        if (is_talisman) {
            let h = normalize(l_dir + view_dir);
            let nh = max(dot(n, h), 0.0);
            let vdh = max(dot(view_dir, h), 0.0);
            let ndv = max(dot(n, view_dir), 0.0);
            let tsk = mesh.material_params.w;
            // Broad wrap term: the diffuse half-vector alignment catches
            // light from a wide arc, not just the mirror direction.
            let broad = max(dot(n, l_dir), 0.0);

            if (tsk < 0.5) {
                // Jade: waxy vitreous luster. A broad green-tinted
                // sheen that strengthens at grazing angles.
                let fresnel = 0.08 + 0.30 * pow(1.0 - ndv, 2.5);
                let lobe = pow(nh, 12.0) * 0.6 + broad * 0.15;
                let tint = vec3<f32>(0.55, 0.95, 0.65);
                sheen_acc = sheen_acc + lc * intensity * atten * cand_vis * lobe * fresnel * tint;
            } else if (tsk < 1.5) {
                // Pearl: pearlescent nacre with pink-to-blue shift
                // driven by viewing angle.
                let fresnel = 0.10 + 0.50 * pow(1.0 - ndv, 3.0);
                let phase = ndv * 3.14159;
                let pearl_tint = vec3<f32>(
                    0.95 + 0.05 * cos(phase),
                    0.85 + 0.10 * cos(phase + 1.2),
                    0.90 + 0.10 * cos(phase + 2.8)
                );
                let lobe = pow(nh, 16.0) * 0.7 + broad * 0.20;
                sheen_acc = sheen_acc + lc * intensity * atten * cand_vis * lobe * fresnel * pearl_tint;
            } else if (tsk < 2.5) {
                // Gilded: metallic gold conductor — Schlick Fresnel
                // tinted by the gold base so highlights read warm.
                let f0 = vec3<f32>(0.95, 0.75, 0.30);
                let f_gold = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vdh, 5.0);
                let lobe = pow(nh, 24.0) * 0.8 + broad * 0.15;
                sheen_acc = sheen_acc + lc * intensity * atten * cand_vis * lobe * f_gold;
            } else {
                // Polychrome: holographic thin-film iridescence —
                // rainbow hue driven by the normal-to-half angle
                // so the spectrum shifts as the light sweeps across.
                let film_angle = dot(n, h);
                let theta = film_angle * 6.0 + ndv * 2.0;
                let holo_r = 0.5 + 0.5 * cos(theta);
                let holo_g = 0.5 + 0.5 * cos(theta + 2.094);
                let holo_b = 0.5 + 0.5 * cos(theta + 4.189);
                let holo_tint = vec3<f32>(holo_r, holo_g, holo_b);
                let fresnel = 0.12 + 0.60 * pow(1.0 - ndv, 2.5);
                let lobe = pow(nh, 10.0) * 0.8 + broad * 0.25;
                sheen_acc = sheen_acc + lc * intensity * atten * cand_vis * lobe * fresnel * holo_tint;
            }
        }

        // ── Foil sheen (metallic wrapping with iridescence) ──────────
        // Combines a tight conductor specular (the foil's mirror) with a
        // broad thin-film rainbow (the holographic overprint). The albedo
        // carries the actual pack art; the sheen sits on top.
        if (is_foil) {
            let h = normalize(l_dir + view_dir);
            let nh = max(dot(n, h), 0.0);
            let vdh = max(dot(view_dir, h), 0.0);
            let ndv = max(dot(n, view_dir), 0.0);
            let broad = max(dot(n, l_dir), 0.0);
            // Silver conductor Fresnel — real foil wrappers have a neutral
            // metallic sheen regardless of the printed art underneath.
            let f0 = vec3<f32>(0.75) + albedo * 0.1;
            let f_foil = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vdh, 5.0);
            let mirror_lobe = pow(nh, 48.0) * 1.6;
            spec_acc = spec_acc + lc * intensity * atten * cand_vis * mirror_lobe * f_foil;
            // Holographic thin-film iridescence — subtle rainbow shimmer
            // layered over the dominant mirror specular. Kept gentle so the
            // foil reads as shiny metallic wrapping, not a green-tinted mess.
            let film_angle = dot(n, h);
            let theta = film_angle * 8.0 + ndv * 3.0;
            let holo_r = 0.5 + 0.5 * cos(theta);
            let holo_g = 0.5 + 0.5 * cos(theta + 2.094);
            let holo_b = 0.5 + 0.5 * cos(theta + 4.189);
            let holo_tint = vec3<f32>(holo_r, holo_g, holo_b);
            let fresnel = 0.05 + 0.35 * pow(1.0 - ndv, 3.0);
            let holo_lobe = pow(nh, 12.0) * 0.35;
            sheen_acc = sheen_acc + lc * intensity * atten * cand_vis * holo_lobe * fresnel * holo_tint;
        }

        // Gold-leaf specular lobe on carved decal text. Conductor Fresnel
        // tinted by the gold paint's own colour so the highlight reads as
        // metallic rather than plastic. The lobe is moderately sharp
        // (power 180) — real gold leaf is smoother than raw wood but
        // rougher than mirror-polished metal, and a too-tight highlight
        // vanishes at typical viewing distances. Only fires on fragments
        // where decal_metallic > 0.
        if (decal_metallic > 0.001) {
            let h = normalize(l_dir + view_dir);
            let nh = max(dot(n, h), 0.0);
            let vdh = max(dot(view_dir, h), 0.0);
            let f_gold = decal_f0 + (vec3<f32>(1.0) - decal_f0) * pow(1.0 - vdh, 5.0);
            let gold_spec = pow(nh, 180.0) * 1.2 * decal_metallic;
            spec_acc = spec_acc + lc * intensity * atten * gold_spec * cand_vis * f_gold;
        }
    }

    // Compose: diffuse light multiplies albedo; sss adds a tinted glow;
    // base specular and clearcoat add on top. For wood we Fresnel-fade
    // the diffuse so energy flows into the coat at glancing angles.
    var diffuse_scale = 1.0;
    if (is_metal) {
        // Conductors do not diffusely scatter light — almost all of the
        // response is in the tinted Fresnel spec lobe above. Leave a
        // sliver of diffuse so unlit-side coins don't read as cutouts.
        diffuse_scale = 0.08;
    }
    if (is_foil) {
        // Semi-metallic foil: more diffuse than a pure conductor (the
        // printed art needs to read) but less than a dielectric. The
        // specular + sheen carry the foil's shine.
        diffuse_scale = 0.45;
    }
    // Gold-painted fragments inside carved decals are conductors: almost
    // all energy goes into the tinted Fresnel spec lobe, very little
    // diffuse. Lerp the diffuse scale down so gold reads as metallic.
    if (decal_metallic > 0.001) {
        diffuse_scale = mix(diffuse_scale, 0.12, decal_metallic);
    }
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
    // ── Talisman Fresnel albedo tint ───────────────────────────────
    // View-dependent color shift baked into the surface albedo so it
    // reads as a material property (always visible), not just a specular
    // highlight that depends on perfect light alignment.
    if (is_talisman) {
        let tsk_comp = mesh.material_params.w;
        let edge = 1.0 - ndv_view;
        if (tsk_comp < 0.5) {
            // Jade: edges brighten toward a cooler, lighter green.
            let rim = pow(edge, 2.0) * 0.25;
            albedo = mix(albedo, vec3<f32>(0.6, 1.0, 0.75), rim);
        } else if (tsk_comp < 1.5) {
            // Pearl: edges shift toward soft pink/blue iridescence.
            let rim = pow(edge, 1.8) * 0.35;
            let phase = ndv_view * 3.14159;
            let pearl_edge = vec3<f32>(
                0.98 + 0.02 * cos(phase),
                0.88 + 0.06 * cos(phase + 1.5),
                0.95 + 0.05 * cos(phase + 3.0)
            );
            albedo = mix(albedo, pearl_edge, rim);
        } else if (tsk_comp < 2.5) {
            // Gilded: edges brighten toward white-gold (metallic sheen).
            let rim = pow(edge, 2.0) * 0.30;
            albedo = mix(albedo, vec3<f32>(1.0, 0.92, 0.65), rim);
        } else {
            // Polychrome: rainbow Fresnel shifts the surface hue at
            // edges, giving a holographic color-change visible from
            // any lighting angle.
            let rim = pow(edge, 1.5) * 0.45;
            let theta = ndv_view * 8.0;
            let holo = vec3<f32>(
                0.5 + 0.5 * cos(theta),
                0.5 + 0.5 * cos(theta + 2.094),
                0.5 + 0.5 * cos(theta + 4.189)
            );
            albedo = mix(albedo, holo, rim);
        }
    }

    // Foil Fresnel edge tint: subtle rainbow color-shift at grazing
    // angles so the wrapper catches a hint of iridescence in ambient.
    if (is_foil) {
        let edge = 1.0 - ndv_view;
        let rim = pow(edge, 2.5) * 0.15;
        let theta = ndv_view * 6.0 + in.uv.x * 2.0;
        let holo = vec3<f32>(
            0.5 + 0.5 * cos(theta),
            0.5 + 0.5 * cos(theta + 2.094),
            0.5 + 0.5 * cos(theta + 4.189)
        );
        albedo = mix(albedo, holo, rim);
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
        + back_acc * back_tint
        + spec_final
        + coat_final
        + sheen_acc;

    let inv_g = 1.0 / max(lights.extras.x, 0.01);
    let out_rgb = pow(rgb, vec3<f32>(inv_g));
    return vec4<f32>(out_rgb, mesh.base_color.a);
}
