//! Sane UI layering: a single ordered command list per frame.
//!
//! `UiFrame` carries one `Vec<DrawCmd>` plus the per-frame data the renderer
//! needs that isn't a draw call (hand tile mesh state, hit-test buttons, etc).
//! The ordering rule for `cmds` is simple:
//!
//! 1. **Elements pushed earlier render under elements pushed later.**
//! 2. **A widget's children render on top of the widget itself** — which falls
//!    out of rule 1 as long as the widget pushes itself before its children.
//!
//! There are no stages, no z-indexes, no overlay-split indices. Modals,
//! tooltips, and pause menus are just "more cmds pushed at the end."
//!
//! ## Markers
//!
//! A few `DrawCmd` variants are markers (`HandTileBackdrop`, `HandTileFaces`,
//! `FluidSmoke`) that the renderer expands into pipeline-specific draws using
//! its own internal animation state. They obey the same ordering rule: a
//! marker draws *between* whatever was pushed before and after it. Scenes
//! place them in declarative order alongside ordinary cmds.

use crate::core::tile::Tile;
use crate::render::wgpu_renderer::{GpuInstance, RelicIcon, TextLabel};
use crate::scenes::{BackgroundId, ButtonDef};

/// One drawable element in a `UiFrame`.
///
/// The renderer walks `UiFrame.cmds` in order and dispatches each variant to
/// the appropriate pipeline. Contiguous runs of the same variant (e.g.
/// several `Quad`s in a row) are batched into a single instanced draw, which
/// is invisible to scenes and preserves ordering exactly.
pub enum DrawCmd {
    /// Full-screen background image.
    Background(BackgroundId),
    /// Light beams + hand tile body quads (drawn via `light_beam_pipeline` +
    /// `tile_quad_pipeline`). Renderer pulls hand state from `UiFrame`.
    HandTileBackdrop,
    /// Fluid smoke overlay. Renderer owns the simulation state.
    FluidSmoke,
    /// Hand tile face text + emoji indicators (text_pipeline). Splitting from
    /// the backdrop lets scenes draw 2D UI panels between hand tile bodies and
    /// their face labels — preserving the existing visual semantics where
    /// tile faces appear on top of overlay panels.
    HandTileFaces,
    /// Generic 2D quad (panels, dimmers, borders, tooltip backgrounds…).
    Quad(GpuInstance),
    /// Rasterized text label.
    Text(TextLabel),
    /// Pre-loaded relic icon texture.
    RelicIcon(RelicIcon),
}

/// Everything a frame's draw needs: an ordered command list plus per-frame
/// state used by hand-tile markers, hit testing, and the main loop.
pub struct UiFrame {
    /// Drawn back-to-front in order. Push earlier = renders under.
    pub cmds: Vec<DrawCmd>,

    // ── Hand tile state (consumed by HandTileBackdrop / HandTileFaces) ──
    /// Logical hand tiles for `update_hand_tiles`.
    pub hand_tiles: Vec<Tile>,
    /// Screen-space slot rects parallel with `hand_tiles`.
    pub hand_slots: Vec<(f32, f32, f32, f32)>,
    /// Focused hand tile index.
    pub focus: usize,
    /// Selection bitmask parallel with `hand_tiles`.
    pub selected_tiles: Vec<bool>,
    /// Tile indices that should glow with a directional hint this frame.
    pub hint_indices: Vec<usize>,
    /// Tile indices that started departing this frame; consumed by
    /// `WgpuRenderer::depart_tiles` before `update_hand_tiles` removes them.
    pub departing_indices: Vec<usize>,

    // ── Non-draw scene metadata ─────────────────────────────────────────
    /// Hit-test rects for clickable buttons (not drawn).
    pub buttons: Vec<ButtonDef>,
    /// Title shown in the OS window chrome.
    pub window_title: String,
}

impl UiFrame {
    pub fn new() -> Self {
        Self {
            cmds: Vec::new(),
            hand_tiles: Vec::new(),
            hand_slots: Vec::new(),
            focus: 0,
            selected_tiles: Vec::new(),
            hint_indices: Vec::new(),
            departing_indices: Vec::new(),
            buttons: Vec::new(),
            window_title: String::new(),
        }
    }

    // ── Push helpers ────────────────────────────────────────────────────
    pub fn background(&mut self, bg: BackgroundId) {
        self.cmds.push(DrawCmd::Background(bg));
    }
    pub fn hand_tile_backdrop(&mut self) {
        self.cmds.push(DrawCmd::HandTileBackdrop);
    }
    pub fn hand_tile_faces(&mut self) {
        self.cmds.push(DrawCmd::HandTileFaces);
    }
    pub fn fluid_smoke(&mut self) {
        self.cmds.push(DrawCmd::FluidSmoke);
    }
    pub fn quad(&mut self, inst: GpuInstance) {
        self.cmds.push(DrawCmd::Quad(inst));
    }
    pub fn quads<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::Quad));
    }
    pub fn text(&mut self, label: TextLabel) {
        self.cmds.push(DrawCmd::Text(label));
    }
    pub fn texts<I: IntoIterator<Item = TextLabel>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::Text));
    }
    pub fn relic_icons<I: IntoIterator<Item = RelicIcon>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::RelicIcon));
    }

    /// Apply a global alpha multiplier to every queued cmd's color.
    /// Used by the main loop for scene transition fades.
    pub fn apply_alpha(&mut self, alpha: f32) {
        if alpha >= 1.0 {
            return;
        }
        for cmd in self.cmds.iter_mut() {
            match cmd {
                DrawCmd::Quad(inst) => inst.color[3] *= alpha,
                DrawCmd::Text(lbl) => lbl.color[3] *= alpha,
                DrawCmd::Background(_)
                | DrawCmd::HandTileBackdrop
                | DrawCmd::HandTileFaces
                | DrawCmd::FluidSmoke
                | DrawCmd::RelicIcon(_) => {}
            }
        }
    }

}

impl Default for UiFrame {
    fn default() -> Self {
        Self::new()
    }
}
