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
    lights: array<PointLight, 8>,
};

@group(1) @binding(0) var<uniform> lights: PointLights;

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
    // Lighting is computed in LOCAL space so the result is independent of the
    // (non-uniform) per-tile model matrix.  The tile's front face is local +Y;
    // place the key light slightly off-axis from there for a soft top-light.
    let n_local = select(-normalize(in.local_n), normalize(in.local_n), front_facing);
    let light = normalize(vec3<f32>(0.25, 1.0, 0.35));
    let ndl = max(dot(n_local, light), 0.0);
    let ambient = 0.78;
    let diffuse = 0.30 * ndl;
    let shade = ambient + diffuse;

    // Front face = vertices whose local normal's dominant component is +Y
    // (the tile's flat top face after Z-up→Y-up conversion).  Use a soft
    // threshold so beveled-edge fragments still pick up the decal.
    let is_front = in.local_n.y > 0.0
        && in.local_n.y >= abs(in.local_n.x)
        && in.local_n.y >= abs(in.local_n.z);

    // Front face: use a flat ivory colour instead of the GLB's albedo so the
    // tile reads cleanly (the GLB texture has UV seams + dark patches that
    // smear across the top of the tile from a top-down view).  Side bevels
    // keep the original wood grain.
    let ivory = vec3<f32>(0.96, 0.93, 0.84);
    let tex_rgb = textureSample(base_color, base_sampler, in.uv).rgb
        * cam.base_color_factor.rgb;
    let base_rgb = select(tex_rgb, ivory, is_front) * shade;

    // Project decal UVs from model-space position onto the front face.
    // The mesh's long face axis is local X (extent 1.0, mapped to screen-vertical
    // by the renderer); local Z is the short axis (extent 0.734, screen-horizontal).
    // Decal U follows the on-screen horizontal (local Z) and V follows on-screen
    // vertical (local X), so the rasterised glyph appears upright on the tile.
    let decal_uv = vec2<f32>(in.local_pos.z * 1.362 + 0.5, in.local_pos.x + 0.5);
    let decal = textureSample(decal_tex, base_sampler, decal_uv);
    let in_uv = decal_uv.x >= 0.0 && decal_uv.x <= 1.0 && decal_uv.y >= 0.0 && decal_uv.y <= 1.0;
    let decal_a = select(0.0, decal.a, is_front && in_uv);
    let decal_rgb = decal.rgb * shade;
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

    let lit_rgb = rgb + rgb * point_contrib;
    return vec4<f32>(lit_rgb, 1.0);
}
