//! Canonical WGSL composition lists for the room shaders that feed offline bakes.
//!
//! Single source of truth for *which* `.wgsl` files are concatenated into each
//! shader program the bakes depend on. The runtime embeds these same files via
//! the `concat!(include_str!(...))` macros in
//! `mahjuro_render::wgpu_renderer::embedded_wgsl`, and the room shadow / GI stamp
//! input lists derive from the consts here. A drift test in that module asserts
//! the embedded composition still matches these lists, so a prepended dependency
//! can no longer be added to the shader without also invalidating the bake stamp.
//!
//! Paths are repo-relative with `/` separators so they slot straight into the
//! stamp input lists (see [`crate::room_shadow`] / [`crate::room_gi`]).

/// Files composed (in order) into `embedded_wgsl::SHADOW`: the hallway vertex
/// warp prelude followed by the depth-only shadow shader, which references the
/// prepended `HallwayDistortion` / `apply_hallway_distortion` symbols.
pub const SHADOW: &[&str] = &["shaders/hallway_vertex_warp.wgsl", "shaders/shadow.wgsl"];

/// Files composed into `embedded_wgsl::ROOM_SHADOW_MASK`: the same hallway warp
/// prelude followed by the bake-only receiver/occluder mask shader.
pub const ROOM_SHADOW_MASK: &[&str] = &[
    "shaders/hallway_vertex_warp.wgsl",
    "shaders/room_shadow_mask.wgsl",
];

/// Files the `scene_pbr_with_hallway_warp!` macro prepends *before* its room-body
/// argument, in macro order.
pub const SCENE_PBR_WITH_HALLWAY_WARP_PREFIX: &[&str] = &[
    "shaders/hallway_vertex_warp.wgsl",
    "shaders/scene_pbr_core.wgsl",
    "shaders/scene_pbr_lights.wgsl",
    "shaders/rainbow_swirl.wgsl",
    "shaders/moon_phase.wgsl",
];

/// Files the same macro appends *after* its room-body argument, in macro order.
pub const SCENE_PBR_WITH_HALLWAY_WARP_SUFFIX: &[&str] = &["shaders/projected_shadow.wgsl"];

/// Full ordered file list for `scene_pbr_with_hallway_warp!(body)`:
/// prefix shaders, the room-body shader, then suffix shaders.
pub fn scene_pbr_with_hallway_warp(body: &str) -> Vec<&str> {
    SCENE_PBR_WITH_HALLWAY_WARP_PREFIX
        .iter()
        .copied()
        .chain(std::iter::once(body))
        .chain(SCENE_PBR_WITH_HALLWAY_WARP_SUFFIX.iter().copied())
        .collect()
}
