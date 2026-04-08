//! Scene system: each screen in the game is a `Scene` variant.
//! Scenes transition by returning `Some(Scene)` from `update()`.

pub mod collection;
pub mod game_over;
pub mod gameplay;
pub mod glossary;
pub mod options;
pub mod pause_menu;
pub mod pick_blind;
pub mod profile_select;
pub mod shop;
pub mod solitaire;
pub mod splash;
pub mod start_screen;

pub use collection::CollectionScene;
pub use game_over::GameOverScene;
pub use gameplay::GameplayScene;
pub use options::OptionsScene;
pub use pick_blind::PickBlindScene;
pub use profile_select::ProfileSelectScene;
pub use shop::ShopScene;
pub use solitaire::SolitaireScene;
pub use splash::SplashScene;
pub use start_screen::StartScreenScene;

use enum_dispatch::enum_dispatch;

use crate::core::relic::{RelicId, RelicState, all_relic_defs};
use crate::core::tile::Tile;
use crate::game::cascade::CascadeTuning;
use crate::game::event_bus::EventBus;
use crate::game::run::RunState;
use crate::render::animation::AnimationController;
use crate::render::draw_cmd::{RelicPlacement, UiFrame};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, RelicIcon, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::layout::{LayoutResult, Rect};

/// Which background image to display behind the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BackgroundId {
    /// No background image — just the clear color.
    #[default]
    None,
    /// Main menu: scattered tiles on dark wood.
    Menu,
    /// Gameplay: dark felt table surface.
    Gameplay,
    /// Score/results: golden radiant center burst.
    Score,
}

impl BackgroundId {
    /// Asset path relative to the `assets/` root (embedded via rust-embed).
    pub fn asset_path(self) -> Option<&'static str> {
        match self {
            BackgroundId::None => None,
            BackgroundId::Menu => Some("backgrounds/menu_bg.png"),
            BackgroundId::Gameplay => Some("backgrounds/gameplay_bg.png"),
            BackgroundId::Score => Some("backgrounds/score_bg.png"),
        }
    }
}

/// Everything a scene's `update()` may need.
pub struct UpdateCtx<'a> {
    pub actions: &'a [UiAction],
    /// Scene-defined button click ids fired this frame.
    /// Each entry is the `id` of a `ButtonAction::Scene(id)` whose rect was
    /// clicked. Scenes interpret these ids however they like — typically by
    /// matching against named const values local to the scene.
    pub button_clicks: &'a [u32],
    pub run: &'a mut RunState,
    pub bus: &'a mut EventBus,
    pub anim: &'a mut AnimationController,
    pub layout: &'a LayoutResult,
    /// Current focused hand-slot index (managed by `App`).
    pub focus_tile_index: usize,
    /// Set to `true` to request the application to quit.
    pub quit_requested: &'a mut bool,
    /// Set to switch the active profile (index 0–2).
    pub switch_profile: &'a mut Option<usize>,
    /// Current mouse cursor position in window coordinates.
    pub cursor_pos: (f32, f32),
    /// `true` once all background asset loading has completed.
    pub loading_done: bool,
    /// Cascade animation timing parameters.
    pub cascade_tuning: &'a CascadeTuning,
}

/// Everything a scene's `draw()` may need.
pub struct DrawCtx<'a> {
    pub layout: &'a LayoutResult,
    pub anim: &'a AnimationController,
    pub run: &'a RunState,
    pub focus_tile_index: usize,
    pub progress: &'a crate::core::progression::PlayerProgress,
    pub active_profile: usize,
    /// Whether a game run is currently in progress (for resume/restart UI).
    pub game_in_progress: bool,
    /// Per-hand-tile screen-space rects from the previous frame's perspective
    /// projection (renderer-side). Empty before the first draw. Scenes that
    /// want to anchor 2D HUD elements (hover tooltips, etc.) to the actual
    /// visible 3D tile should look the index up here and fall back to the
    /// layout slot rect if not found.
    pub projected_hand_rects: &'a [(usize, [f32; 4])],
    /// Index of the hand tile under the cursor as determined by raycasting
    /// from the camera through the cursor against each tile's OBB. `None` if
    /// no tile is under the cursor or no pick data is available yet (first
    /// frame). This is the source of truth for hand-tile hover/click —
    /// `projected_hand_rects` remains for anchoring 2D HUD elements.
    pub picked_hand_tile: Option<usize>,
    /// Per-relic-placeholder screen-space rects from the previous frame's
    /// perspective projection — analogous to `projected_hand_rects` but for
    /// the physical relic boxes sitting in the dish. Empty before the first
    /// frame the dish is drawn.
    pub projected_relic_rects: &'a [[f32; 4]],
}

