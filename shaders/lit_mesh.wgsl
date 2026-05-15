// Generic lit-mesh shader used by procedural scene props (candles, table).
//
// One pipeline, one shader; the `material_kind` field of MeshUniform selects
// the per-fragment look:
//
//   0.0  = plain      — lit base color
//   1.0  = wax        — pale beeswax with a high ambient floor (fake SSS)
//   2.0  = wick       — dark, no specular
//   3.0  = lacquered wood — procedural ring grain + Blinn-Phong specular
//   5.0  = metal      — tinted-Fresnel conductor (gold coins)
//   8.0  = foil       — metallic wrapping with thin-film iridescence
//   9.0  = glass      — faux translucent crystal / glazed glass
//   10.0 = enamel     — hard-enamel pin look
//   11.0 = jade       — waxy carved jade with broad green sheen
//   12.0 = moonstone — transparent feldspar with blue adularescence
//   13.0 = pearl      — pearlescent nacre, base_color tints the sheen
//   14.0 = gold nugget — pitted metallic gold (procedural noise normals)
//   15.0 = polychrome  — holographic thin-film rainbow
//   20.0 = emissive    — additive self-illumination (strength in specular_strength)
//
// All material variants share the candle/spot point-light loop from the tile
// shader so the new geometry catches the same warm pools as the hand tiles.

struct MeshUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    base_color: vec4<f32>,
    // x = material_kind, y = specular_strength (emissive scale for kind 20),
    // z = specular_power, w = decal / talisman slot (see lit_mesh.rs)
    material_params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> mesh: MeshUniform;
@group(0) @binding(1) var albedo_tex: texture_2d<f32>;
@group(0) @binding(2) var albedo_samp: sampler;
/// Linear grayscale relief for relic enamel (separate from color albedo).
@group(0) @binding(3) var relief_tex: texture_2d<f32>;

struct PointLight {
    pos: vec4<f32>,   // xyz = world position, w = radius or inverse-square range
    color: vec4<f32>, // rgb = color, a = intensity
    params: vec4<f32>, // x = 0 smooth, 1 inverse-square
};

struct PointLights {
    count: vec4<u32>,
    // extras.x = display gamma exponent.
    // extras.y = wall-clock time in seconds (used by the water material
    //            branch to scroll the surface and animate foam crests).
    // extras.z = candle flame height (lightbake).
    // extras.w = scales inverse-square intensities from embedded GLB punctual.
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

// `aces_fitted` — see `scene_hdr_tonemap.wgsl` (prepended at pipeline creation).

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
    let lp = light_pos + vec3<f32>(off.x, off.y, 0.0);
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

// Spotlights — same `SpotLights` buffer / layout as `tile_3d.wgsl` (group 3).
struct SpotLight {
    pos: vec4<f32>,
    dir: vec4<f32>,
    color: vec4<f32>,
    params: vec4<f32>,
}
struct SpotLights {
    count: vec4<u32>,
    lights: array<SpotLight, 8>,
}
@group(3) @binding(0) var<uniform> spot_lights: SpotLights;

// ── SSR globals (group 3, bindings 1–4; spotlights occupy binding 0) ──
// WebGPU allows only four bind groups (0..3). SSR shares group 3 with
// `spot_lights` so lit_mesh stays within the limit.
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
    // x = felt procedural LOD (0..2); y = physical HDR (glTF punctual + ACES); z = linear exposure;
    // w = ambient hemispheric scale (see `upload_camera_uniforms`).
    felt: vec4<f32>,
    // x = 1/shop_env_world_scale for embedded glTF punctual (document-space falloff); 0 = world units.
    // y = shop vitrine material tuning (1 = shop + embedded punctual); attenuation uses x only.
    shop_punctual: vec4<f32>,
};
@group(3) @binding(1) var<uniform> ssr_globals: SsrGlobals;
@group(3) @binding(2) var ssr_scene_prev: texture_2d<f32>;
@group(3) @binding(3) var ssr_depth: texture_depth_2d;
@group(3) @binding(4) var ssr_samp: sampler;

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
    let gs = sum / 9.0;
    let phys_hdr = clamp(ssr_globals.felt.y, 0.0, 1.0);
    return mix(gs, 1.0, phys_hdr);
}

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_n: vec3<f32>,
    @location(2) local_pos: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) local_n: vec3<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) _tangent_pad: vec4<f32>,
) -> VsOut {
    var world_normal = normalize((mesh.model * vec4<f32>(n, 0.0)).xyz);
    var world_pos_out = (mesh.model * vec4<f32>(pos, 1.0)).xyz;

    // Lacquered wood (kind 3 — the table): evaluate the procedural wood
    // field in WORLD XY coordinates (Z-up) rather than local mesh coordinates.
    // Displacement is along world +Z (table normal); tangents span XY.
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
        let wxy = world_pos_out.xy;
        let h_c = wood_height_world(wxy);
        let h_x = wood_height_world(wxy + vec2<f32>(eps_w, 0.0));
        let h_y = wood_height_world(wxy + vec2<f32>(0.0, eps_w));

        world_pos_out.z = world_pos_out.z + h_c * amp;

        let dh_x = (h_x - h_c) * amp;
        let dh_y = (h_y - h_c) * amp;
        let t_x = vec3<f32>(eps_w, 0.0, dh_x);
        let t_y = vec3<f32>(0.0, eps_w, dh_y);
        world_normal = normalize(cross(t_x, t_y));
    }

    var o: VsOut;
    o.clip_pos = mesh.view_proj * vec4<f32>(world_pos_out, 1.0);
    o.world_pos = world_pos_out;
    o.world_n = world_normal;
    // Pass the *undisplaced* local position so the FS samples the wood
    // basis at the same surface coordinate the VS displaced from.
    o.local_pos = pos;
    o.uv = uv;
    o.local_n = n;
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

// 2D Voronoi edge field. Returns F2 - F1, the distance from the sample
// point to the *nearest cell border* — small near a border, larger inside
// a cell. Threshold the result with a small constant to draw the edge
// network. Used for porcelain crazing (the spider-web of fine cracks
// across aged glaze) but generic. Cells are jittered random points
// inside a 3x3 lattice neighborhood, so cell shape is irregular like
// real shrinkage cracks rather than a regular tile floor.
fn voronoi2_edge(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    var f1 = 8.0;
    var f2 = 8.0;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let g = vec2<f32>(f32(dx), f32(dy));
            let h = vec2<f32>(
                hash21(i + g),
                hash21(i + g + vec2<f32>(17.3, 4.7)),
            );
            let r = g + h - f;
            let d = dot(r, r);
            if (d < f1) {
                f2 = f1;
                f1 = d;
            } else if (d < f2) {
                f2 = d;
            }
        }
    }
    return sqrt(f2) - sqrt(f1);
}

// 3D value noise: trilinear-blends 8 hashed lattice corners. Cheap and
// orientation-independent, used for procedural surface texture (gold
// nugget pitting) where 2D screen-space noise would shift with view.
fn hash31(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}
fn noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let n000 = hash31(i + vec3<f32>(0.0, 0.0, 0.0));
    let n100 = hash31(i + vec3<f32>(1.0, 0.0, 0.0));
    let n010 = hash31(i + vec3<f32>(0.0, 1.0, 0.0));
    let n110 = hash31(i + vec3<f32>(1.0, 1.0, 0.0));
    let n001 = hash31(i + vec3<f32>(0.0, 0.0, 1.0));
    let n101 = hash31(i + vec3<f32>(1.0, 0.0, 1.0));
    let n011 = hash31(i + vec3<f32>(0.0, 1.0, 1.0));
    let n111 = hash31(i + vec3<f32>(1.0, 1.0, 1.0));
    let nx00 = mix(n000, n100, u.x);
    let nx10 = mix(n010, n110, u.x);
    let nx01 = mix(n001, n101, u.x);
    let nx11 = mix(n011, n111, u.x);
    return mix(mix(nx00, nx10, u.y), mix(nx01, nx11, u.y), u.z);
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

// World-space scale factors that map (world_x, world_y) into the wood
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
fn wood_basis_world(world_xy: vec2<f32>) -> WoodBasis {
    return wood_basis_p(vec2<f32>(
        world_xy.x * TABLE_WOOD_SCALE_X,
        world_xy.y * TABLE_WOOD_SCALE_Z,
    ));
}

// Signed surface height from the wood basis. Positive on early-wood
// ridges, negative inside late-wood lines and pores. Returned in
// world units so the vertex shader can apply it directly to world Z.
fn wood_height(local_xy: vec2<f32>) -> f32 {
    let b = wood_basis(local_xy);
    // Early-wood bulges up; late-wood and pores recess. The mean is
    // close to zero so displacement does not visibly raise the slab.
    return b.early_wood * 0.55 - b.late_wood * 0.85 - b.pore * 1.6;
}

