// Gold-metal "shell" outline for selected tiles.
//
// The renderer draws this pipeline BEFORE the normal tile mesh, with the
// model matrix scaled up ~7% around the tile center and front-face culling
// enabled. Only the back side of the inflated shell survives, so when the
// real tile is drawn on top it overwrites the interior of the shell —
// leaving a thin gold rim around the tile silhouette. The fragment shader
// uses the same candle point-light buffer the tile shader uses, so the
// outline visibly catches candlelight as candles flicker around the table.

struct OutlineFrame {
    view_proj: mat4x4<f32>,
    hdr_tonemap: vec4<f32>,
}

// `aces_fitted` — see `scene_hdr_tonemap.wgsl` (prepended at pipeline creation).

@group(0) @binding(0) var<uniform> outline_frame: OutlineFrame;

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

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) wn: vec3<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) local_n: vec3<f32>,
    @location(3) sel_y: f32,
};

@vertex
fn vs_main(
    @location(0) pos: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) _tangent_pad: vec4<f32>,
    @location(4) _uv_emr_pad: vec2<f32>,
    @location(5) _color_pad: vec4<f32>,
    @location(6) model_c0: vec4<f32>,
    @location(7) model_c1: vec4<f32>,
    @location(8) model_c2: vec4<f32>,
    @location(9) model_c3: vec4<f32>,
    @location(10) inst_base_color_factor: vec4<f32>,
) -> VsOut {
    let model = mat4x4<f32>(model_c0, model_c1, model_c2, model_c3);
    let world = model * vec4<f32>(pos, 1.0);
    var o: VsOut;
    o.clip_pos = outline_frame.view_proj * world;
    o.wn = normalize((model * vec4<f32>(n, 0.0)).xyz);
    o.world_pos = world.xyz;
    o.local_n = n;
    o.sel_y = inst_base_color_factor.y;
    return o;
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    // base_color_factor.y encodes hover/select state:
    //   >= 0.75 → selected (warm gold rim)
    //   ~0.5    → hovered (saturated blue rim)
    let sel = in.sel_y;
    let is_selected = sel >= 0.75;
    let is_hovered = sel > 0.25 && sel < 0.75;
    var base_color = vec3<f32>(0.72, 0.88, 1.00);
    if (is_selected) {
        base_color = vec3<f32>(1.00, 0.38, 0.02);
    } else if (is_hovered) {
        base_color = vec3<f32>(0.05, 0.40, 1.00);
    }

    // Local-space directional ambient + diffuse so the rim has shape even
    // when no candles are lit. Mirrors tile_3d.wgsl's key-light direction.
    let n_local = select(-normalize(in.local_n), normalize(in.local_n), front_facing);
    let key = normalize(vec3<f32>(0.25, 1.0, 0.35));
    let ndl = max(dot(n_local, key), 0.0);
    var base_shade = 0.45 + 0.55 * ndl;
    if (is_selected) {
        base_shade = 0.72 + 0.33 * ndl;
    } else if (is_hovered) {
        base_shade = 0.58 + 0.48 * ndl;
    }

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
        var metal = 0.30 + 1.60 * pow(nl, 1.8);
        if (is_selected) {
            metal = 0.35 + 2.05 * pow(nl, 1.65);
        } else if (is_hovered) {
            metal = 0.32 + 1.85 * pow(nl, 1.72);
        }
        contrib = contrib + lc * intensity * atten * metal;
    }

    var lit = base_color * base_shade + base_color * contrib;
    // Emissive + chroma punch so ACES/HDR does not wash rims to grey/white.
    lit = lit + select(vec3<f32>(0.0), base_color * 0.16, is_hovered);
    lit = lit + select(vec3<f32>(0.0), base_color * 0.26, is_selected);
    if (is_selected) {
        let luma = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
        let sat = 1.75;
        lit = vec3<f32>(luma) + (lit - vec3<f32>(luma)) * sat;
        lit = max(lit, vec3<f32>(0.0));
    } else if (is_hovered) {
        let luma = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
        let sat = 1.58;
        lit = vec3<f32>(luma) + (lit - vec3<f32>(luma)) * sat;
        lit = max(lit, vec3<f32>(0.0));
    }
    var gain = 1.0;
    if (is_selected) {
        gain = 1.38;
    } else if (is_hovered) {
        gain = 1.20;
    }
    lit = lit * gain;
    let inv_g = 1.0 / max(lights.extras.x, 0.01);
    var out_rgb: vec3<f32>;
    if (outline_frame.hdr_tonemap.x > 0.5) {
        var hdr = lit + outline_frame.hdr_tonemap.z * base_color * vec3<f32>(0.08);
        hdr = hdr * outline_frame.hdr_tonemap.y;
        out_rgb = pow(aces_fitted(hdr), vec3<f32>(inv_g));
    } else {
        out_rgb = pow(lit, vec3<f32>(inv_g));
    }
    return vec4<f32>(out_rgb, 1.0);
}