/// What happens when a `ButtonDef` is clicked.
///
/// `Ui(action)` enqueues `action` into the next frame's action queue, just
/// like a key/gamepad press — use this when the click is semantically the
/// same as some keyboard input the scene already handles (e.g. a "Next"
/// button that does the same thing as Enter).
///
/// `Scene(id)` enqueues `id` into `UpdateCtx::button_clicks`, where the
/// scene's `update()` can match against its own named const values. Use this
/// when N buttons each need to do something different and there is no
/// natural keyboard equivalent — it avoids hijacking unrelated `UiAction`
/// variants. See AGENTS.md "Mouse Input in Scenes".
#[derive(Clone, Copy, Debug)]
pub enum ButtonAction {
    Ui(UiAction),
    Scene(u32),
}

/// A clickable UI button: screen rect + the action it triggers.
pub struct ButtonDef {
    pub rect: (f32, f32, f32, f32),
    pub action: ButtonAction,
}

impl ButtonDef {
    /// Convenience constructor for the common `Ui(action)` case.
    pub fn ui(rect: (f32, f32, f32, f32), action: UiAction) -> Self {
        Self {
            rect,
            action: ButtonAction::Ui(action),
        }
    }

    /// Convenience constructor for scene-defined click ids.
    pub fn scene(rect: (f32, f32, f32, f32), id: u32) -> Self {
        Self {
            rect,
            action: ButtonAction::Scene(id),
        }
    }
}

/// What a scene returns from `draw()`.
///
/// This is a *transitional* shape: scenes still hand the main loop separate
/// `instances` / `text_labels` / `relic_icons` vecs, plus hand tile state.
/// The main loop converts this into a `UiFrame` (a single ordered command
/// list) before calling the renderer. Over time, scenes can migrate to
/// pushing into a `UiFrame` directly via [`SceneDrawOutput::into_frame_with`]
/// — the resulting frame's command order *is* the z-order, with no hidden
/// stages.
pub struct SceneDrawOutput {
    /// Background image to render behind everything else.
    pub background: BackgroundId,
    /// 2D quads drawn *before* the 3D hand tile backdrop. Used by scenes that
    /// want a "tray" or "slot pocket" visual to sit underneath the hand tiles
    /// — pushing these via `instances` would land *after* the tile bodies in
    /// frame order and overdraw them.
    pub tray_instances: Vec<GpuInstance>,
    pub instances: Vec<GpuInstance>,
    /// Tiles to render in the hand strip (empty = no hand tiles).
    pub hand_tiles: Vec<Tile>,
    /// Screen-space `(x, y, w, h)` rects for each hand tile; parallel with `hand_tiles`.
    pub hand_slots: Vec<(f32, f32, f32, f32)>,
    /// Index of the focused tile within `hand_tiles`.
    pub focus: usize,
    /// Which hand tiles are selected for discard (parallel with `hand_tiles`).
    pub selected_tiles: Vec<bool>,
    /// Text labels drawn on top of UI panels.
    pub text_labels: Vec<TextLabel>,
    /// Relic icons drawn as textured quads.
    pub relic_icons: Vec<RelicIcon>,
    /// Clickable buttons overlaid on the scene.
    pub buttons: Vec<ButtonDef>,
    pub window_title: String,
    /// Hand tile indices that should animate departing this frame (discard/score).
    pub departing_indices: Vec<usize>,
    /// Hand tile indices that should show a directional light hint.
    pub hint_indices: Vec<usize>,
    /// Procedural flame quads (rendered with the additive flame pipeline).
    /// Each instance's `color.a` carries a per-flame phase offset in [0,1].
    /// Most scenes leave this empty; gameplay populates it for candle flames.
    pub flame_instances: Vec<GpuInstance>,
    /// Point lights for this frame, fed to the 3D tile shader. Most scenes
    /// leave this empty; gameplay populates it for candle flames.
    pub point_lights: Vec<PointLight>,
    /// 3D candle placements drawn via the lit-mesh pipeline. Most scenes
    /// leave this empty; gameplay populates it.
    pub candles: Vec<crate::render::candle_mesh::CandlePlacement>,
    /// Physical relic placeholders sitting on the table dish. Empty for
    /// scenes that don't show the dish (most non-gameplay scenes). When
    /// non-empty, the renderer also draws the dish underneath.
    pub relic_placements: Vec<RelicPlacement>,
    /// Whether to draw the procedural lacquered-wood table backplane behind
    /// the 3D scene. Set by gameplay-style scenes that want a physical
    /// surface under the floating tiles.
    pub draw_table: bool,
    /// Wind impulses to inject into the volumetric smoke sim this frame.
    /// Forwarded verbatim onto the resulting `UiFrame`. Most scenes leave
    /// this empty; gameplay populates it for the post-deal "blow it away"
    /// effect.
    pub wind_gusts: Vec<crate::render::draw_cmd::WindGust>,
}

