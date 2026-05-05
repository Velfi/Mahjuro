// Depth-only vertex shader for the shadow map pre-pass.
//
// Renders caster meshes from the directional key light's POV into a single
// 2048×2048 D32Float texture. Both the lit-mesh casters (table/dish/candles
// /relic boxes — though the table itself does not cast) and the hand-tile
// glb mesh share this shader because both vertex layouts start with
// `position : vec3<f32>` at offset 0; everything after that is ignored.
//
// One uniform per caster instance: light_view_proj * model. Written each
// frame from the renderer alongside the main-pass camera uniform.

struct ShadowCaster {
    light_view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> caster: ShadowCaster;

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return caster.light_view_proj * caster.model * vec4<f32>(pos, 1.0);
}
