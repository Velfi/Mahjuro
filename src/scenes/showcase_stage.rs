//! Shared building blocks for the **Object3d showcase overlay**: draw-order
//! contract and helpers that presenters compose.
//!
//! ## Typical fullscreen celebration / stage order
//!
//! 1. [`UiFrame::background`] — often [`BackgroundId::Black`].
//! 2. Optional dimmer / starfield / fullscreen FX — see [`super::celebration_overlay`].
//! 3. [`UiFrame::clear_scene_depth`] before perspective `Object3d` or [`DrawCmd::ShowcaseTileBatch`].
//! 4. Subject meshes / tile batches + punctual lights + `camera_override` as needed.
//!
//! GPU differences (ray-plane placement, shadows, tonemap) are driven by
//! [`crate::render::draw_cmd::ShowcaseRenderHints`] on [`crate::render::draw_cmd::UiFrame`],
//! not by spoofing legacy per-flow scene keys.
