struct CameraUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    base_color_factor: vec4<f32>,
};

@group(0) @binding(0) var<uniform> cam: CameraUniform;
@group(0) @binding(1) var base_color: texture_2d<f32>;
@group(0) @binding(2) var base_sampler: sampler;
@group(0) @binding(3) var decal_tex: texture_2d<f32>;

struct PointLight {
    // xyz = position in screen-pixel space (z is unused; we treat the table
    // as a flat plane), w = falloff radius in pixels.
    pos: vec4<f32>,
    // rgb = linear colour, a = intensity multiplier.
    color: vec4<f32>,
};

struct PointLights {
    // count.x = number of active lights; rest is std140 padding.
    count: vec4<u32>,
    // extras.x = display gamma exponent; rest reserved.
    extras: vec4<f32>,
    lights: array<PointLight, 16>,
};

@group(1) @binding(0) var<uniform> lights: PointLights;

// ── Shadow sampling (group 2, shared frame-wide) ─────────────────────
struct ShadowGlobals {
    light_view_proj: mat4x4<f32>,
    // x = enabled (0/1), y = depth bias, z = texel size, w = unused
    params: vec4<f32>,
};
@group(2) @binding(0) var<uniform> shadow_globals: ShadowGlobals;
@group(2) @binding(1) var shadow_map: texture_depth_2d;
@group(2) @binding(2) var shadow_samp: sampler_comparison;

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

// ── Procedural bamboo helpers ───────────────────────────────────────────
// Cheap value-noise primitives used to build a bamboo wood-fiber texture
// for the tile sides. Bamboo's defining traits are (a) long parallel
// fibers running along the cane's growth axis — visible as fine vertical
// striations on side faces — and (b) circular fiber cross-sections on
// end-grain faces, which read as a cluster of darker dots in a lighter
// matrix. Each tile-mesh fiber runs along local X (the tile's long axis).
fn hash21(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453);
}

