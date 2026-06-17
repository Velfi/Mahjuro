// UI outline shell for focused/selected tiles.
//
// The renderer draws this pipeline BEFORE the normal tile mesh, with the
// model matrix scaled up ~7% around the tile center and front-face culling
// enabled. Only the back side of the inflated shell survives, so when the
// real tile is drawn on top it overwrites the interior of the shell,
// leaving a thin UI rim around the tile silhouette.

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
    // extras.x = reserved (display gamma; outline writes HDR, tonemap is composite).
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
    @location(10) normal_c0: vec4<f32>,
    @location(11) normal_c1: vec4<f32>,
    @location(12) normal_c2: vec4<f32>,
    @location(13) inst_base_color_factor: vec4<f32>,
) -> VsOut {
    let model = mat4x4<f32>(model_c0, model_c1, model_c2, model_c3);
    let normal_model = mat3x3<f32>(normal_c0.xyz, normal_c1.xyz, normal_c2.xyz);
    let world = model * vec4<f32>(pos, 1.0);
    var o: VsOut;
    o.clip_pos = outline_frame.view_proj * world;
    o.wn = normalize(normal_model * n);
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

    // Linear HDR: write the selected shell color directly; `tonemap_composite.wgsl`
    // applies ACES + sRGB once for the whole scene.
    let hdr = base_color;
    // Clamp to prevent Rgba16Float overflow (Infinity) which causes NaN during bloom bilinear filtering on Metal
    let out_rgb = min(hdr, vec3<f32>(65000.0));
    return vec4<f32>(out_rgb, 1.0);
}