impl Default for SceneDrawOutput {
    fn default() -> Self {
        Self {
            background: BackgroundId::None,
            tray_instances: Vec::new(),
            instances: Vec::new(),
            hand_tiles: Vec::new(),
            hand_slots: Vec::new(),
            focus: 0,
            selected_tiles: Vec::new(),
            text_labels: Vec::new(),
            relic_icons: Vec::new(),
            buttons: Vec::new(),
            window_title: String::new(),
            departing_indices: Vec::new(),
            hint_indices: Vec::new(),
            flame_instances: Vec::new(),
            point_lights: Vec::new(),
            candles: Vec::new(),
            relic_placements: Vec::new(),
            draw_table: false,
            wind_gusts: Vec::new(),
        }
    }
}

impl SceneDrawOutput {
    /// Build the bones of a `UiFrame` from this scene output, in the canonical
    /// order: background → hand-tile backdrop → fluid smoke → scene quads →
    /// hand-tile faces → scene text → relic icons. The caller is expected to
    /// then push any modal / tooltip / debug overlay cmds at the end of the
    /// returned frame's `cmds` (so they render on top of everything).
    pub fn into_frame(self) -> UiFrame {
        let mut frame = UiFrame::new();
        frame.background(self.background);
        // Wood-table backplane sits behind everything 3D so the tiles and
        // candles read as floating just above its surface.
        if self.draw_table {
            frame.table();
        }
        // Tray quads sit between the table and the hand-tile bodies so the
        // 3D tiles read as floating *on* a recessed pocket rather than above
        // the bare table surface.
        frame.quads(self.tray_instances);
        frame.hand_tile_backdrop();
        // Candle meshes after the hand tiles so the per-candle point lights
        // (which are part of the same pass via group 1) still apply.
        if !self.candles.is_empty() {
            frame.candles(self.candles);
        }
        // Physical relic dish + placeholders. The dish auto-sizes around the
        // batch in the renderer, so we always push them as a pair.
        if !self.relic_placements.is_empty() {
            frame.dish();
            frame.relic_batch(self.relic_placements);
        }
        // Flames belong to the 3D candle scene — push them *before* the
        // 2D scene quads so any UI panel (score, buttons, tooltips) draws
        // on top instead of the additive flame bleeding through.
        frame.flames(self.flame_instances);
        frame.fluid_smoke();
        frame.quads(self.instances);
        frame.hand_tile_faces();
        frame.texts(self.text_labels);
        frame.relic_icons(self.relic_icons);

        frame.hand_tiles = self.hand_tiles;
        frame.hand_slots = self.hand_slots;
        frame.focus = self.focus;
        frame.selected_tiles = self.selected_tiles;
        frame.hint_indices = self.hint_indices;
        frame.departing_indices = self.departing_indices;
        frame.point_lights = self.point_lights;
        frame.wind_gusts = self.wind_gusts;
        frame.buttons = self.buttons;
        frame.window_title = self.window_title;
        frame
    }
}

/// Screen rect of relic badge slot `slot_idx` inside the relic strip.
/// Single source of truth for badge layout — used by `relic_row` and by
/// scenes that need to hit-test or highlight a specific badge.
pub fn relic_badge_rect(
    strip: &Rect,
    window_w: f32,
    max_slots: usize,
    slot_idx: usize,
) -> (f32, f32, f32, f32) {
    let scale = window_w / 600.0;
    let badge_w = (window_w / max_slots.max(1) as f32).min(160.0 * scale);
    let total_w = badge_w * max_slots as f32;
    let start_x = (window_w - total_w) * 0.5;
    let inset = 2.0 * scale;
    let bx = start_x + slot_idx as f32 * badge_w;
    (bx + inset, strip.y, badge_w - inset * 2.0, strip.h)
}

