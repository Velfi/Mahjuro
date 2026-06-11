// Bake-only room shadow capture pass.
//
// Renders imported room GLB primitives with the same position transform as the
// depth-only shadow pass, but also writes per-primitive receiver/occluder
// classification and world normals into Rgba8Unorm masks.
// `hallway_vertex_warp.wgsl` is prepended by `embedded_wgsl::ROOM_SHADOW_MASK`.

struct ShadowCaster {
    light_view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    normal_model: mat4x4<f32>,
};

struct RoomShadowMask {
    params: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_n: vec3<f32>,
};

struct FsOut {
    @location(0) class_mask: vec4<f32>,
    @location(1) normal_mask: vec4<f32>,
};

@group(0) @binding(0) var<uniform> caster: ShadowCaster;
@group(1) @binding(0) var<uniform> hd: HallwayDistortion;
@group(2) @binding(0) var<uniform> mask: RoomShadowMask;

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) n: vec3<f32>) -> VsOut {
    let world_h = (caster.model * vec4<f32>(pos, 1.0)).xyz;
    let world = select(world_h, apply_hallway_distortion(world_h, hd), hd.flags.x > 0.5);
    var out: VsOut;
    out.clip_pos = caster.light_view_proj * vec4<f32>(world, 1.0);
    out.world_n = normalize((caster.normal_model * vec4<f32>(n, 0.0)).xyz);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> FsOut {
    let n = normalize(in.world_n);
    var out: FsOut;
    out.class_mask = vec4<f32>(
        clamp(mask.params.x, 0.0, 1.0),
        clamp(mask.params.y, 0.0, 1.0),
        clamp(mask.params.z, 0.0, 1.0),
        clamp(mask.params.w, 0.0, 1.0),
    );
    out.normal_mask = vec4<f32>(n * 0.5 + vec3<f32>(0.5), 1.0);
    return out;
}
