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

// ACES tonemapping is applied once in `tonemap_composite.wgsl`. This shader
// writes linear HDR to `scene_color` (`Rgba16Float`).

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
    @location(4) local_pos: vec3<f32>,
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
    o.local_pos = pos;
    return o;
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    // base_color_factor.y encodes hover/select state:
    //   >= 0.75 && < 1.25 → selected (warm gold rim)
    //   ~0.5    → hovered (saturated blue rim)
    //   >= 1.25 → alternating hover/selected perimeter colors
    //   >= 1.90 && < 2.50 → decimation pick (crimson rim)
    //   >= 2.50 && < 3.50 → house claim (umber rim)
    let sel = in.sel_y;
    let is_combo = sel >= 1.25 && sel < 1.90;
    let is_decimation = sel >= 1.90 && sel < 2.50;
    let is_house_claim = sel >= 2.50 && sel < 3.50;
    let is_selected = sel >= 0.75 && sel < 1.25;
    let is_hovered = sel > 0.25 && sel < 0.75;
    let selected_color = vec3<f32>(1.00, 0.38, 0.02);
    let decimation_color = vec3<f32>(0.90, 0.12, 0.08);
    let house_color = vec3<f32>(0.62, 0.38, 0.28);
    let hovered_color = vec3<f32>(0.05, 0.40, 1.00);
    var base_color = vec3<f32>(0.72, 0.88, 1.00);
    if (is_decimation) {
        base_color = decimation_color;
    } else if (is_house_claim) {
        base_color = house_color;
    } else if (is_selected) {
        base_color = selected_color;
    } else if (is_hovered) {
        base_color = hovered_color;
    } else if (is_combo) {
        // Alternate two colors around the perimeter using local X/Z so top and
        // bottom edges are included (Y is tile thickness in local space).
        let ang = atan2(in.local_pos.z, in.local_pos.x);
        let u = fract((ang + 3.14159265) / (2.0 * 3.14159265));
        // Larger segments for readability at gameplay distance.
        let segment = floor(u * 16.0);
        let stripe = select(0.0, 1.0, fract(segment * 0.5) >= 0.5);
        base_color = mix(hovered_color, selected_color, stripe);
    }

    // Local-space directional ambient + diffuse so the rim has shape even
    // when no candles are lit. Mirrors tile_3d.wgsl's key-light direction.
    let n_local = select(-normalize(in.local_n), normalize(in.local_n), front_facing);
    let key = normalize(vec3<f32>(0.25, 1.0, 0.35));
    let ndl = max(dot(n_local, key), 0.0);
    var base_shade = 0.45 + 0.55 * ndl;
    if (is_decimation) {
        base_shade = 0.50 + 0.30 * ndl;
    } else if (is_selected) {
        base_shade = 0.72 + 0.33 * ndl;
    } else if (is_house_claim) {
        base_shade = 0.55 + 0.38 * ndl;
    } else if (is_hovered) {
        base_shade = 0.58 + 0.48 * ndl;
    } else if (is_combo) {
        base_shade = 0.70 + 0.40 * ndl;
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
        if (is_decimation) {
            // Keep the candle wash low so the crimson rim stays saturated
            // rather than blowing the red channel into ACES white-out.
            metal = 0.28 + 0.70 * pow(nl, 1.9);
        } else if (is_selected) {
            metal = 0.35 + 2.05 * pow(nl, 1.65);
        } else if (is_house_claim) {
            metal = 0.32 + 1.55 * pow(nl, 1.72);
        } else if (is_hovered) {
            metal = 0.32 + 1.85 * pow(nl, 1.72);
        } else if (is_combo) {
            metal = 0.34 + 1.95 * pow(nl, 1.68);
        }
        contrib = contrib + lc * intensity * atten * metal;
    }

    var lit = base_color * base_shade + base_color * contrib;
    // Emissive + chroma punch so ACES/HDR does not wash rims to grey/white.
    lit = lit + select(vec3<f32>(0.0), base_color * 0.16, is_hovered);
    lit = lit + select(vec3<f32>(0.0), base_color * 0.26, is_selected);
    lit = lit + select(vec3<f32>(0.0), base_color * 0.20, is_decimation);
    lit = lit + select(vec3<f32>(0.0), base_color * 0.18, is_house_claim);
    lit = lit + select(vec3<f32>(0.0), base_color * 0.23, is_combo);
    if (is_decimation) {
        let luma = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
        let sat = 2.10;
        lit = vec3<f32>(luma) + (lit - vec3<f32>(luma)) * sat;
        lit = max(lit, vec3<f32>(0.0));
    } else if (is_selected) {
        let luma = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
        let sat = 1.75;
        lit = vec3<f32>(luma) + (lit - vec3<f32>(luma)) * sat;
        lit = max(lit, vec3<f32>(0.0));
    } else if (is_house_claim) {
        let luma = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
        let sat = 1.35;
        lit = vec3<f32>(luma) + (lit - vec3<f32>(luma)) * sat;
        lit = max(lit, vec3<f32>(0.0));
    } else if (is_hovered) {
        let luma = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
        let sat = 1.58;
        lit = vec3<f32>(luma) + (lit - vec3<f32>(luma)) * sat;
        lit = max(lit, vec3<f32>(0.0));
    } else if (is_combo) {
        let luma = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
        let sat = 1.66;
        lit = vec3<f32>(luma) + (lit - vec3<f32>(luma)) * sat;
        lit = max(lit, vec3<f32>(0.0));
    }
    var gain = 1.0;
    if (is_selected) {
        gain = 5.52;
    } else if (is_decimation) {
        gain = 1.38;
    } else if (is_house_claim) {
        gain = 1.12;
    } else if (is_hovered) {
        gain = 4.80;
    } else if (is_combo) {
        gain = 5.28;
    }
    lit = lit * gain;
    let inv_g = 1.0 / max(lights.extras.x, 0.01);
    var out_rgb: vec3<f32>;
    if (outline_frame.hdr_tonemap.x > 0.5) {
        // Linear HDR path: write the un-tonemapped HDR; `tonemap_composite.wgsl`
        // applies the single ACES pass + sRGB encode (per-shader `lights.extras.x`
        // gamma slider is intentionally a no-op here).
        var hdr = lit + outline_frame.hdr_tonemap.z * base_color * vec3<f32>(0.08);
        hdr = hdr * outline_frame.hdr_tonemap.y;
        out_rgb = hdr;
    } else {
        out_rgb = pow(lit, vec3<f32>(inv_g));
    }
    return vec4<f32>(out_rgb, 1.0);
}
