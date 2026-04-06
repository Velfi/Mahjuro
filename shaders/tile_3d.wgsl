struct CameraUniform {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    base_color_factor: vec4<f32>,
};

@group(0) @binding(0) var<uniform> cam: CameraUniform;
@group(0) @binding(1) var base_color: texture_2d<f32>;
@group(0) @binding(2) var base_sampler: sampler;
@group(0) @binding(3) var decal_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) wn: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) local_pos: vec3<f32>,
    @location(3) local_n: vec3<f32>,
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
    return o;
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    // Two-sided lighting: flip the normal for back-facing fragments so both
    // sides of the tile receive correct illumination.
    let n = select(-normalize(in.wn), normalize(in.wn), front_facing);
    let light = normalize(vec3<f32>(0.38, 0.82, 0.42));
    let ndl = max(dot(n, light), 0.0);
    let ambient = 0.18;
    let diffuse = 0.82 * ndl;
    let shade = ambient + diffuse;

    let albedo = textureSample(base_color, base_sampler, in.uv);
    let base_rgb = albedo.rgb * cam.base_color_factor.rgb * shade;

    // Project decal UVs from model-space position onto the front face (+Y normal).
    // After normalization the tile spans X[-0.5..0.5], Z[-0.37..0.37] on the front face.
    let decal_uv = vec2<f32>(in.local_pos.x + 0.5, -in.local_pos.z * 1.37 + 0.5);
    let decal = textureSample(decal_tex, base_sampler, decal_uv);
    let is_front = in.local_n.y > 0.5;
    let decal_a = select(0.0, decal.a, is_front);
    let decal_rgb = decal.rgb * shade;
    let rgb = mix(base_rgb, decal_rgb, decal_a);

    let a = albedo.a * cam.base_color_factor.a;
    return vec4<f32>(rgb, a);
}
