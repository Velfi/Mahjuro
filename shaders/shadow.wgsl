// Depth-only vertex shader for the shadow map pre-pass.
//
// Renders caster meshes from the directional key light's POV into a single
// 2048×2048 D32Float texture. Lit-mesh casters (table/dish/candles/relics —
// the table felt does not cast), imported room GLB primitives (`shop.glb`,
// hallway, archive, main menu), and hand-tile glb meshes share this shader
// because all vertex layouts start with `position : vec3<f32>` at offset 0;
// everything after that is ignored.
//
// `hallway_vertex_warp.wgsl` is prepended in `embedded_wgsl::SHADOW` so room
// env depth matches the lit pass when pick-blind vertex warp is active.
// Group 1 binds the same `HallwayDistortion` bytes as `room_glb` @binding(8);
// zeroed buffer ⇒ `flags.x` = 0 ⇒ no warp (tiles, props, shop, etc.).

struct ShadowCaster {
    light_view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> caster: ShadowCaster;
@group(1) @binding(0) var<uniform> hd: HallwayDistortion;

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    let world_h = (caster.model * vec4<f32>(pos, 1.0)).xyz;
    let world = select(world_h, apply_hallway_distortion(world_h, hd), hd.flags.x > 0.5);
    return caster.light_view_proj * vec4<f32>(world, 1.0);
}
