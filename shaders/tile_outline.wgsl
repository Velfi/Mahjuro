// Gold-metal "shell" outline for selected tiles.
//
// The renderer draws this pipeline BEFORE the normal tile mesh, with the
// model matrix scaled up ~6% around the tile center and front-face culling
// enabled. Only the back side of the inflated shell survives, so when the
// real tile is drawn on top it overwrites the interior of the shell —
// leaving a thin gold rim around the tile silhouette. The fragment shader
// uses the same candle point-light buffer the tile shader uses, so the
// outline visibly catches candlelight as candles flicker around the table.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    base_color_factor: vec4<f32>,
};

@group(0) @binding(0) var<uniform> cam: CameraUniform;
// bindings 1..3 (textures + sampler) exist on the shared layout but are
// unused here — the outline is a flat-coloured metal, not a textured tile.

struct PointLight {
    pos: vec4<f32>,
    color: vec4<f32>,
};
struct PointLights {
    count: vec4<u32>,
    // extras.x = display gamma exponent; rest reserved.
    extras: vec4<f32>,
    lights: array<PointLight, 16>,
};
@group(1) @binding(0) var<uniform> lights: PointLights;

// Group 2 (shadow sampling) bindings exist on the shared tile pipeline
// layout. The outline shell is a thin gold rim drawn before the real
// tile mesh — it doesn't sample the shadow map (the rim is mostly
// occluded by the tile itself), but the bindings must be declared so
// the pipeline layout matches.
struct ShadowGlobals {
    light_view_proj: mat4x4<f32>,
    params: vec4<f32>,
};
@group(2) @binding(0) var<uniform> shadow_globals: ShadowGlobals;
@group(2) @binding(1) var shadow_map: texture_depth_2d;
@group(2) @binding(2) var shadow_samp: sampler_comparison;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) wn: vec3<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) local_n: vec3<f32>,
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
    o.world_pos = world.xyz;
    o.local_n = n;
    return o;
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    // Polished champagne gold base — warm, slightly desaturated so the
    // candle highlights read as specular instead of saturating to orange.
    let gold = vec3<f32>(1.00, 0.78, 0.34);

    // Local-space directional ambient + diffuse so the rim has shape even
    // when no candles are lit. Mirrors tile_3d.wgsl's key-light direction.
    let n_local = select(-normalize(in.local_n), normalize(in.local_n), front_facing);
    let key = normalize(vec3<f32>(0.25, 1.0, 0.35));
    let ndl = max(dot(n_local, key), 0.0);
    let base_shade = 0.45 + 0.55 * ndl;

    // Point-light pass: same falloff/Lambert model the tile shader uses,
    // but with a sharper exponent so the gold reads as polished metal —
    // candle facets near a flame light up much more than the diffuse body.
    var n_world = normalize(in.wn);
    if (!front_facing) {
        n_world = -n_world;
    }
    var contrib = vec3<f32>(0.0);
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
        // Diffuse floor + sharper specular-ish term for the metallic look.
        let metal = 0.30 + 1.60 * pow(nl, 1.8);
        contrib = contrib + lc * intensity * atten * metal;
    }

    // Reference shadow bindings so naga keeps the group 2 layout entries
    // and the pipeline layout matches the lit-tile pipeline. The actual
    // sample is force-mixed at zero strength so it never affects the
    // outline color.
    let shadow_uv = vec2<f32>(0.5, 0.5);
    let shadow_dummy = textureSampleCompare(shadow_map, shadow_samp, shadow_uv, 1.0);
    let shadow_keep = shadow_globals.params.x * 0.0 + shadow_dummy * 0.0;

    let lit = gold * base_shade + gold * contrib;
    let inv_g = 1.0 / max(lights.extras.x, 0.01);
    let out_rgb = pow(lit, vec3<f32>(inv_g)) + vec3<f32>(shadow_keep);
    return vec4<f32>(out_rgb, 1.0);
}