fn vnoise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Returns a bamboo albedo for a fragment whose surface lies on the tile
// body. `local_pos` is in normalised mesh space (X≈[-0.5,0.5] long axis,
// Y≈[-0.21,0.21] thickness, Z≈[-0.37,0.37] short axis). `local_n` is the
// raw mesh normal so we can detect end-grain vs side-grain faces.
fn bamboo_albedo(local_pos: vec3<f32>, local_n: vec3<f32>) -> vec3<f32> {
    // Warm cream fiber colour and a deeper amber for the matrix between
    // fibers / for end-grain. Tuned to sit slightly cooler than the ivory
    // tile face so the bamboo body reads as a separate material.
    let fiber_light = vec3<f32>(0.93, 0.83, 0.55);
    let fiber_dark  = vec3<f32>(0.66, 0.50, 0.26);
    let end_base    = vec3<f32>(0.46, 0.32, 0.16);

    // ── Side-grain (±Y and ±Z faces) ────────────────────────────────────
    // Fibers run along local X. The "across-fiber" coordinate is whichever
    // tangent axis isn't X — Z on top/bottom faces, Y on left/right faces.
    let across = select(local_pos.y, local_pos.z, abs(local_n.y) > abs(local_n.z));

    // Stretch the noise *heavily* along the fiber direction so each
    // value-noise cell becomes a long thin streak — that's the parallel
    // fiber look. Two octaves give the fibers some thickness variance.
    let n1 = vnoise2(vec2<f32>(local_pos.x * 4.0, across * 70.0));
    let n2 = vnoise2(vec2<f32>(local_pos.x * 1.5, across * 140.0));
    let fiber_pattern = n1 * 0.65 + n2 * 0.35;

    // A faint sinusoidal striping reinforces the long-grain feel between
    // noise cells, especially under raking candlelight.
    let stripes = sin(across * 220.0) * 0.5 + 0.5;
    let side_t = clamp(fiber_pattern * 0.75 + stripes * 0.25, 0.0, 1.0);

    // Subtle along-length tonal variation (warmer/cooler bands).
    let length_var = vnoise2(vec2<f32>(local_pos.x * 6.0, across * 4.0));
    var side_rgb = mix(fiber_dark, fiber_light, side_t);
    side_rgb = side_rgb * (0.92 + 0.16 * length_var);

    // ── End-grain (±X faces) ────────────────────────────────────────────
    // Cross-section through the fibers: cluster of darker circular fiber
    // bundles in a lighter matrix. Use 2D noise on (Y, Z) at high
    // frequency, then sharpen with a power curve to make the dark cores
    // sparse and well-defined.
    let end_n = vnoise2(vec2<f32>(local_pos.y * 110.0, local_pos.z * 110.0));
    let end_n2 = vnoise2(vec2<f32>(local_pos.y * 45.0 + 7.3, local_pos.z * 45.0 - 2.1));
    let dots = pow(end_n, 2.6);
    var end_rgb = mix(end_base * 1.55, end_base * 0.55, dots);
    // Slight large-scale warmth variation across the end face.
    end_rgb = end_rgb * (0.90 + 0.20 * end_n2);

    // Blend the two based on how end-grain the surface is. Use a soft
    // smoothstep so beveled corners get a believable transition rather
    // than a hard seam.
    let end_grain = smoothstep(0.55, 0.92, abs(local_n.x));
    return mix(side_rgb, end_rgb, end_grain);
}

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) wn: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) local_pos: vec3<f32>,
    @location(3) local_n: vec3<f32>,
    @location(4) world_pos: vec3<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
) -> VsOut {
    let world = cam.model * vec4<f32>(pos, 1.0);
    var o: VsOut;
    o.clip_pos = cam.view_proj * world;
    o.wn = normalize((cam.model * vec4<f32>(n, 0.0)).xyz);
    o.uv = uv;
    o.local_pos = pos;
    o.local_n = n;
    o.world_pos = world.xyz;
    return o;
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    // Candle-only lighting: no ambient floor, no directional key. The
    // tile's base albedo is preserved (not multiplied by any global
    // shade) and the only illumination comes from the wick PointLights
    // accumulated below. Fragments outside any candle pool stay black.

    // Front face = vertices whose local normal's dominant component is +Y
    // (the tile's flat top face after Z-up→Y-up conversion).  Use a soft
    // threshold so beveled-edge fragments still pick up the decal.
    let is_front = in.local_n.y > 0.0
        && in.local_n.y >= abs(in.local_n.x)
        && in.local_n.y >= abs(in.local_n.z);

    // Real mahjong tiles are a thin ivory/bone face layer glued onto a
    // bamboo body — the ivory wraps around the top of the side bevels
    // for a couple of millimetres before the bamboo grain takes over.
    // The mesh's local Y extent is ~[-0.212, 0.212] (≈ 16 mm of tile
    // thickness), so a 0.040 ivory band represents ~3 mm of ivory.
    // Sides that are above the threshold but not the +Y top face become
    // ivory; everything below is bamboo. The bottom (-Y) face has
    // local_n.y < 0 and is excluded so it stays bamboo.
    let ivory_layer_y: f32 = 0.172; // band starts ~3 mm below the top
    let ivory_band_softness: f32 = 0.012; // smooth transition (~1 mm)
    let ivory_band = smoothstep(
        ivory_layer_y - ivory_band_softness,
        ivory_layer_y + ivory_band_softness,
        in.local_pos.y,
    ) * select(0.0, 1.0, in.local_n.y > -0.25);

    // Front face: flat ivory so the decal reads cleanly. Body (bevels and
    // bottom + ends): procedural bamboo wood-fiber so the tile sides look
    // like real cane — long parallel fibers on the long sides, darker
    // cross-section dots on the short ends. The GLB albedo is intentionally
    // bypassed: its baked UV seams smeared across the tile under our
    // top-down camera and never matched real bamboo.
    let ivory = vec3<f32>(0.96, 0.93, 0.84);
    let bamboo = bamboo_albedo(in.local_pos, normalize(in.local_n));
    let body_rgb = mix(bamboo, ivory, ivory_band);
    let base_rgb = select(body_rgb, ivory, is_front);

    // Project decal UVs from model-space position onto the front face.
    // The mesh's long face axis is local X (extent 1.0, mapped to screen-vertical
    // by the renderer); local Z is the short axis (extent 0.734, screen-horizontal).
    // Decal U follows the on-screen horizontal (local Z) and V follows on-screen
    // vertical (local X), so the rasterised glyph appears upright on the tile.
    let decal_uv = vec2<f32>(in.local_pos.z * 1.362 + 0.5, in.local_pos.x + 0.5);
    let decal = textureSample(decal_tex, base_sampler, decal_uv);
    let in_uv = decal_uv.x >= 0.0 && decal_uv.x <= 1.0 && decal_uv.y >= 0.0 && decal_uv.y <= 1.0;
    let decal_a = select(0.0, decal.a, is_front && in_uv);
    let decal_rgb = decal.rgb;
    let rgb = mix(base_rgb, decal_rgb, decal_a);

    // ── Point-light pass ────────────────────────────────────────────────
    // Accumulate candle / point-light contributions on top of the base
    // shade. Each light uses a smooth quadratic falloff plus a Lambertian
    // term against the world-space normal so the side bevel facing a candle
    // gets the most warmth. Lighting tints existing colour (rgb * contrib)
    // instead of overwriting it, so the tile's albedo still reads through.
    var n_world = normalize(in.wn);
    if (!front_facing) {
        n_world = -n_world;
    }
    var point_contrib = vec3<f32>(0.0);
    let light_count = lights.count.x;
    for (var i: u32 = 0u; i < light_count; i = i + 1u) {
        let lp = lights.lights[i].pos.xyz;
        let radius = lights.lights[i].pos.w;
        let lc = lights.lights[i].color.rgb;
        let intensity = lights.lights[i].color.a;
        let to_light = lp - in.world_pos;
        let dist = length(to_light);
        let t = clamp(1.0 - dist / max(radius, 1.0), 0.0, 1.0);
        let atten = t * t;
        let l_dir = to_light / max(dist, 0.0001);
        let nl = max(dot(n_world, l_dir), 0.0);
        // 0.35 ambient floor so even back-facing fragments warm up a little
        // (matches how a real candle bounces off the table around a tile).
        let lambert = 0.35 + 0.65 * nl;
        point_contrib = point_contrib + lc * intensity * atten * lambert;
    }

    // Candle-only composition: tile albedo modulated purely by the
    // accumulated point-light contribution. No directional shadow
    // attenuation — the wicks are the only lights in the scene.
    let lit_rgb = rgb * point_contrib;
    let inv_g = 1.0 / max(lights.extras.x, 0.01);
    let out_rgb = pow(lit_rgb, vec3<f32>(inv_g));
    return vec4<f32>(out_rgb, 1.0);
}
