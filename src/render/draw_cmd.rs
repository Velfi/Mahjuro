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

use crate::core::relic::RelicId;
use crate::core::tile::Tile;
use crate::render::candle_mesh::CandlePlacement;
use crate::render::wgpu_renderer::{GpuInstance, PointLight, RelicIcon, TextLabel};
use crate::scenes::{BackgroundId, ButtonDef};

/// One soft wind impulse to inject into the volumetric smoke sim this frame.
///
/// Coordinates use the same `(pixel_x, pixel_y)` convention as the rest of the
/// scene draw output: the renderer projects them onto the table plane (with
/// the optional `lift_px` height) using its `pixel_to_world` helper before
/// queueing the impulse on the fluid sim. Velocity is in world units (the same
/// space the existing candle plumes and cursor wind use), so a small upward +Z
/// push reads as a gentle breath flowing toward the back of the table.
#[derive(Clone, Copy, Debug)]
pub struct WindGust {
    /// `(pixel_x, pixel_y)` center of the gust in layout-pixel space.
    pub center_px: (f32, f32),
    /// Height above the table plane in world units.
    pub lift: f32,
    /// Velocity in world units per second (x, y, z).
    pub velocity: [f32; 3],
    /// Impulse radius in world units.
    pub radius: f32,
    /// Density delta to add at the impulse center. Negative values pull
    /// existing smoke apart; small positive values trail a faint puff.
    pub density: f32,
}

/// One physical relic placeholder sitting in the dish on the table.
///
/// Coordinates use the same convention as `CandlePlacement`: `world_pos` is
/// `(pixel_x, pixel_y, world_y_lift)` — the renderer maps the pixel x/y onto
/// the table plane and uses world_y as the height above the wood.
#[derive(Clone, Copy, Debug)]
pub struct RelicPlacement {
    /// `(pixel_x, pixel_y, world_y_lift)` for the box's *base center*.
    pub world_pos: [f32; 3],
    /// Half-extents of the relic box in world units (x = width/2, y = height/2,
    /// z = depth/2). Each placeholder gets a slightly different size so the
    /// row reads as a collection of distinct objects.
    pub half_extents: [f32; 3],
    /// Tint color (linear RGBA). Driven by relic rarity in the gameplay scene.
    pub color: [f32; 4],
    /// Which relic this placeholder represents — used by the scene to look up
    /// the name + description for the hover tooltip.
    pub relic_id: RelicId,
    /// Activation glow intensity in [0, 1]. The gameplay scene drives this
    /// with a fast attack + smooth decay envelope when a scoring cascade
    /// step credits this relic. The renderer brightens the relic's base
    /// color and emits an additive halo around its projected screen rect.
    /// Zero (the default) means "not glowing" and skips both effects.
    pub glow: f32,
}

/// One drawable element in a `UiFrame`.
///
/// The renderer walks `UiFrame.cmds` in order and dispatches each variant to
/// the appropriate pipeline. Contiguous runs of the same variant (e.g.
/// several `Quad`s in a row) are batched into a single instanced draw, which
/// is invisible to scenes and preserves ordering exactly.
pub enum DrawCmd {
    /// Full-screen background image.
    Background(BackgroundId),
    /// Procedural lacquered-wood table mesh (one per scene, drawn via
    /// `lit_mesh_pipeline`). Sized by the renderer from the current window.
    Table,
    /// 3D candle meshes for the gameplay scene. Each placement becomes one
    /// wax-body draw + one wick draw via the `lit_mesh_pipeline`. Limited to
    /// the renderer's pre-allocated candle slot pool (currently 4).
    CandleBatch(Vec<CandlePlacement>),
    /// 3D dish mesh sitting on the table — a wide low brass tray that holds
    /// the physical relic placeholders. The renderer reads the placement out
    /// of `RelicBatch` (the dish auto-sizes to enclose the row).
    Dish,
    /// Batch of physical relic placeholders sitting in the dish. Each
    /// placement is a colored axis-aligned box rendered via the
    /// `lit_mesh_pipeline`, instanced from the renderer's pre-allocated
    /// relic slot pool.
    RelicBatch(Vec<RelicPlacement>),
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
    /// Procedural candle flame (additive blend, animated by globals.time).
    /// Instance `color.a` carries a per-flame phase offset in [0,1].
    Flame(GpuInstance),
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
    /// Active point lights this frame. Uploaded to the tile pipeline so the
    /// 3D hand-tile shader can apply candle / spot illumination.
    pub point_lights: Vec<PointLight>,
    /// Mouse cursor position in pixel coordinates, if the scene tracks one.
    /// The renderer projects this onto the table plane and feeds it into the
    /// volumetric smoke sim as a continuous wind impulse.
    pub cursor_pos: Option<(f32, f32)>,
    /// Discrete wind impulses to inject into the smoke sim this frame, on
    /// top of the per-cursor wind. Used by gameplay to "blow" smoke off the
    /// hand strip a few seconds after dealing.
    pub wind_gusts: Vec<WindGust>,

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
            point_lights: Vec::new(),
            cursor_pos: None,
            wind_gusts: Vec::new(),
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
    pub fn table(&mut self) {
        self.cmds.push(DrawCmd::Table);
    }
    pub fn candles(&mut self, placements: Vec<CandlePlacement>) {
        self.cmds.push(DrawCmd::CandleBatch(placements));
    }
    pub fn dish(&mut self) {
        self.cmds.push(DrawCmd::Dish);
    }
    pub fn relic_batch(&mut self, placements: Vec<RelicPlacement>) {
        self.cmds.push(DrawCmd::RelicBatch(placements));
    }
    pub fn quad(&mut self, inst: GpuInstance) {
        self.cmds.push(DrawCmd::Quad(inst));
    }
    pub fn quads<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::Quad));
    }
    pub fn flames<I: IntoIterator<Item = GpuInstance>>(&mut self, iter: I) {
        self.cmds.extend(iter.into_iter().map(DrawCmd::Flame));
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
                // Flame `color.a` is a phase offset, not a transparency.
                // Don't scale it on transitions — the flame fades naturally
                // because the underlying scene quads behind it fade.
                DrawCmd::Flame(_) => {}
                DrawCmd::Text(lbl) => lbl.color[3] *= alpha,
                DrawCmd::Background(_)
                | DrawCmd::HandTileBackdrop
                | DrawCmd::HandTileFaces
                | DrawCmd::FluidSmoke
                | DrawCmd::Table
                | DrawCmd::CandleBatch(_)
                | DrawCmd::Dish
                | DrawCmd::RelicBatch(_)
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