/// Build glow overlay quads for relics that recently activated during a
/// scoring cascade. Draws *behind* the relic row's icon so the existing badge
/// background remains the dominant color, with the glow blooming around it.
///
/// `glow_starts` maps each glowing relic id to the `Instant` it last fired.
/// The glow fades over `lifetime`, after which the entry should be evicted by
/// the caller. Returns the additive overlay quads.
///
/// The gameplay scene uses its own 3D-projected glow path (the relics live
/// on a brass dish, not in this 2D row), so this helper is here for future
/// scenes that *do* render relics via `relic_row` (shop, results).
#[allow(dead_code)]
pub fn relic_glow_overlays(
    relics: &RelicState,
    glow_starts: &std::collections::HashMap<RelicId, std::time::Instant>,
    strip: &Rect,
    window_w: f32,
    now: std::time::Instant,
    lifetime: std::time::Duration,
) -> Vec<GpuInstance> {
    if glow_starts.is_empty() {
        return Vec::new();
    }
    let total_slots = relics.max_slots;
    if total_slots == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let lifetime_s = lifetime.as_secs_f32().max(0.001);
    for (slot_idx, id) in relics.active.iter().enumerate() {
        let Some(start) = glow_starts.get(id) else {
            continue;
        };
        let age = now.saturating_duration_since(*start).as_secs_f32();
        if age >= lifetime_s {
            continue;
        }
        // Quadratic falloff: bright at start, soft tail.
        let t = (1.0 - age / lifetime_s).clamp(0.0, 1.0);
        let alpha = (t * t * 0.85).clamp(0.0, 0.85);
        let (rx, ry, rw, rh) = relic_badge_rect(strip, window_w, total_slots, slot_idx);
        // Bloom rect: slightly larger than the badge so the glow appears to
        // spill outward. Use the gold accent so it reads as "this fired".
        let pad = (rh * 0.18).max(2.0);
        out.push(GpuInstance {
            rect: [rx - pad, ry - pad, rw + pad * 2.0, rh + pad * 2.0],
            color: crate::render::theme::color::alpha(
                crate::render::theme::color::CHAMPAGNE,
                alpha,
            ),
        });
    }
    out
}

/// Build GPU elements for a relic display row inside the relic strip.
/// Returns (background quads, text labels, relic icon quads).
pub fn relic_row(
    relics: &RelicState,
    strip: &Rect,
    window_w: f32,
) -> (Vec<GpuInstance>, Vec<TextLabel>, Vec<RelicIcon>) {
    let defs = all_relic_defs();
    let active: Vec<(RelicId, &str)> = relics
        .active
        .iter()
        .filter_map(|id| defs.iter().find(|d| d.id == *id).map(|d| (*id, d.name)))
        .collect();
    let total_slots = relics.max_slots;
    if total_slots == 0 {
        return (vec![], vec![], vec![]);
    }
    let row_h = strip.h;
    let row_y = strip.y;
    // Scale badge width with window size, cap at a reasonable max.
    let scale = window_w / 600.0;
    let badge_w = (window_w / total_slots.max(1) as f32).min(160.0 * scale);
    let total_w = badge_w * total_slots as f32;
    let start_x = (window_w - total_w) * 0.5;

    let mut instances = Vec::new();
    let mut labels = Vec::new();
    let mut icons = Vec::new();
    for i in 0..total_slots {
        let bx = start_x + i as f32 * badge_w;
        let inset = 2.0 * scale;
        let cell_w = badge_w - inset * 2.0;

        if let Some((relic_id, name)) = active.get(i) {
            // Filled slot background.
            instances.push(GpuInstance {
                rect: [bx + inset, row_y, cell_w, row_h],
                color: crate::render::theme::color::INDIGO,
            });
            // Icon: square, centered horizontally, in the upper portion.
            let icon_size = row_h * 0.65;
            let icon_x = bx + inset + (cell_w - icon_size) * 0.5;
            let icon_y = row_y + row_h * 0.02;
            icons.push(RelicIcon {
                rect: [icon_x, icon_y, icon_size, icon_size],
                relic_id: *relic_id,
            });
            // Name label below the icon.
            let label_h = row_h * 0.28;
            let label_y = icon_y + icon_size + row_h * 0.02;
            labels.push(TextLabel {
                rect: [bx + inset, label_y, cell_w, label_h],
                text: name.to_string(),
                color: crate::render::theme::color::CHAMPAGNE,
                ..Default::default()
            });
        } else {
            // Empty slot: dim outline.
            instances.push(GpuInstance {
                rect: [bx + inset, row_y, cell_w, row_h],
                color: crate::render::theme::color::alpha(
                    crate::render::theme::color::OBSIDIAN,
                    0.5,
                ),
            });
        }
    }
    (instances, labels, icons)
}