fn wood_height_world(world_xy: vec2<f32>) -> f32 {
    let b = wood_basis_world(world_xy);
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

fn wood_sample_world(world_xy: vec2<f32>) -> WoodSample {
    return wood_sample_basis(wood_basis_world(world_xy));
}

// ── Felt (mahjong-parlor green baize) ─────────────────────────────────
// Procedural felt sampler used by the `FeltGreen` material on the
// horizontal table. Samples in world XY so the felt grain stays at a
// fixed physical scale regardless of how far the table is stretched
// (the gameplay table is enlarged to "infinite plane" extents). Returns
// a low-saturation green albedo plus a slightly perturbed normal so the
// broad sheen breaks up across the surface.
struct FeltSample {
    albedo: vec3<f32>,
    n: vec3<f32>,
};

// Sample the procedural felt field. `micro_strength` controls how much
// of the high-frequency microfiber normal perturbation to apply — call
// sites fade this out at distance so the sub-mm fibers don't alias
// into screen-space noise on the horizon.
//
// Stage 9 hook: to escape procedural land entirely and use authored
// PBR textures, replace the body of this function with samples from
// new bindings (albedo, normal, roughness, AO, height) added to the
// lit_mesh material layout. Keep this signature so the call sites in
// the FS don't change. The procedural path is fine as a fallback.
fn felt_sample_world(
    world_xy: vec2<f32>,
    base_n: vec3<f32>,
    micro_strength: f32,
    lod: f32,
) -> FeltSample {
    let lod_clamped = clamp(lod, 0.0, 2.0);

    // Effects Off — uniform baize tint; skips all procedural noise.
    if (lod_clamped < 0.5) {
        var s: FeltSample;
        s.albedo = vec3<f32>(0.038, 0.135, 0.058);
        s.n = normalize(base_n);
        return s;
    }

    // World-space frequency of the felt fiber field (after early-out so
    // Metal doesn't emit an unused module constant on the uniform path).
    const FELT_SCALE: f32 = 0.18;
    let p = world_xy * FELT_SCALE;

    // ── Screen-space derivative for noise filtering ─────────────────
    let footprint = max(fwidth(p.x), fwidth(p.y));

    let macro_filter = 1.0 - smoothstep(0.4, 1.4, footprint * 0.6);
    let macro_var = clamp(
        0.5 + (fbm2(p * 0.6) - 0.5) * macro_filter * 0.4,
        0.0, 1.0,
    );

    let dark  = vec3<f32>(0.022, 0.080, 0.035);
    let mid_c = vec3<f32>(0.038, 0.135, 0.058);
    let light = vec3<f32>(0.060, 0.185, 0.085);
    var c = mix(dark, mid_c, macro_var);
    c = mix(c, light, smoothstep(0.55, 0.95, macro_var));

    let full_detail = lod_clamped >= 1.995;

    // Effects Low — macro palette + light fiber normal; no microfiber /
    // mid-octave / multi-tap micro normals (major ALU savings).
    if (!full_detail) {
        let fiber_p = p * vec2<f32>(38.0, 14.0);
        let fiber_footprint = max(fwidth(fiber_p.x), fwidth(fiber_p.y));
        let fiber_filter = 1.0 - smoothstep(0.5, 1.5, fiber_footprint);
        let fiber = (vnoise2(fiber_p) - 0.5) * 0.12 * fiber_filter;
        c = c * (1.0 + fiber * 0.6);
        let dx = (vnoise2(fiber_p + vec2<f32>(0.0, 1.7)) - 0.5);
        let dy = (vnoise2(fiber_p + vec2<f32>(2.3, 0.0)) - 0.5);
        let macro_offset = vec3<f32>(dx, dy, 0.0) * 0.05 * fiber_filter;
        var s: FeltSample;
        s.albedo = max(c, vec3<f32>(0.0));
        s.n = normalize(base_n + macro_offset);
        return s;
    }

    // Mid-frequency tonal noise — fills in between the low octave and
    // the fiber octave so the felt has variation at the cm-scale,
    // not just the chunky 10-cm-scale. Faded out independently.
    let mid_filter = 1.0 - smoothstep(0.05, 0.20, footprint);
    let mid_n = (fbm2(p * 3.5) - 0.5) * mid_filter * 0.18;

    // High-frequency anisotropic fiber noise: stretched along Y so the
    // fuzz reads as combed in one direction. Same multi-octave fade
    // so the fibers don't alias into chunky lattice cells.
    let fiber_p = p * vec2<f32>(38.0, 14.0);
    let fiber_footprint = max(fwidth(fiber_p.x), fwidth(fiber_p.y));
    let fiber_filter = 1.0 - smoothstep(0.5, 1.5, fiber_footprint);
    let fiber = (vnoise2(fiber_p) - 0.5) * 0.12 * fiber_filter;

    // Sub-mm microfiber field: a 5.5× higher-frequency version of the
    // fiber noise. Faded with distance via `micro_strength` PLUS the
    // per-pixel footprint check.
    let micro_p = fiber_p * 5.5;
    let micro_footprint = fiber_footprint * 5.5;
    let micro_filter = (1.0 - smoothstep(0.5, 1.5, micro_footprint)) * micro_strength;

    // Subtle fiber-tinted variance so the surface doesn't read as a
    // flat painted plane. Mid octave folds in for cm-scale variation.
    c = c * (1.0 + fiber * 0.6 + mid_n);

    // Perturb the normal along TWO octaves of the fiber field. Both
    // octaves are footprint-filtered to avoid the noise lattice
    // showing through as visible blocks on distant surface.
    let dx = (vnoise2(fiber_p + vec2<f32>(0.0, 1.7)) - 0.5);
    let dy = (vnoise2(fiber_p + vec2<f32>(2.3, 0.0)) - 0.5);
    let mdx = (vnoise2(micro_p + vec2<f32>(0.0, 1.7)) - 0.5);
    let mdy = (vnoise2(micro_p + vec2<f32>(2.3, 0.0)) - 0.5);
    let macro_offset = vec3<f32>(dx,  dy,  0.0) * 0.05 * fiber_filter;
    let micro_offset = vec3<f32>(mdx, mdy, 0.0) * 0.025 * micro_filter;
    let n  = normalize(base_n + macro_offset + micro_offset);

    var s: FeltSample;
    s.albedo = max(c, vec3<f32>(0.0));
    s.n = n;
    return s;
}

// Soft circle field: returns a 0..1 mask where 1 is the centre of any
// circle sampled by the hash. Used for tile-contact ghosts (Stage 5)
// and edge-wear hotspots (Stage 6). Each cell of a coarse grid hashes
// to a centre offset and a radius — most cells produce nothing, a few
// produce dim rings.
fn felt_contact_field(world_xy: vec2<f32>, cell_size: f32, density: f32, max_r: f32) -> f32 {
    let g = world_xy / cell_size;
    let cell = floor(g);
    let g_frac = g - cell;
    var acc = 0.0;
    // 3x3 neighbourhood lookup — handles circles that span cell boundaries.
    for (var dx = -1; dx <= 1; dx = dx + 1) {
        for (var dy = -1; dy <= 1; dy = dy + 1) {
            let neighbour = cell + vec2<f32>(f32(dx), f32(dy));
            let h0 = hash21(neighbour);
            // Most cells skip — only `density` fraction host a ring.
            if (h0 > density) {
                continue;
            }
            let h1 = hash21(neighbour + vec2<f32>(17.3, 0.0));
            let h2 = hash21(neighbour + vec2<f32>(0.0, 23.7));
            let h3 = hash21(neighbour + vec2<f32>(91.1, 41.5));
            // Centre offset within the cell + radius scale.
            let centre_off = vec2<f32>(f32(dx), f32(dy)) + vec2<f32>(h1, h2);
            let r = mix(0.18, max_r, h3);
            let d = length(g_frac - centre_off);
            // Soft falloff — fades to zero at the radius.
            let m = 1.0 - smoothstep(r * 0.4, r, d);
            acc = max(acc, m);
        }
    }
    return acc;
}

// Apply Stage 5 (contact ghosts), Stage 6 (edge wear + dust), and
// Stage 7 (sub-visible weave) to a base felt albedo. Kept in one
// function so the felt branch in the FS is just a single call. All
// detail layers are filtered by the screen-space derivative so they
// fade out cleanly as the surface moves further from the camera —
// this is what kills the pixellated lattice look on distant felt.
fn felt_apply_wear_dust(base: vec3<f32>, world_xy: vec2<f32>, lod: f32) -> vec3<f32> {
    // Contact rings + weave are skipped below Medium — `felt_contact_field`
    // is the hottest helper here (nested 3×3 grid).
    if (lod < 1.995) {
        return max(base, vec3<f32>(0.0));
    }

    var c = base;
    let footprint = max(fwidth(world_xy.x), fwidth(world_xy.y));

    // ── Stage 5: Tile contact ghosts ───────────────────────────────
    // Persistent dark rings sprinkled across the felt — read as the
    // memory of past tile placements where the nap was compressed.
    // Amplitude pulled back to a near-imperceptible 0.8% so the surface
    // reads as fresh card-table baize rather than a worn parlor table.
    let contact_filter = 1.0 - smoothstep(8.0, 24.0, footprint);
    let contact_mask = felt_contact_field(world_xy, 36.0, 0.10, 0.5);
    c = c * (1.0 - contact_mask * 0.008 * contact_filter);

    // ── Stage 6a: Edge-wear hotspots ───────────────────────────────
    // Disabled: the wear ring read as a stain on fresh baize. Real
    // card-table felt only shows wear after years of use; the table
    // here should look new.

    // ── Stage 6b: Dust / lint specks ──────────────────────────────
    // Disabled. At any visible density these read as digital noise on
    // the felt rather than as dust — the high-frequency fiber field
    // already carries the surface variation we need.

    // ── Stage 7: Sub-visible weave lattice ─────────────────────────
    // Real baize has a fine perpendicular weave. Two perpendicular
    // sin grids modulate the brightness by ±2% — invisible at game
    // distance, adds depth on close-up screenshots. Fades out as the
    // weave frequency exceeds Nyquist for the current pixel footprint.
    let weave_freq = 28.0;
    let weave_filter = 1.0 - smoothstep(0.05, 0.18, footprint * weave_freq);
    let weave_x = sin(world_xy.x * weave_freq);
    let weave_y = sin(world_xy.y * weave_freq);
    let weave = (weave_x + weave_y) * 0.5;
    c = c * (1.0 + weave * 0.025 * weave_filter);

    return max(c, vec3<f32>(0.0));
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
    //   0 = Plain, 1 = Wax, 2 = Wick, 3 = LacqueredWood,
    //   4 = LacqueredWoodFlat, 5 = Metal, 6 = Water,
    //   8 = Foil, 9 = Glass, 10 = Enamel,
    //   11 = Jade, 12 = Moonstone, 13 = Pearl, 14 = GoldNugget,
    //   15 = Polychrome, 16 = Porcelain, 17 = Brass, 18 = Leather,
    //   19 = FeltGreen, 20 = Emissive
    let is_wax       = (kind > 0.5 && kind < 1.5);
    let is_wick      = (kind > 1.5 && kind < 2.5);
    let is_wood      = (kind > 2.5 && kind < 4.5);
    let is_metal     = (kind > 4.5 && kind < 5.5);
    let is_water_mat = (kind > 5.5 && kind < 6.5);
    let is_foil      = (kind > 7.5 && kind < 8.5);
    let is_glass     = (kind > 8.5 && kind < 9.5);
    let is_enamel    = (kind > 9.5 && kind < 10.5);
    let is_jade      = (kind > 10.5 && kind < 11.5);
    let is_moonstone = (kind > 11.5 && kind < 12.5);
    let is_pearl     = (kind > 12.5 && kind < 13.5);
    let is_goldnug   = (kind > 13.5 && kind < 14.5);
    let is_poly      = (kind > 14.5 && kind < 15.5);
    let is_porcelain = (kind > 15.5 && kind < 16.5);
    let is_brass     = (kind > 16.5 && kind < 17.5);
    let is_leather   = (kind > 17.5 && kind < 18.5);
    let is_felt      = (kind > 18.5 && kind < 19.5);
    let is_emissive  = (kind > 19.5 && kind < 20.5);
    let felt_lod = clamp(ssr_globals.felt.x, 0.0, 2.0);
    let phys_hdr = clamp(ssr_globals.felt.y, 0.0, 1.0);
    // Shop storeroom-only albedo / spec / ambient tweaks (see `shop_punctual.y` upload).
    let shop_vitrine_tuning = phys_hdr > 0.5 && ssr_globals.shop_punctual.y > 0.5;

    // Brass is a conductor too; group with metal for the per-light
    // Fresnel-spec branch and for the rim halo. Skips the coin-face
    // heightmap perturbation since brass fittings are smooth, not
    // engraved.
    let is_conductor = (is_metal || is_brass);
    // Any of the five carved-tablet materials: shares the heightmap
    // normal-perturbation pipeline and the rim/SSS treatment.
    let is_talisman  = (is_jade || is_moonstone || is_pearl || is_goldnug || is_poly);

    // Sample the albedo texture unconditionally — material kind is uniform
    // across the draw, but hoisting the sample keeps naga's uniform-control-
    // flow analysis happy regardless of how it inlines the branch below.
    let tex_sample = textureSample(albedo_tex, albedo_samp, in.uv);
    let tex_rgb = tex_sample.rgb;
    // material_params.w doubles as a "this instance has an engraved decal"
    // flag for non-talisman materials. When >0.5 the texture is treated as
    // a transparent overlay (engraved label) composited *over* the procedural
    // base material rather than multiplied with it. For talismans, .w carries
    // the per-kind heightmap index used by the relief sampling (no per-kind
    // shader branching keys off it any more — each MaterialKind has its own
    // dedicated branch).
    let has_decal = mesh.material_params.w > 0.5 && !is_talisman && !is_foil && !is_enamel && !is_felt && !is_wick;
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
    if (is_brass) {
        // Brass: smooth polished conductor, no heightmap. Albedo is the
        // base tint and the conductor Fresnel sheen does the rest.
        albedo = mesh.base_color.rgb;
    }
    if (is_leather) {
        // UV.x sentinels (set in book_mesh.rs):
        //   < 1.5             : leather body (default)
        //   ≈ 2.0             : page-stack edge (cream stratified paper)
        //   ≈ 3.0             : silk ribbon bookmark (saturated gold)
        //   ≥ 3.5 (4.0 + tex_u): page-content surface, sample journal
        //                        render target at (uv.x − 4.0, uv.y)
        if (in.uv.x > 3.5) {
            // Page content. The book body's per-instance base_color.a
            // carries `open_amount` (0 = closed, 1 = fully open) so we
            // can discard page fragments while the cover still
            // occludes them. Once the cover has swung > halfway, the
            // page surface fades in; before that, it's invisible.
            if (mesh.base_color.a < 0.5) {
                discard;
            }
            // Sample the live journal-scene render target at slot 3
            // (`relief_tex`) in **screen space**, not UV. The journal
            // scene was rendered into a fullscreen texture matching the
            // swapchain dims, so reading by `frag_coord / screen_size`
            // makes the page region read as a window cut through the
            // page mesh into the underlying scene. As the book scales
            // up during the open/close transition, the page region
            // grows on screen and reveals more of the journal frame —
            // at full zoom the page covers the whole viewport and the
            // shop-side journal scene push is a visual no-op.
            let tex_dim = vec2<f32>(textureDimensions(relief_tex, 0));
            let tex_uv = in.clip_pos.xy / tex_dim;
            albedo = textureSampleLevel(relief_tex, albedo_samp, tex_uv, 0.0).rgb;
        } else if (in.uv.x > 2.5) {
            // Silk ribbon — saturated gold-yellow that pops against
            // the cordovan cover. Subtle striations along the ribbon
            // axis (local Z) read as woven silk grain.
            let weft = 0.5 + 0.5 * cos(in.local_pos.z * 180.0);
            let warp = 0.5 + 0.5 * cos(in.local_pos.x * 220.0);
            let weave = 0.85 + 0.15 * (weft * 0.6 + warp * 0.4);
            let silk_a = vec3<f32>(1.20, 0.88, 0.34);
            let silk_b = vec3<f32>(0.78, 0.50, 0.18);
            albedo = mix(silk_b, silk_a, weave);
        } else if (in.uv.x > 1.5) {
            // Page-stack edge — procedural cream paper striations on
            // the visible edges of the bound page block. Independent
            // of the journal render target.
            let stripe = 0.5 + 0.5 * cos(in.local_pos.y * 220.0);
            let cream_a = vec3<f32>(1.05, 0.98, 0.84);
            let cream_b = vec3<f32>(0.84, 0.74, 0.55);
            albedo = mix(cream_b, cream_a, stripe) * 0.96;
        } else {
            // Leather body: procedural cordovan with grain-driven tonal
            // variation. Two octaves of noise modulate the flat base
            // tint so the cover reads as hand-rubbed leather rather
            // than painted plastic. The carved-decal block below lays
            // the gold title on top of this grained under-coat.
            let coarse = noise3(in.local_pos * 18.0);
            let fine   = noise3(in.local_pos * 64.0);
            // Tone the body (±9%) — enough to break up the flat surface
            // without reading as spotty. Bias toward darker so the
            // base oxblood stays rich.
            let tone = 0.91 + 0.18 * coarse;
            // Fine pore darkening — micro-pits where dye pools deeper.
            let pore = 1.0 - 0.18 * smoothstep(0.55, 0.85, fine);
            // Subtle warming on the high points so the polish picks up
            // gold rather than washing toward pink.
            let high = smoothstep(0.55, 0.95, coarse);
            albedo = mesh.base_color.rgb * tone * pore + vec3<f32>(0.05, 0.025, 0.0) * high;
        }
    }
    if (is_foil) {
        // Foil: base_color.rgb is the metallic wrapper tint; the texture is
        // a decal whose alpha masks where the art sits on the front of the
        // pack. Front face is local -Y (see build_pack_mesh). Back + edges
        // stay pure foil so the pack reads as a wrapped card from every
        // angle. Metallic foil sheen is layered on top in the per-light
        // loop regardless — decal art still catches the iridescence.
        // `material_params.w > 0.5` signals a talisman-shaped foil draw
        // (different mesh axis, heightmap bound instead of decal) — skip
        // the decal composite so the tablet reads as pure gold foil.
        if (mesh.material_params.w > 0.5) {
            albedo = mesh.base_color.rgb;
        } else {
            // Keep ~18% of the foil tint bleeding through even fully-opaque
            // art so the metallic sheen in the lighting pass has a tinted
            // base to modulate. Pure-white decal pixels otherwise wash out
            // the wrapper and kill the "foil card pack" read.
            let front_face = smoothstep(-0.42, -0.48, in.local_pos.y);
            let decal_mask = tex_sample.a * front_face * 0.82;
            albedo = mix(mesh.base_color.rgb, tex_rgb, decal_mask);
        }
    }
    if (is_glass) {
        // The bound texture remains visible, but we brighten and cool it so
        // the small prop reads more like glazed glass than painted plastic.
        albedo = mix(mesh.base_color.rgb * 0.85, tex_rgb, 0.55);
    }
    if (is_enamel) {
        // Relic mesh caps have local ±Y normals; discriminate in LOCAL
        // space so the texture shows regardless of how the placement
        // orients the mesh in world space (collection uses a Y-up
        // overhead camera; shop/gameplay use the Z-up table camera).
        let cap_mask = smoothstep(0.55, 0.82, abs(in.local_n.y));
        // Shop GLB vitrine: caps used `tex_rgb` alone — under punctual + key shadow,
        // dark tex reads as a black slab while foil/talisman spec stays hot. Tint caps
        // by `base_color` like the sides so material + rarity survive.
        if (shop_vitrine_tuning) {
            let tinted = mesh.base_color.rgb * tex_rgb;
            albedo = mix(mesh.base_color.rgb, tinted, cap_mask);
        } else {
            albedo = mix(mesh.base_color.rgb, tex_rgb, cap_mask);
        }
    }
    var wood_grain = 0.0;
    var wood_pore = 0.0;
    if (is_wood) {
        // Lacquered wood: procedural grain overrides the (white) albedo tex.
        // Kind 3 (the horizontal table) samples in world XY so the grain
        // tiles at a fixed world-space frequency regardless of the model's
        // scale — this is what lets the table extend to the horizon as an
        // "infinite plane" without stretching the rings. Kind 4 (the
        // upright score plaque) keeps using local surface coords because
        // its slab face isn't aligned with world XY.
        var w: WoodSample;
        if (kind < 3.5) {
            w = wood_sample_world(in.world_pos.xy);
        } else {
            w = wood_sample(in.local_pos);
        }
        albedo = w.albedo;
        wood_grain = w.grain;
        wood_pore = w.pore;
    }
    if (is_felt) {
        // Mahjong-parlor green felt: procedural baize overrides the (white)
        // albedo tex. Sampled in world XY for the same reason as kind 3 —
        // the gameplay table is enlarged to "infinite plane" extents and
        // we don't want the felt grain to stretch with the model scale.
        // The perturbed normal is folded into `n` after the world-normal
        // setup further down. Albedo doesn't need the microfiber octave
        // (only the normal does), so we pass strength 0 here.
        let f = felt_sample_world(in.world_pos.xy, vec3<f32>(0.0, 0.0, 1.0), 0.0, felt_lod);
        albedo = f.albedo;
        albedo = felt_apply_wear_dust(albedo, in.world_pos.xy, felt_lod);
    }

    // ── Porcelain crazing (the spider-web of fine cracks across aged
    // glaze) ────────────────────────────────────────────────────────
    // Age is driven by `base_color.r`: pristine white (r≈1) stays clean,
    // sepia/cream (r≈0.85) gets a light crackle, antique brown (r≈0.55)
    // gets heavy crazing. This couples the cracks to the visual age the
    // artist is already painting via tint, without needing a new uniform
    // field. The crack mask is hoisted so the porcelain spec block below
    // can dip the glaze lobes inside the cracks, and the normal block
    // can break the highlight across them.
    var crazing_age = 0.0;
    var crack_mask = 0.0;
    var crack_local_perturb = vec3<f32>(0.0);
    if (is_porcelain) {
        crazing_age = clamp((1.0 - mesh.base_color.r) * 1.6, 0.0, 1.0);
        if (crazing_age > 0.001) {
            // Tri-planar pick: project local_pos onto whichever pair of
            // local axes is most perpendicular to the local normal, so a
            // ±Y-faced cap parameterizes in XZ, a ±Z-faced top in XY, and
            // ±X side walls in YZ. Cell continuity breaks at the
            // dominant-axis switch but real glaze crazing is independent
            // per-face anyway (each face has its own shrinkage stress),
            // so the seam reads as natural.
            let an = abs(in.local_n);
            var p2: vec2<f32>;
            var basis_u: vec3<f32>;
            var basis_v: vec3<f32>;
            if (an.y >= an.x && an.y >= an.z) {
                p2 = in.local_pos.xz;
                basis_u = vec3<f32>(1.0, 0.0, 0.0);
                basis_v = vec3<f32>(0.0, 0.0, 1.0);
            } else if (an.z >= an.x) {
                p2 = in.local_pos.xy;
                basis_u = vec3<f32>(1.0, 0.0, 0.0);
                basis_v = vec3<f32>(0.0, 1.0, 0.0);
            } else {
                p2 = in.local_pos.yz;
                basis_u = vec3<f32>(0.0, 1.0, 0.0);
                basis_v = vec3<f32>(0.0, 0.0, 1.0);
            }

            // Voronoi cells in object space. The mesh local extent is
            // ~1 unit, so a frequency of ~16 yields ~16 cells across the
            // major silhouette — large enough that individual cracks
            // read as visible lines at typical relic / tablet sizes
            // (~80–120px on screen when the player is engaged with the
            // prop), rather than dissolving into per-pixel speckle. The
            // hash-jitter inside the helper means each instance gets a
            // unique pattern.
            let p_obj = p2 * 16.0;
            let edge = voronoi2_edge(p_obj);
            // Thin line, AA'd by screen-space derivative so subpixel
            // cracks don't shimmer. The fwidth term means the line
            // visibly fades to nothing once each crack is much
            // narrower than a pixel — no separate distance-fade
            // needed, this scales correctly to whatever projection.
            let line_w = 0.08 + fwidth(edge) * 1.2;
            let crack = 1.0 - smoothstep(0.0, line_w, edge);
            // Coverage modulation — some patches of an aged piece craze
            // densely while others stay clear. Without this every
            // antique relic looks uniformly cracked. Frequency picked
            // so the coverage patches are roughly 2–3× the cell size.
            let cov_noise = vnoise2(p2 * 2.5);
            let coverage = smoothstep(0.20, 0.85, cov_noise * 0.7 + 0.8);
            crack_mask = crack * coverage * crazing_age;

            // Finite-difference the edge field for normal perturbation.
            // Each crack tilts the surface slightly so the highlight
            // breaks across the fracture instead of sliding over it.
            // eps in the same units as p_obj (post-frequency scale).
            let eps = 0.05;
            let edge_u = voronoi2_edge(p_obj + vec2<f32>(eps, 0.0));
            let edge_v = voronoi2_edge(p_obj + vec2<f32>(0.0, eps));
            let dhdu = (edge_u - edge) / eps * crack_mask * 0.4;
            let dhdv = (edge_v - edge) / eps * crack_mask * 0.4;
            crack_local_perturb = -basis_u * dhdu - basis_v * dhdv;

            // Tea/dust stain in the cracks. Real aged crazing is amber
            // /sepia, never pure black — liquid wicks into the body
            // through the cracks and stains the surrounding glaze.
            // A second slow noise lets some cracks read younger and
            // some darker, which sells "decades of accumulated stain"
            // rather than "uniformly inked decal".
            let stain_var = 0.5 + 0.5 * vnoise2(p2 * 0.6);
            let stain_strength = mix(0.55, 0.95, stain_var);
            let stain_tint = vec3<f32>(0.40, 0.30, 0.22);
            albedo = mix(albedo, albedo * stain_tint, crack_mask * stain_strength);
        }
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

    // Candle-keyed lighting: no directional sun; wick PointLights drive the
    // per-light loop below. `rgb` starts at black; fragments outside candle
    // pools stay dark.
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

    // ── Leather grain normal perturbation ───────────────────────────
    // Sample noise at two scales and finite-difference it into a
    // tangent-space normal so the cover catches raking light as if
    // pebbled. Same model-matrix re-orientation pattern as the carved
    // decal — the book is a uniformly-scaled local +Y face after the
    // shop's `cam_rot`, so the upper-3×3 transform is exact. Skipped on
    // page-edge fragments (paper is flat, not pebbled).
    if (is_leather && in.uv.x < 1.5) {
        let p = in.local_pos * 32.0;
        let eps = 0.04;
        let n_c = noise3(p);
        let n_x = noise3(p + vec3<f32>(eps, 0.0, 0.0));
        let n_z = noise3(p + vec3<f32>(0.0, 0.0, eps));
        let grain_amp = 0.55;
        let dhdu = (n_x - n_c) * grain_amp;
        let dhdv = (n_z - n_c) * grain_amp;
        let perturbed_local = normalize(vec3<f32>(-dhdu, 1.0, -dhdv));
        let perturbed_world = normalize((mesh.model * vec4<f32>(perturbed_local, 0.0)).xyz);
        n = normalize(mix(n, perturbed_world, 0.45));
    }

    // ── Porcelain crazing normal perturbation ───────────────────────
    // The local-space perturbation vector was computed up in the albedo
    // block (already aligned to the tri-planar tangent basis we picked
    // there). Add it to the local normal, transform back to world, and
    // mix in by the crack mask so flat glaze keeps its mirror-finish
    // and only crack cells flex the highlight.
    if (is_porcelain && crack_mask > 0.001) {
        let perturbed_local = normalize(in.local_n + crack_local_perturb);
        let perturbed_world = normalize((mesh.model * vec4<f32>(perturbed_local, 0.0)).xyz);
        n = normalize(mix(n, perturbed_world, clamp(crack_mask * 0.6, 0.0, 0.6)));
    }

    // ── Felt fiber normal perturbation ──────────────────────────────
    // Resample the felt fiber field (the same one that drove albedo)
    // and use its perturbed normal so the broad sheen breaks up across
    // the surface. Felt isn't a mirror but it isn't perfectly Lambertian
    // either — this is what makes the candle pools read as "soft glow on
    // baize" rather than "spotlight on painted plane".
    //
    // The microfiber octave (sub-mm fiber wobble) is faded out with
    // distance from the camera so distant felt doesn't alias into
    // shimmering noise on the horizon. Within ~0.3 world units of the
    // camera we get full microfiber detail; past 1.5 units it drops to
    // zero and only the macro fiber field remains.
    if (is_felt) {
        let cam_for_fade = ssr_globals.view_pos.xyz;
        let dist = length(cam_for_fade - in.world_pos);
        let micro_strength = 1.0 - smoothstep(0.3, 1.5, dist);
        let f = felt_sample_world(in.world_pos.xy, n, micro_strength, felt_lod);
        n = f.n;
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
            // Reconstruct the meander centerline so flow-relative
            // quantities (bank distance, along-flow scroll) line up
            // with the geometry in river_mesh.rs. Must match:
            //   centerline_z(t) = sin(tau*1.6*t)*0.085
            //                   + sin(tau*3.68*t + 0.7)*0.085*0.22
            //   half_width(t)   = 0.15 + 0.055*(sin(tau*1.1*t+1.3)*0.6
            //                                  + sin(tau*0.6*t-0.4)*0.4)
            // uv.x carries the flow parameter t ∈ [0, 1] on water verts.
            let t = in.uv.x;
            let tau = 6.28318530718;
            let center_z = sin(tau * 1.6 * t) * 0.085
                         + sin(tau * 3.68 * t + 0.7) * 0.085 * 0.22;
            let hw = 0.15 + 0.055 * (sin(tau * 1.1 * t + 1.3) * 0.6
                                    + sin(tau * 0.6 * t - 0.4) * 0.4);
            // Offset from centerline, normalized so ±1 is the shore.
            let z_off = (in.local_pos.z - center_z) / max(hw, 0.01);
            let bank_mask = smoothstep(0.55, 1.0, abs(z_off));
            let centerline = clamp(1.0 - abs(z_off), 0.0, 1.0);

            // Directional flow: two noise layers scrolling along the
            // river's length parameter t (not world X), so the current
            // follows the meander. Cross-flow noise scaled much tighter
            // than along-flow so wavelets elongate into stream streaks.
            let p = vec2<f32>(t * 9.0, z_off * 3.2);
            let f1 = fbm2(p + vec2<f32>(time * 1.10, time * 0.04));
            let f2 = fbm2(p * vec2<f32>(1.6, 2.1) + vec2<f32>(time * 0.70, -time * 0.06));
            // Slow wide-lattice layer for long drifting swells.
            let p_slow = vec2<f32>(t * 3.2, z_off * 1.4);
            let f3 = fbm2(p_slow + vec2<f32>(time * 0.42, 0.0));
            // Foam crests: streamwise ribbons, boosted against the shore
            // where real whitewater piles up.
            let crest_raw = f1 * 0.55 + f2 * 0.45 + f3 * 0.25;
            let crest = smoothstep(0.55, 0.82, crest_raw)
                      + bank_mask * 0.40 * smoothstep(0.30, 0.65, f1);
            // Anisotropic finite-difference normal.
            let eps_t = 0.9;
            let eps_z = 0.30;
            let h_c = f1 + f2 + f3 * 0.5;
            let h_t = fbm2(p + vec2<f32>(eps_t + time * 1.10, time * 0.04))
                    + fbm2(p * vec2<f32>(1.6, 2.1) + vec2<f32>(eps_t * 1.6 + time * 0.70, -time * 0.06))
                    + fbm2(p_slow + vec2<f32>(eps_t + time * 0.42, 0.0)) * 0.5;
            let h_z = fbm2(p + vec2<f32>(time * 1.10, eps_z + time * 0.04))
                    + fbm2(p * vec2<f32>(1.6, 2.1) + vec2<f32>(time * 0.70, eps_z * 2.1 - time * 0.06))
                    + fbm2(p_slow + vec2<f32>(time * 0.42, eps_z)) * 0.5;
            // The water strip is nearly axis-aligned with local X (the
            // meander angle is small) so we map these derivatives back
            // to local X / Z with a slight rotation. Cheaper: just use
            // the ratio, since small meander slope doesn't shift the
            // ripple orientation noticeably at this scale.
            var dhdu_total = (h_t - h_c) * 0.22;
            var dhdv_total = (h_z - h_c) * 1.00;
            // Indigo deep channel → teal lift at the shore and in the
            // noise valleys. Centerline runs brightest.
            let deep = vec3<f32>(0.012, 0.022, 0.058);
            let mid  = vec3<f32>(0.050, 0.095, 0.170);
            var albedo = mix(deep, mid, clamp(f3 * 1.0 + centerline * 0.40, 0.0, 1.0))
                       + vec3<f32>(crest) * vec3<f32>(0.62, 0.70, 0.84);

            // ── Bubbling spring source ─────────────────────────────────
            // At the -X end the water widens into a pool (see
            // `SPRING_CX`, `SPRING_POOL_R`, `SPRING_T` in river_mesh.rs).
            // Inside that pool we overlay concentric ripples expanding
            // outward from the spring eye, plus a bright upwelling foam
            // patch at the center. The effect fades out as the water
            // tapers into the stream.
            //   Spring center in local XZ (match river_mesh.rs):
            //     SPRING_CX = -0.48 + 0.315 + 0.02 = -0.145
            //     SPRING_CZ = centerline_z(0.0) = sin(0) + sin(0.7)*0.0187 ≈ 0.0121
            //     SPRING_POOL_R = 0.315
            //     SPRING_T = 0.18
            let spring_cx = -0.145;
            let spring_cz = sin(0.7) * 0.085 * 0.22;
            let dx = in.local_pos.x - spring_cx;
            let dz = in.local_pos.z - spring_cz;
            let r = sqrt(dx * dx + dz * dz);
            // Spring region mask: strongest at the eye, fading to 0 by
            // the time we leave the pool (r ≈ SPRING_POOL_R) or the
            // along-flow taper finishes (t ≈ SPRING_T).
            let r_mask = 1.0 - smoothstep(0.02, 0.315, r);
            let t_mask = 1.0 - smoothstep(0.0, 0.18, t);
            let spring_mask = r_mask * t_mask;

            if (spring_mask > 0.001) {
                // Concentric outgoing ripples. Phase advances with time
                // so wavefronts travel outward. Multiple rings packed
                // close together read as sustained bubbling, not a
                // single splash.
                let ripple_freq = 70.0;   // rings per unit local length
                let ripple_speed = 4.5;   // rings per second outward
                let ripple_phase = r * ripple_freq - time * ripple_speed;
                let ripple_h = cos(ripple_phase);
                // Ripple amplitude falls off from the eye and with
                // distance so the rings don't compete with the stream.
                let ripple_falloff = spring_mask * smoothstep(0.02, 0.28, r);
                // Radial gradient of ripple_h ≈ -sin(phase) * freq.
                let ripple_grad = -sin(ripple_phase) * ripple_freq * 0.012 * ripple_falloff;
                // Unit radial direction in local XZ.
                let inv_r = 1.0 / max(r, 0.0005);
                let rdx = dx * inv_r;
                let rdz = dz * inv_r;
                // Add the ripple gradient into the normal perturbation
                // (u maps to local X, v to local Z, same basis as above).
                dhdu_total = dhdu_total + ripple_grad * rdx;
                dhdv_total = dhdv_total + ripple_grad * rdz;

                // Upwelling foam: a soft bright disk at the very eye,
                // modulated by a second fast noise for roiling texture.
                let eye_mask = pow(1.0 - smoothstep(0.0, 0.11, r), 2.0);
                let turb = fbm2(vec2<f32>(dx * 28.0, dz * 28.0)
                              + vec2<f32>(time * 2.1, -time * 1.6));
                let boil = clamp(turb * 1.2 - 0.15, 0.0, 1.0);
                let upwell_foam = eye_mask * (0.45 + 0.55 * boil);

                // Brighter, more turquoise tone inside the pool; the
                // center gets a warm aerated highlight from all the
                // churn.
                let pool_tint = vec3<f32>(0.075, 0.130, 0.195);
                let foam_col = vec3<f32>(0.85, 0.92, 1.00);
                albedo = mix(albedo, pool_tint, spring_mask * 0.55)
                       + foam_col * upwell_foam * 0.75
                       + vec3<f32>(ripple_h * 0.5 + 0.5) * spring_mask * 0.04;
            }

            let n_w = normalize(vec3<f32>(-dhdu_total, 1.0, -dhdv_total));
            water_n = normalize((mesh.model * vec4<f32>(n_w, 0.0)).xyz);
            water_albedo = albedo;
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
        let cam_pos_w = ssr_globals.view_pos.xyz;
        let view_dir_w = normalize(cam_pos_w - in.world_pos);
        let pt_n_w = lights.count.x;
        for (var i: u32 = 0u; i < pt_n_w; i = i + 1u) {
            let pl = lights.lights[i];
            let lp = pl.pos.xyz;
            let range_w = pl.pos.w;
            let lc = pl.color.rgb;
            let is_inv = pl.params.x > 0.5;
            let intensity = pl.color.a * select(1.0, lights.extras.w, is_inv);
            let to_light = lp - in.world_pos;
            let dist = length(to_light);
            let atten = select(
                scene_smooth_point_atten(dist, range_w),
                punctual_attenuation_with_inv_doc_scale(dist, range_w, ssr_globals.shop_punctual.x),
                is_inv,
            );
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
        let spot_count_w = spot_lights.count.x;
        for (var si: u32 = 0u; si < spot_count_w; si = si + 1u) {
            let s = spot_lights.lights[si];
            let to_frag = in.world_pos - s.pos.xyz;
            let dist = length(to_frag);
            let radius = max(s.pos.w, 1.0);
            let t_sp = clamp(1.0 - dist / radius, 0.0, 1.0);
            let atten_sp = t_sp * t_sp;
            if (atten_sp <= 0.0) {
                continue;
            }
            let frag_dir = to_frag / max(dist, 0.0001);
            let cos_a = dot(frag_dir, s.dir.xyz);
            let spot_factor = khr_spot_angle_attenuation_scene(cos_a, s.params.x, s.dir.w);
            if (spot_factor <= 0.0) {
                continue;
            }
            let to_light = -frag_dir;
            let nl = max(dot(water_n, to_light), 0.0);
            let lambert_sp = select(0.30, 0.45, is_water) + 0.55 * nl;
            let sc = s.color.rgb * s.color.a * atten_sp * spot_factor * lights.extras.w;
            lit_water = lit_water + sc * lambert_sp;

            if (water_spec_strength > 0.001) {
                let h = normalize(to_light + view_dir_w);
                let nh = max(dot(water_n, h), 0.0);
                let sp = pow(nh, water_spec_power) * water_spec_strength;
                spec_water = spec_water + sc * sp;
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
    var enamel_height = 0.0;
    var enamel_ridge = 0.0;
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
    if (is_enamel) {
        let face_flat = abs(n.y);
        // Relief is bound separately (linear grayscale); same remap as the old
        // baked-alpha path so existing height PNGs still read correctly.
        let hr = textureSampleLevel(relief_tex, albedo_samp, in.uv, 0.0).r;
        let h_c = clamp((hr - 0.62) / 0.33, 0.0, 1.0);
        enamel_height = h_c;
        enamel_ridge = smoothstep(0.70, 0.92, h_c);
        if (face_flat > 0.6) {
            let dim = vec2<f32>(textureDimensions(relief_tex, 0));
            let texel = vec2<f32>(1.0 / max(dim.x, 1.0), 1.0 / max(dim.y, 1.0));
            let h_l = clamp((textureSampleLevel(relief_tex, albedo_samp, in.uv + vec2<f32>(-texel.x, 0.0), 0.0).r - 0.62) / 0.33, 0.0, 1.0);
            let h_r = clamp((textureSampleLevel(relief_tex, albedo_samp, in.uv + vec2<f32>( texel.x, 0.0), 0.0).r - 0.62) / 0.33, 0.0, 1.0);
            let h_d = clamp((textureSampleLevel(relief_tex, albedo_samp, in.uv + vec2<f32>(0.0, -texel.y), 0.0).r - 0.62) / 0.33, 0.0, 1.0);
            let h_u = clamp((textureSampleLevel(relief_tex, albedo_samp, in.uv + vec2<f32>(0.0,  texel.y), 0.0).r - 0.62) / 0.33, 0.0, 1.0);
            let bump = 3.6;
            let dhdu = (h_r - h_l) * bump;
            let dhdv = (h_u - h_d) * bump;
            let sgn = sign(n.y);
            let perturbed = normalize(vec3<f32>(-dhdu, sgn, -dhdv));
            var n_face = vec3<f32>(perturbed.x, perturbed.y * sgn, perturbed.z);
            n_face = normalize(n_face);
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
    let is_foil_talisman = is_foil && mesh.material_params.w > 0.5;
    if (is_talisman || is_foil_talisman) {
        let face_flat = abs(abs(in.local_pos.z) - 0.09);
        if (face_flat < 0.02) {
            let dim = vec2<f32>(textureDimensions(albedo_tex, 0));
            let texel = vec2<f32>(1.0 / max(dim.x, 1.0), 1.0 / max(dim.y, 1.0));
            let h_l = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>(-texel.x, 0.0), 0.0).r;
            let h_r = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>( texel.x, 0.0), 0.0).r;
            let h_d = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>(0.0, -texel.y), 0.0).r;
            let h_u = textureSampleLevel(albedo_tex, albedo_samp, in.uv + vec2<f32>(0.0,  texel.y), 0.0).r;
            // Bump strength tuned for shallow carved relief on jade.
            let bump = 10.0;
            var dhdu = (h_r - h_l) * bump;
            var dhdv = (h_u - h_d) * bump;
            // Gold: subtle surface variation layered on top of the carved
            // relief — just enough to break up the highlight into soft
            // caustic-like ripples without reading as pitted raw metal.
            if (is_goldnug) {
                let p2 = in.local_pos * 14.0;
                let off = vec3<f32>(0.015, 0.0, 0.0);
                let off_y = vec3<f32>(0.0, 0.015, 0.0);
                let h_c2 = noise3(p2);
                let h_x2 = noise3(p2 + off * 14.0);
                let h_y2 = noise3(p2 + off_y * 14.0);
                let pit_bump = 1.0;
                dhdu = dhdu + (h_x2 - h_c2) * pit_bump;
                dhdv = dhdv + (h_y2 - h_c2) * pit_bump;
            }
            // Screen-space derivative tangent basis: works for any
            // orientation without needing explicit tangent attributes.
            // Per-component dpdx avoids naga emitting an unused df_dy(vec3).
            let tangent = normalize(vec3<f32>(
                dpdx(in.world_pos.x),
                dpdx(in.world_pos.y),
                dpdx(in.world_pos.z),
            ));
            let bitangent = normalize(cross(n, tangent));
            let perturbed = normalize(n + tangent * dhdu + bitangent * dhdv);
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

    let cam_pos = ssr_globals.view_pos.xyz;
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
    } else if (is_jade) {
        // Jade: warm green transmission through translucent nephrite.
        wrap = 0.45;
        sss_strength = 0.40;
        sss_tint = vec3<f32>(0.55, 0.92, 0.60);
    } else if (is_moonstone) {
        // Moonstone: exaggerated schiller — the inner glow reads almost
        // luminous, as if a cool light source were buried in the stone.
        // Schiller colour is derived from base_color so red tablets glow
        // red, green tablets green, etc. The saturated tint is the base
        // colour lifted toward its bright bloom point.
        wrap = 0.65;
        sss_strength = 0.60;
        sss_tint = mesh.base_color.rgb + vec3<f32>(0.30);
    } else if (is_pearl) {
        // Pearl: cool pink-white glow biased toward the base tint so
        // the gold "pearl" reads warm and the white pearl reads cool.
        wrap = 0.45;
        sss_strength = 0.30;
        sss_tint = mix(vec3<f32>(0.90, 0.85, 0.95), mesh.base_color.rgb, 0.40);
    } else if (is_goldnug) {
        // Polished gold: opaque conductor with a warm ambient glow so
        // unlit faces pick up enough light to read as reflective metal
        // rather than black silhouettes.
        wrap = 0.35;
        sss_strength = 0.20;
        sss_tint = vec3<f32>(1.0, 0.85, 0.45);
    } else if (is_poly) {
        // Polychrome: cool violet-pink glow.
        wrap = 0.45;
        sss_strength = 0.30;
        sss_tint = vec3<f32>(0.75, 0.55, 0.95);
    } else if (is_porcelain) {
        // Porcelain: soft warm wrap so the glazed ceramic reads as
        // lit-through rather than flat painted. Tint biases slightly
        // toward the base colour so coloured glazes stay coherent.
        wrap = 0.40;
        // Low `spec_strength` marks prop presets (e.g. shop abacus beads):
        // pull wrap energy down so tiny beads don't halo like wax.
        sss_strength = select(0.22, 0.10, mesh.material_params.y < 0.45);
        sss_tint = mix(vec3<f32>(1.00, 0.95, 0.90), mesh.base_color.rgb, 0.30);
    } else if (is_leather) {
        // Leather: warm wrap so the shadow side keeps a tinted bleed
        // rather than going pure black under the candle pools — real
        // leather scatters a touch of light through its surface dye.
        // Tint pulls toward the base colour so cordovan reads warm and
        // a tan-leather variant would read honey.
        wrap = 0.35;
        sss_strength = 0.28;
        sss_tint = mesh.base_color.rgb + vec3<f32>(0.18, 0.10, 0.04);
    } else if (is_felt) {
        // Felt: gentle wrap so the dark side of each candle pool
        // doesn't read as dead black. Real wool fibers scatter a small
        // amount of light through the surface — tinted slightly green
        // so the bleed feels like the felt's own colour rather than a
        // foreign halo. Strength stays low; felt is much less
        // translucent than wax or leather.
        wrap = 0.22;
        sss_strength = 0.14;
        sss_tint = vec3<f32>(0.10, 0.18, 0.08);
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
    let back_power = select(4.0, 6.0, is_talisman); // dense tablets get a tighter lobe
    var back_scale = 0.0;
    var back_tint = vec3<f32>(0.0);
    if (is_wax) {
        back_scale = 1.4;
        back_tint = vec3<f32>(1.00, 0.72, 0.32);
    } else if (is_jade) {
        back_scale = 0.9;
        back_tint = vec3<f32>(0.50, 0.88, 0.55);
    } else if (is_moonstone) {
        // Moonstone: silhouettes burn with a saturated adularescent glow
        // whose colour is driven by base_color — pushed well past plausible
        // so the stone reads as if lit by a coloured flame behind it.
        back_scale = 1.85;
        back_tint = mesh.base_color.rgb + vec3<f32>(0.35);
    } else if (is_pearl) {
        back_scale = 0.7;
        back_tint = mix(vec3<f32>(0.88, 0.82, 0.92), mesh.base_color.rgb, 0.45);
    } else if (is_goldnug) {
        // Polished gold: mostly opaque but a warm back-glow on edges
        // so the talisman reads as luminous rather than dead on the
        // shadow side.
        back_scale = 0.35;
        back_tint = vec3<f32>(1.0, 0.82, 0.40);
    } else if (is_poly) {
        back_scale = 0.9;
        back_tint = vec3<f32>(0.70, 0.50, 0.90);
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

    let pt_n_main = lights.count.x;
    for (var i: u32 = 0u; i < pt_n_main; i = i + 1u) {
        let pl = lights.lights[i];
        let lp = pl.pos.xyz;
        let range_w = pl.pos.w;
        let lc = pl.color.rgb;
        let is_inv = pl.params.x > 0.5;
        let intensity = pl.color.a * select(1.0, lights.extras.w, is_inv);
        let to_light = lp - in.world_pos;
        let dist = length(to_light);
        let atten = select(
            scene_smooth_point_atten(dist, range_w),
            punctual_attenuation_with_inv_doc_scale(dist, range_w, ssr_globals.shop_punctual.x),
            is_inv,
        );
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
        let cand_vis = mix(
            mix(0.18, 1.0, candle_occlusion(lp, in.world_pos, in.clip_pos.xy)),
            1.0,
            phys_hdr,
        );
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
            if (is_conductor) {
                // Conductor: Schlick Fresnel against the half-vector with
                // F0 = base colour. The reflected light then takes on the
                // metal's tint (no white "plastic" highlight) and swells
                // toward full reflectivity at glancing angles. Brass uses
                // the same lobe shape; a softer sheen is produced by the
                // caller picking a lower `specular_power`.
                let vdh = max(dot(view_dir, h), 0.0);
                let f0 = mesh.base_color.rgb;
                let f_metal = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vdh, 5.0);
                // Brass is slightly less reflective than "Metal" so the
                // warm body tone stays readable under museum lighting.
                let conductor_scale = select(1.0, 0.85, is_brass);
                spec_acc = spec_acc + lc * intensity * atten * s * cand_vis * f_metal * conductor_scale;
            } else if (is_enamel) {
                let vdh = max(dot(view_dir, h), 0.0);
                let ridge_f0 = mesh.base_color.rgb;
                let f_pin = ridge_f0 + (vec3<f32>(1.0) - ridge_f0) * pow(1.0 - vdh, 5.0);
                let ridge_lobe = pow(nh, max(spec_power * 1.8, 1.0)) * smoothstep(0.68, 0.92, enamel_height);
                spec_acc = spec_acc + lc * intensity * atten * s * cand_vis * 0.55;
                spec_acc = spec_acc + lc * intensity * atten * cand_vis * ridge_lobe * 1.15 * f_pin;
            } else if (is_glass) {
                let vdh = max(dot(view_dir, h), 0.0);
                let fresnel = 0.10 + 0.90 * pow(1.0 - vdh, 5.0);
                let glass_tint = mix(vec3<f32>(0.92, 0.97, 1.0), mesh.base_color.rgb, 0.35);
                spec_acc = spec_acc + lc * intensity * atten * s * cand_vis * fresnel * glass_tint * 1.35;
            } else if (is_porcelain) {
                // Porcelain glaze (chiclet/pillow look): a tight pinpoint
                // highlight sits inside a broader wet-glaze lobe, then a
                // punchy Fresnel rim wraps the silhouette so the shape
                // reads as a rounded candy-coated pebble. Dielectric
                // F0 ≈ 0.04; the wide lobe is what sells the "wet glaze".
                let vdh = max(dot(view_dir, h), 0.0);
                let ndv = max(dot(n, view_dir), 0.0);
                let fresnel = 0.04 + 0.96 * pow(1.0 - vdh, 5.0);
                let glaze_tint = vec3<f32>(1.0, 0.98, 0.96);
                let wide_lobe = pow(nh, max(spec_power * 0.35, 1.0)) * 0.55;
                let rim = 0.45 * pow(1.0 - ndv, 3.0);
                // Inside a crack the glaze surface is interrupted, so dip
                // the wet-glaze and rim terms (the pinpoint stays — the
                // crack walls themselves can still be glossy, only the
                // unbroken-glaze film is missing). 0.6 = full dip in the
                // line centre, 0 = pristine glaze.
                let glaze_break = 1.0 - 0.6 * crack_mask;
                // `spec_strength` scales the Blinn lobe (`s`) for the chiclet
                // pinch — these wet-glaze + silhouette rims did not, so tiny
                // props (e.g. abacus beads) blew out white. Normalize to the
                // default porcelain preset (~0.7) so dish-sized pieces match
                // their historical brightness when strength stays at 0.7.
                let glaze_k = mesh.material_params.y / 0.7;
                spec_acc = spec_acc + lc * intensity * atten * s * cand_vis * fresnel * glaze_tint * 1.55;
                spec_acc = spec_acc + lc * intensity * atten * wide_lobe * cand_vis * glaze_tint * 0.80 * glaze_break * glaze_k;
                spec_acc = spec_acc + lc * intensity * atten * cand_vis * rim * glaze_tint * glaze_break * glaze_k;
            } else if (is_leather) {
                let vdh = max(dot(view_dir, h), 0.0);
                let ndv = max(dot(n, view_dir), 0.0);
                if (in.uv.x > 1.5) {
                    // Page edges: matte paper, almost zero specular —
                    // just a hint of glance to keep them from looking
                    // dead flat under the candles.
                    let paper_lobe = pow(nh, 4.0) * 0.06;
                    spec_acc = spec_acc + lc * intensity * atten * cand_vis * paper_lobe * vec3<f32>(0.95, 0.92, 0.84);
                } else {
                    // Leather body: dielectric with a broad waxy sheen,
                    // no tight pinpoint. Two layered lobes — a soft
                    // hand-rubbed polish and a wider Fresnel-rim sheen
                    // at glancing angles — give cordovan its
                    // characteristic warm dull glow without ever
                    // picking up a glassy hotspot.
                    // Dielectric F0 ≈ 0.04, biased very slightly warm
                    // by the base tint so highlights pick up the
                    // leather hue.
                    let fresnel = 0.04 + 0.30 * pow(1.0 - vdh, 5.0);
                    let polish_tint = mix(vec3<f32>(1.0, 0.95, 0.85), mesh.base_color.rgb + vec3<f32>(0.4), 0.35);
                    // Soft polish lobe — wide, half-energy.
                    let soft_polish = pow(nh, max(spec_power, 1.0)) * 0.55;
                    spec_acc = spec_acc + lc * intensity * atten * cand_vis * soft_polish * fresnel * polish_tint;
                    // Sheen rim: glancing-angle bloom that reads as
                    // the waxed edge catching the candle. Independent
                    // of lobe sharpness so it shows up on flat panels
                    // too.
                    let sheen = pow(1.0 - ndv, 2.5) * pow(nh, 4.0) * 0.45;
                    spec_acc = spec_acc + lc * intensity * atten * cand_vis * sheen * polish_tint;
                }
            } else if (is_felt) {
                // ── Felt anisotropic sheen (Kajiya-Kay) ─────────────────
                // Real felt has a nap direction. Light raking across the
                // nap glows softly; light along it stays matte. We pick
                // world +Y as the nap axis (matches the anisotropic
                // frequency ratio used in felt_sample_world). The lobe
                // is sharpest perpendicular to the tangent — sin(θ)^k
                // around the tangent ring.
                //
                // Two perpendicular tangent samples (warp + weft) so the
                // weave catches light from both major axes — single-axis
                // Kajiya reads as "brushed metal" not "fabric."
                let nap_y = vec3<f32>(0.0, 1.0, 0.0);
                let nap_x = vec3<f32>(1.0, 0.0, 0.0);
                let t_warp = normalize(nap_y - n * dot(n, nap_y));
                let t_weft = normalize(nap_x - n * dot(n, nap_x));

                let tdh_w = dot(t_warp, h);
                let tdh_x = dot(t_weft, h);
                let sin_w = sqrt(max(1.0 - tdh_w * tdh_w, 0.0));
                let sin_x = sqrt(max(1.0 - tdh_x * tdh_x, 0.0));
                // Warp lobe (along nap) is broader; weft (across) is
                // tighter — felted wool isn't perfectly woven so the
                // two lobes are unequal.
                let aniso_w = pow(sin_w, max(spec_power * 6.0, 1.0)) * 0.65;
                let aniso_x = pow(sin_x, max(spec_power * 12.0, 1.0)) * 0.35;
                let aniso = aniso_w + aniso_x;

                // Schlick Fresnel — felt is a dielectric, F0 ≈ 0.04, but
                // the sheen rim swells aggressively at glancing angles.
                let vdh = max(dot(view_dir, h), 0.0);
                let fresnel = 0.04 + 0.96 * pow(1.0 - vdh, 5.0);
                // Slight green-yellow tint so the highlight feels like
                // it's picking up the felt's own colour rather than a
                // foreign white plastic glint.
                let sheen_tint = vec3<f32>(0.85, 1.05, 0.78);
                spec_acc = spec_acc + lc * intensity * atten * cand_vis
                         * aniso * fresnel * sheen_tint * spec_strength * 8.0;

                // ── Retroreflection rim ─────────────────────────────
                // Real felt fibers retroreflect light back toward the
                // viewer, which is why pool/poker baize looks slightly
                // lit-from-within at grazing angles. Modeled as a
                // view-dependent rim term — strongest where the surface
                // turns away from the camera. Tinted by the felt's own
                // sheen colour so the bloom feels like the fabric
                // glowing rather than a foreign halo.
                let ndv = max(dot(n, view_dir), 0.0);
                let retro = pow(1.0 - ndv, 4.0) * 0.55;
                spec_acc = spec_acc + lc * intensity * atten * cand_vis
                         * retro * sheen_tint * 0.6;
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

        // ── Per-talisman-kind sheen lobes ─────────────────────────────
        // View-dependent sheen layered on top of the base dielectric
        // specular. Each MaterialKind gets its own block so the math
        // can vary freely (lobe sharpness, Fresnel curve, tint source).
        if (is_talisman) {
            let h = normalize(l_dir + view_dir);
            let nh = max(dot(n, h), 0.0);
            let vdh = max(dot(view_dir, h), 0.0);
            let ndv = max(dot(n, view_dir), 0.0);
            // Broad wrap term: the diffuse half-vector alignment catches
            // light from a wide arc, not just the mirror direction.
            let broad = max(dot(n, l_dir), 0.0);

            if (is_jade) {
                // Jade: waxy vitreous luster — a broad green-tinted sheen
                // that strengthens at grazing angles.
                let fresnel = 0.08 + 0.30 * pow(1.0 - ndv, 2.5);
                let lobe = pow(nh, 12.0) * 0.6 + broad * 0.15;
                let tint = vec3<f32>(0.55, 0.95, 0.65);
                sheen_acc = sheen_acc + lc * intensity * atten * cand_vis * lobe * fresnel * tint;
            } else if (is_moonstone) {
                // Moonstone: three stacked lobes for a theatrical
                // schiller. Tight white pinpoint (surface glaze), a
                // wide coloured halo (adularescence), and a *very*
                // wide deep bloom that fills most of the lit
                // hemisphere — the stone looks like it's leaking
                // coloured light from under the surface. Halo and
                // bloom colours come from base_color so per-suit
                // tablets read red/green/etc. rather than always blue.
                let fresnel = 0.12 + 0.70 * pow(1.0 - ndv, 2.5);
                let pinpoint = pow(nh, 96.0) * 1.4;
                let schiller = pow(nh, 4.0) * 1.10;
                let bloom    = pow(nh, 1.5) * 0.55;
                let halo      = mesh.base_color.rgb + vec3<f32>(0.25);
                let deep_halo = mesh.base_color.rgb + vec3<f32>(0.15);
                sheen_acc = sheen_acc + lc * intensity * atten * cand_vis * (
                    pinpoint * fresnel * vec3<f32>(1.0) +
                    schiller * halo +
                    bloom    * deep_halo
                );
            } else if (is_pearl) {
                // Pearl: pearlescent nacre with pink-to-blue shift driven
                // by viewing angle. Tint biases toward the base colour so
                // a gold-tinted pearl reads warm and a white pearl reads
                // cool.
                let fresnel = 0.10 + 0.50 * pow(1.0 - ndv, 3.0);
                let phase = ndv * 3.14159;
                let pearl_white = vec3<f32>(
                    0.95 + 0.05 * cos(phase),
                    0.85 + 0.10 * cos(phase + 1.2),
                    0.90 + 0.10 * cos(phase + 2.8)
                );
                let pearl_tint = mix(pearl_white, mesh.base_color.rgb + vec3<f32>(0.20), 0.45);
                let lobe = pow(nh, 16.0) * 0.7 + broad * 0.20;
                sheen_acc = sheen_acc + lc * intensity * atten * cand_vis * lobe * fresnel * pearl_tint;
            } else if (is_goldnug) {
                // Polished gold: metallic conductor Schlick Fresnel with
                // a tight highlight lobe that reads as mirror-polished
                // sheet gold catching the candlelight.
                let f0 = vec3<f32>(1.0, 0.82, 0.36);
                let f_gold = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vdh, 5.0);
                let lobe = pow(nh, 64.0) * 1.8 + pow(nh, 12.0) * 0.35 + broad * 0.12;
                sheen_acc = sheen_acc + lc * intensity * atten * cand_vis * lobe * f_gold;
            } else if (is_poly) {
                // Polychrome: holographic thin-film iridescence — rainbow
                // hue driven by the normal-to-half angle so the spectrum
                // shifts as the light sweeps across.
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
        // Three stacked layers that, together, read as a shiny plastic
        // foil card pack: (1) a tight tinted mirror for the conductor
        // highlight, (2) anisotropic vertical streaks so the wrapper
        // picks up a brushed-foil look rather than a plastic gloss, and
        // (3) a view-swept holographic band that slides across the front
        // as the camera / pack angle changes — the signature "tilt it
        // in your hand" rainbow.
        if (is_foil) {
            let h = normalize(l_dir + view_dir);
            let nh = max(dot(n, h), 0.0);
            let vdh = max(dot(view_dir, h), 0.0);
            let ndv = max(dot(n, view_dir), 0.0);
            // Tinted conductor Fresnel — F0 is the foil's own colour so a
            // gold-tinted instance reflects gold, a silver one reflects
            // silver, etc. Real metallic foil wrappers take their sheen
            // from the metal itself, not a neutral spec.
            let f0 = mesh.base_color.rgb;
            let f_foil = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vdh, 5.0);

            // Anisotropic streak modulation. Real foil wrappers have
            // micro-ridges along the long axis of the pack; those ridges
            // smear the specular into vertical streaks rather than a
            // single round highlight. We fake it by modulating the
            // mirror lobe with a high-frequency cosine keyed to UV.x
            // (which runs across the width of the front face — see
            // build_pack_mesh). material_params.w > 0.5 marks the
            // talisman path, which has no UV art and shouldn't streak.
            var streak = 1.0;
            if (mesh.material_params.w <= 0.5) {
                let streak_wave = 0.5 + 0.5 * cos(in.uv.x * 64.0);
                // Gentle bias (0.55..1.35) so streaks add contrast
                // without darkening the wrapper overall.
                streak = mix(0.55, 1.35, streak_wave);
            }

            // Main mirror highlight — tighter and brighter than before so
            // the specular actually punches through the scene lighting.
            let mirror_lobe = pow(nh, 96.0) * 2.6 * streak;
            spec_acc = spec_acc + lc * intensity * atten * cand_vis * mirror_lobe * f_foil;

            // Broad diffuse-wrapped sheen — keeps the lit side of the
            // pack obviously brighter than the shadow side even when the
            // camera misses the mirror lobe.
            let broad_lobe = pow(nh, 16.0) * 0.8 * streak;
            spec_acc = spec_acc + lc * intensity * atten * cand_vis * broad_lobe * f_foil * 0.7;

            // View-swept holographic band. `ndv` goes 1→0 as we look
            // toward the grazing edge; combined with a per-light term it
            // forms a rainbow stripe that slides across the front face as
            // the pack tilts. Only fires on the decaled front (material
            // params.w == 0 path) — edges already carry the streaks.
            if (mesh.material_params.w <= 0.5) {
                let band_pos = ndv * 5.5 + in.uv.y * 2.8 + dot(n, l_dir) * 3.0;
                let band_r = 0.5 + 0.5 * cos(band_pos);
                let band_g = 0.5 + 0.5 * cos(band_pos + 2.094);
                let band_b = 0.5 + 0.5 * cos(band_pos + 4.189);
                let band_tint = vec3<f32>(band_r, band_g, band_b);
                // Band is strongest at glancing angles; stays visible but
                // reduced straight-on so the wrapper looks alive from any
                // viewing angle.
                let band_gain = (0.15 + 0.5 * pow(1.0 - ndv, 2.0)) * streak;
                let band_lobe = pow(nh, 18.0) * 0.55;
                sheen_acc = sheen_acc + lc * intensity * atten * cand_vis * band_lobe * band_gain * band_tint;
            }
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

    // ── Spotlights (same cone + falloff as `tile_3d.wgsl`) ─────────────
    // Unified shop path matches `shop_glb.wgsl` punctual attenuation and omits
    // the dual-buffer intensity product.
    let lm_punctual_scale = select(lights.extras.w, 1.0, phys_hdr > 0.5);
    let spot_count_fs = spot_lights.count.x;
    for (var si: u32 = 0u; si < spot_count_fs; si = si + 1u) {
        let s = spot_lights.lights[si];
        let lp = s.pos.xyz;
        let to_frag = in.world_pos - lp;
        let dist = length(to_frag);
        let radius = max(s.pos.w, 1.0);
        let t_sp = clamp(1.0 - dist / radius, 0.0, 1.0);
        let atten_sp = t_sp * t_sp;
        if (atten_sp <= 0.0) {
            continue;
        }
        let frag_dir = to_frag / max(dist, 0.0001);
        let cos_a = dot(frag_dir, s.dir.xyz);
        let spot_factor = khr_spot_angle_attenuation_scene(cos_a, s.params.x, s.dir.w);
        if (spot_factor <= 0.0) {
            continue;
        }
        let l_dir = -frag_dir;
        let ndl_raw = dot(n, l_dir);
        let nl = max(ndl_raw, 0.0);
        let lambert_sp = 0.35 + 0.65 * nl;
        let cand_vis_sp = mix(
            mix(0.18, 1.0, candle_occlusion(lp, in.world_pos, in.clip_pos.xy)),
            1.0,
            phys_hdr,
        );
        let sc = s.color.rgb * s.color.a * atten_sp * spot_factor * cand_vis_sp * lm_punctual_scale;
        lit = lit + sc * lambert_sp;

        if (sss_strength > 0.001) {
            let wrapped_sp = max((ndl_raw + wrap) / (1.0 + wrap), 0.0);
            let sss_band_sp = max(wrapped_sp - nl, 0.0);
            let sss_mask_sp = select(1.0, 1.0 - wood_pore * 0.7, is_wood);
            sss_acc = sss_acc + sc * sss_band_sp * sss_strength * sss_mask_sp;
        }

        if (back_scale > 0.001) {
            let lt_sp = normalize(l_dir + n * back_distortion);
            let back_sp = pow(max(dot(view_dir, -lt_sp), 0.0), back_power);
            back_acc = back_acc + sc * back_sp * back_thinness * back_scale;
        }

        if (spec_strength > 0.001) {
            let h_sp = normalize(l_dir + view_dir);
            let nh_sp = max(dot(n, h_sp), 0.0);
            let sharp_sp = pow(nh_sp, spec_power);
            let soft_sp = pow(nh_sp, max(spec_power * 0.5, 1.0)) * 0.10;
            var s_bl_sp = (sharp_sp + soft_sp) * spec_strength;
            if (is_wood) {
                s_bl_sp = s_bl_sp * mix(0.55, 1.15, wood_grain) * (1.0 - wood_pore * 0.85);
            }
            if (is_conductor) {
                let vdh_sp = max(dot(view_dir, h_sp), 0.0);
                let f0_sp = mesh.base_color.rgb;
                let f_metal_sp = f0_sp + (vec3<f32>(1.0) - f0_sp) * pow(1.0 - vdh_sp, 5.0);
                let conductor_scale_sp = select(1.0, 0.85, is_brass);
                spec_acc = spec_acc + sc * s_bl_sp * f_metal_sp * conductor_scale_sp;
            } else if (!is_felt) {
                spec_acc = spec_acc + sc * s_bl_sp;
            }
        }

        if (coat_strength > 0.001) {
            let h_ct = normalize(l_dir + view_dir);
            let nh_ct = max(dot(n, h_ct), 0.0);
            let vdh_ct = max(dot(view_dir, h_ct), 0.0);
            let f_schlick_ct = coat_f0 + (1.0 - coat_f0) * pow(1.0 - vdh_ct, 5.0);
            let d_ct = pow(nh_ct, coat_power) * (coat_power + 2.0) / 8.0;
            let coat_sp = d_ct * f_schlick_ct * coat_strength * (1.0 - wood_pore * 0.6);
            coat_acc = coat_acc + sc * coat_sp;
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
    if (is_brass) {
        // Brass fittings keep a bit more diffuse so shelf rails etc.
        // read as warm polished metal even outside specular angles —
        // otherwise the rails go near-black in overhead museum light.
        diffuse_scale = 0.22;
    }
    if (is_foil) {
        // Semi-metallic foil: more diffuse than a pure conductor (the
        // printed art needs to read) but less than a dielectric. The
        // specular + sheen carry the foil's shine.
        diffuse_scale = 0.45;
    }
    if (is_glass) {
        diffuse_scale = 0.18;
    }
    if (is_enamel) {
        diffuse_scale = 0.82;
    }
    if (is_moonstone) {
        // Moonstone: push diffuse even lower so the body sits dark and
        // lets the schiller/rim/SSS carry almost the entire lighting
        // signal. The gem reads as luminous rather than lit.
        diffuse_scale = 0.28;
    }
    if (is_goldnug) {
        // Polished gold conductor: low diffuse so the look is dominated
        // by the bright tinted Fresnel sheen, but enough to keep the
        // body warm and luminous rather than dark.
        diffuse_scale = 0.18;
    }
    // Gold-painted fragments inside carved decals are conductors: almost
    // all energy goes into the tinted Fresnel spec lobe, very little
    // diffuse. Lerp the diffuse scale down so gold reads as metallic.
    if (decal_metallic > 0.001) {
        diffuse_scale = mix(diffuse_scale, 0.12, decal_metallic);
    }
    let shop_vitrine_d = shop_vitrine_tuning;
    if (shop_vitrine_d && is_foil) {
        diffuse_scale = diffuse_scale * 0.58;
    }
    if (shop_vitrine_d && is_talisman) {
        diffuse_scale = diffuse_scale * 0.62;
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
    // Shop vitrine enamel: diffuse is already balanced vs foil via shadow floor + albedo tint;
    // skip extra `lit` knee so relics don't fall to black next to spec-heavy props.
    // Conductors: soften only extreme hot spec (metal props), not enough to kill readable highlights.
    if (shop_vitrine_tuning && is_conductor) {
        spec_acc = spec_acc / (vec3<f32>(1.0) + spec_acc * 0.07);
    }
    // Vitrine: foil packs + carved talismans keep extra spec/sheen lobes; pull them back so relic
    // enamel (diffuse-led) isn't the only thing that looks under-lit.
    let shop_vitrine = shop_vitrine_tuning;
    if (shop_vitrine && is_foil) {
        spec_acc = spec_acc * 0.22;
        sheen_acc = sheen_acc * 0.22;
    } else if (shop_vitrine && is_talisman) {
        spec_acc = spec_acc * 0.26;
        sheen_acc = sheen_acc * 0.26;
    }
    // ── Talisman Fresnel albedo tint ───────────────────────────────
    // View-dependent color shift baked into the surface albedo so it
    // reads as a material property (always visible), not just a specular
    // highlight that depends on perfect light alignment.
    if (is_talisman) {
        let edge = 1.0 - ndv_view;
        if (is_jade) {
            // Jade: edges brighten toward a cooler, lighter green.
            let rim = pow(edge, 2.0) * 0.25;
            albedo = mix(albedo, vec3<f32>(0.6, 1.0, 0.75), rim);
        } else if (is_moonstone) {
            // Moonstone: heavy rim pull toward a saturated, bright
            // version of the base colour. Tight falloff (high power)
            // + large mix amount means the silhouette burns bright
            // while the core stays gem-tinted — the piece reads as if
            // hollow with a coloured star inside.
            let rim = pow(edge, 1.2) * 0.90;
            let moon_rim = mesh.base_color.rgb + vec3<f32>(0.30);
            albedo = mix(albedo, moon_rim, rim);
        } else if (is_pearl) {
            // Pearl: edges shift toward soft pink/blue iridescence
            // overlaid onto the base tint.
            let rim = pow(edge, 1.8) * 0.35;
            let phase = ndv_view * 3.14159;
            let pearl_white = vec3<f32>(
                0.98 + 0.02 * cos(phase),
                0.88 + 0.06 * cos(phase + 1.5),
                0.95 + 0.05 * cos(phase + 3.0)
            );
            let pearl_edge = mix(pearl_white, mesh.base_color.rgb + vec3<f32>(0.18), 0.45);
            albedo = mix(albedo, pearl_edge, rim);
        } else if (is_goldnug) {
            // Polished gold: strong bright rim toward white-gold that
            // sells the shiny metallic conductor look. Subtle surface
            // variation keeps it from reading as flat plastic.
            let rim = pow(edge, 1.6) * 0.45;
            albedo = mix(albedo, vec3<f32>(1.0, 0.95, 0.72), rim);
            // Light surface variation — just enough to break up the flat
            // gold without making it look pitted or rough.
            let pit = noise3(in.local_pos * 14.0) * 0.7 + noise3(in.local_pos * 30.0) * 0.3;
            let pit_signed = pit - 0.5;
            albedo = albedo * (1.0 + pit_signed * 0.12);
        } else if (is_poly) {
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
        if (shop_vitrine) {
            albedo = albedo * 0.86;
        }
    }
    if (is_enamel) {
        let rim_gold = mix(vec3<f32>(0.92, 0.76, 0.28), mesh.base_color.rgb, 0.35);
        albedo = mix(albedo, rim_gold, enamel_ridge * 0.78);
        let edge = 1.0 - ndv_view;
        let glaze = pow(edge, 2.4) * 0.12;
        albedo = mix(albedo, albedo * 1.10 + vec3<f32>(0.04, 0.04, 0.05), glaze);
    }

    // Foil Fresnel edge tint: strong rainbow color-shift at grazing
    // angles so the wrapper catches iridescence even when no direct
    // light hits the mirror lobe. Combined with the lifted-tint albedo
    // and the per-light swept band, this is what reads as "glossy
    // holographic plastic" rather than a flat card.
    if (is_foil && mesh.material_params.w <= 0.5) {
        let edge = 1.0 - ndv_view;
        // Tint boost that runs from 0 dead-on to ~0.55 at silhouette.
        let rim = pow(edge, 2.0) * 0.32;
        let theta = ndv_view * 7.0 + in.uv.x * 3.5 + in.uv.y * 1.8;
        let holo = vec3<f32>(
            0.5 + 0.5 * cos(theta),
            0.5 + 0.5 * cos(theta + 2.094),
            0.5 + 0.5 * cos(theta + 4.189)
        );
        // Brighten the rim — foil wrappers catch ambient indirect even
        // on shadowed edges, which the per-light spec can't deliver.
        let rim_gain = mix(albedo, albedo * 0.6 + holo * 0.5, rim);
        albedo = rim_gain;
        if (shop_vitrine_tuning) {
            albedo = albedo * 0.84;
        }
    }
    if (is_glass) {
        let edge = 1.0 - ndv_view;
        let rim = pow(edge, 2.0) * 0.55;
        let cool_edge = vec3<f32>(0.82, 0.93, 1.0);
        albedo = mix(albedo, cool_edge, rim);
    }

    // Metal Fresnel sheen: view-dependent rim halo tinted toward warm
    // white-gold so the bronze mirror catches a soft edge glow even
    // outside specular angles. Independent of light direction so the
    // disc reads as polished bronze from every camera angle, not just
    // when a candle aligns with the half-vector.
    if (is_metal) {
        let edge = 1.0 - ndv_view;
        let rim = pow(edge, 3.0) * 0.55;
        let warm_edge = mix(vec3<f32>(1.0, 0.93, 0.72), vec3<f32>(1.0), 0.25);
        albedo = mix(albedo, warm_edge, rim);
    }
    if (is_brass) {
        // Brass rim halo: same warm-gold tint as Metal but a wider,
        // softer edge so shelf rails and fittings read as polished
        // brass from any camera angle, not just at grazing view.
        let edge = 1.0 - ndv_view;
        let rim = pow(edge, 2.0) * 0.40;
        let warm_edge = vec3<f32>(1.00, 0.88, 0.60);
        albedo = mix(albedo, warm_edge, rim);
    }
    if (is_leather && in.uv.x < 1.5) {
        // Leather rim: soft warm halo where the cover face curves away
        // from the camera, suggesting the polish lifts the edge tint
        // toward toasted-honey. Subtler than the brass rim — leather
        // is a dielectric, not a conductor. Skipped on page-edge
        // fragments — paper has its own aging tint above.
        let edge = 1.0 - ndv_view;
        let rim = pow(edge, 2.2) * 0.22;
        let warm_edge = mix(mesh.base_color.rgb + vec3<f32>(0.35, 0.20, 0.10), vec3<f32>(0.95, 0.78, 0.50), 0.45);
        albedo = mix(albedo, warm_edge, rim);
    }

    // Directional shadow map (mesh casters): PCF visibility in key-light
    // space. Analytic candle AABB occlusion (`cand_vis` above) still gates
    // each wick; this adds contact shadows from tall geometry onto the felt.
    let shop_vitrine_shadow = shop_vitrine_tuning;
    var lit_vitrine = lit;
    if (shop_vitrine_shadow && is_foil) {
        lit_vitrine = lit_vitrine * 0.40;
    }
    if (shop_vitrine_shadow && is_talisman) {
        lit_vitrine = lit_vitrine * 0.45;
    }
    var shadow_vis = sample_shadow_visibility(in.world_pos);
    if (shop_vitrine_shadow && is_enamel) {
        shadow_vis = max(shadow_vis, 0.58);
    }
    let lit_shadowed = lit_vitrine * shadow_vis;
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
        if (r.z > 0.02) {
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
                // world Z at the same screen pixel.
                if (p.z <= scene_world.z + stride * 0.6
                    && scene_world.z > in.world_pos.z + 0.1) {
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
                        if (pm.z <= sw.z && sw.z > in.world_pos.z + 0.1) {
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
    // Polychrome: HUD popups (score deltas, etc.) need to read as their
    // own base color regardless of scene lighting. The hand strip is only
    // lit by warm candle point lights, so albedo * lit multiplies out the
    // cool channels and a blue popup ends up orange. Add an unlit
    // emissive floor so the popup's tint dominates; the holographic sheen
    // and iridescent rim still layer on top as accents.
    //
    // Talisman tablets share MaterialKind::Polychrome but pass a lower
    // specular_power in material_params.z (~32); extruded glyphs use ~48.
    // Skip the emissive floor for tablets so shop tokens stay lit by the
    // scene instead of blowing out with bloom.
    var emissive = vec3<f32>(0.0);
    if (is_poly && mesh.material_params.z >= 40.0) {
        emissive = mesh.base_color.rgb * 0.85;
    }
    if (is_emissive) {
        emissive = emissive + mesh.base_color.rgb * spec_strength;
    }
    rgb = rgb
        + albedo * lit_shadowed * diffuse_scale
        + sss_acc * sss_tint
        + back_acc * back_tint
        + spec_final
        + coat_final
        + sheen_acc
        + emissive;

    let inv_g = 1.0 / max(lights.extras.x, 0.01);
    var out_rgb: vec3<f32>;
    if (phys_hdr > 0.5) {
        var amb = ssr_globals.felt.w * 0.08;
        if (ssr_globals.shop_punctual.y > 0.5 && is_enamel) {
            amb = ssr_globals.felt.w * 0.20;
        }
        var hdr = rgb + albedo * vec3<f32>(amb) * diffuse_scale;
        hdr = hdr * ssr_globals.felt.z;
        out_rgb = pow(aces_fitted(hdr), vec3<f32>(inv_g));
    } else {
        out_rgb = pow(rgb, vec3<f32>(inv_g));
    }

    let out_alpha = mesh.base_color.a;
    return vec4<f32>(out_rgb, out_alpha);
}