/// `None` = stay in current scene; `Some(scene)` = transition.
pub type SceneTransition = Option<Scene>;

/// Behavior shared by every scene variant.
///
/// `enum_dispatch` generates the dispatch arms on `Scene` automatically: each
/// trait method becomes a `match self { Scene::X(s) => s.method(...), ... }`
/// at compile time, so calling `scene.update(ctx)` on the enum forwards to
/// the inner type with zero overhead and no `Box<dyn Trait>` indirection.
///
/// **Why a trait instead of hand-rolled match dispatch:**
///   - Adding a method here = define it once. The compiler enforces that
///     every variant implements it (or inherits a default), so there is no
///     hand-maintained dispatch table to forget.
///   - Default methods like [`Self::has_blocking_overlay`] mean scenes that
///     don't care just inherit the safe answer — no per-variant `_ => false`
///     enumeration.
///   - Adding a new scene = add it to the [`Scene`] enum and `impl
///     SceneBehavior for NewScene { ... }`. No other files change.
#[enum_dispatch]
pub trait SceneBehavior {
    /// Advance scene state by one frame. Returns `Some(next)` to transition.
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition;

    /// Build the draw output for this frame.
    fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput;

    /// Build the canonical `UiFrame` for this scene's frame.
    ///
    /// **This is the new canonical scene draw API.** It returns a single
    /// ordered `UiFrame.cmds` list where push order *is* z-order, so quads
    /// and text labels interleave correctly. Scenes that need fine-grained
    /// z-control between background panels and text on top of them
    /// (notably the gameplay HUD with its hover tooltips) should override
    /// this method directly and push into `UiFrame` in canonical order.
    ///
    /// The default impl forwards to the legacy [`Self::draw`] +
    /// [`SceneDrawOutput::into_frame`] path, which flushes ALL quads, then
    /// hand-tile faces, then ALL text. That ordering is fine for simple
    /// scenes (start screen, splash, options, game over, …) but breaks any
    /// scene where a quad must sit *above* a text label (e.g. tooltip
    /// panels). Migrated scenes override this method and either don't
    /// implement `draw` at all or leave it as `unimplemented!()`.
    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        self.draw(ctx).into_frame()
    }

    /// Whether the scene has a *modal-like internal overlay* currently up
    /// (pause menu, glossary, embedded options sub-screen, scoring cascade,
    /// etc). This is the scene's contribution to `App::modal_overlay_active`,
    /// which centralizes the "is anything blocking input below it" question.
    ///
    /// **Pattern contract:** any new in-scene overlay that should block
    /// input and hover for elements below it MUST be reported here. The
    /// two universal gates (`skip_tooltips` and the `active_buttons` safety
    /// wipe in `main.rs`) consult `App::modal_overlay_active`, so a scene
    /// that forgets to declare its overlay will leak hover/clicks through
    /// it.
    ///
    /// Default: `false`. Scenes without internal overlays inherit the
    /// safe answer automatically.
    fn has_blocking_overlay(&self) -> bool {
        false
    }

    /// Borrow the in-pause-menu options overlay if the player has opened
    /// it. Used by the main loop to sync live audio/graphics settings the
    /// same way it does for the standalone `OptionsScene`.
    ///
    /// Default: `None`. Scenes without an embedded pause menu inherit it.
    fn pause_options_overlay(&self) -> Option<&OptionsScene> {
        None
    }
}

/// The active scene. Enum dispatch — no `Box<dyn Trait>`.
///
/// `enum_dispatch` reads this attribute and generates `impl SceneBehavior
/// for Scene` arms automatically; see [`SceneBehavior`] for the contract.
#[enum_dispatch(SceneBehavior)]
pub enum Scene {
    Splash(SplashScene),
    StartScreen(StartScreenScene),
    ProfileSelect(ProfileSelectScene),
    Shop(ShopScene),
    PickBlind(PickBlindScene),
    Gameplay(GameplayScene),
    GameOver(GameOverScene),
    Options(OptionsScene),
    Collection(CollectionScene),
    Solitaire(SolitaireScene),
}
