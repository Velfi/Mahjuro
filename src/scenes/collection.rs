//! Archive collection — 3D skeuomorphic vault. Five tabs
//! (Relics / Yaku / Bosses / Talismans / Chronicle) each render as an infinite
//! scrolling grid of artifacts; focusing a cell lifts its close-up
//! + description card into the foreground HUD.

use std::time::Instant;

use crate::audio::SfxId;
use crate::core::boss::{all_bosses, final_bosses};
use crate::core::relic::{Rarity, RelicId, all_relic_defs};
use crate::core::talisman::TalismanKind;
use crate::core::yaku::YakuKind;
use crate::core::zodiac::ZodiacKind;
use crate::game::event_bus::GameEvent;
use crate::render::draw_cmd::{CameraParams, Object3d, Object3dKind, UiFrame};
use crate::render::table_transform::{euler_xyz_rad_from_deg, rot_fixed_axes_deg};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, GradientQuadInstance, PointLight, TextAlign, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::archive_career;
use super::main_menu_exterior::MainMenuExteriorScene;
use super::profile_select::ProfileSelectScene;
use super::{DrawCtx, OverlayRequest, Scene, SceneBehavior, SceneTransition, UpdateCtx};
use crate::scenes::object3d_inspect::{
    InspectFrameEnv, InspectRig, ItemInspectOrbitState, apply_inspect_view_to_frame,
};

/// 2D chrome sizes shared by [`CollectionScene::draw_collection_frame`] and
/// [`CollectionScene::flat_items`] — tuned for legibility at TV distance.
#[derive(Clone, Copy)]
struct ArchiveChromeLayout {
    scale: f32,
    margin_x: f32,
    title_y: f32,
    chrome_btn_h: f32,
    back_w: f32,
    switch_w: f32,
    arrow_w: f32,
}

fn archive_chrome_layout(w: f32, h: f32) -> ArchiveChromeLayout {
    let scale = metrics::scene_scale(w, h);
    let margin_x = w * 0.04;
    let title_y = h * 0.02;
    // ~5% of screen height, clamped so 720p sofas stay readable and 4K doesn't balloon.
    let chrome_btn_h = (h * 0.052).clamp(44.0, 72.0);
    let back_w = (104.0 * scale).max(90.0);
    let switch_w = (168.0 * scale).min(w * 0.36).max(142.0);
    let arrow_w = (58.0 * scale).max(48.0);
    ArchiveChromeLayout {
        scale,
        margin_x,
        title_y,
        chrome_btn_h,
        back_w,
        switch_w,
        arrow_w,
    }
}

/// One catalog section. Each tab drives a separate grid of artifacts.
/// Yaku entries carry their matching Zodiac ribbon as the 3D prop — the
/// two concepts are 1:1, so keeping them as separate tabs was redundant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Relics,
    Yaku,
    Bosses,
    Talismans,
    Chronicle,
}

const TABS: [Tab; 5] = [
    Tab::Relics,
    Tab::Yaku,
    Tab::Bosses,
    Tab::Talismans,
    Tab::Chronicle,
];

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Relics => "Relics",
            Tab::Yaku => "Yaku",
            Tab::Bosses => "Bosses",
            Tab::Talismans => "Talismans",
            Tab::Chronicle => "Chronicle",
        }
    }
}

/// One catalogued item, summarised for layout. The `kind` field decides
/// which 3D primitive represents the artifact on the table; `unlocked`
/// gates whether we render the real prop or a draped-cloth placeholder.
#[derive(Clone, Debug)]
struct Artifact {
    name: String,
    unlocked: bool,
    kind: ArtifactKind,
    /// Used as a tint on the placeholder when locked, or as a rarity
    /// accent on relics.
    accent: [f32; 4],
}

#[derive(Clone, Debug)]
enum ArtifactKind {
    Relic(RelicId),
    Talisman(TalismanKind),
    Zodiac(ZodiacKind),
    /// Yaku, rules, and bosses don't have per-item 3D models — they
    /// render as engraved plaques on the table.
    PlaqueOnly,
    /// Index into [`crate::core::progression::PlayerProgress::run_history`].
    ChronicleRun(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CollectionAction {
    Back,
    SwitchSave,
    PrevTab,
    NextTab,
    /// Click on an artifact in the current tab's row → set as the
    /// featured item on the inspection pedestal. Indexes into the
    /// active tab's artifact list globally so the selection survives
    /// scroll position changes.
    SelectArtifact(usize),
}

impl CollectionAction {
    fn id(self) -> FocusId {
        match self {
            CollectionAction::Back => FocusId(20),
            CollectionAction::SwitchSave => FocusId(23),
            CollectionAction::PrevTab => FocusId(21),
            CollectionAction::NextTab => FocusId(22),
            // SelectArtifact IDs start at 200. The widget tree just needs
            // unique IDs per hit target — the values themselves don't matter.
            CollectionAction::SelectArtifact(i) => FocusId(200 + i as u32),
        }
    }
}

pub struct CollectionScene {
    tree: TreeState,
    /// Arrange-mode-tunable placements for the cabinet, pedestal, and
    /// pedestal-featured artifact pose.
    pub positions: crate::ui::scene_layout::CollectionPositions,
    /// Currently-selected tab. Determines which content row sits on the
    /// table.
    active_tab: Tab,
    /// Index into the active tab's visible row of the artifact
    /// featured on the inspection pedestal. `None` falls back to the
    /// "first unlocked" artifact for the tab. Set by mouse click or by
    /// pressing Confirm while a row item is focused; reset on tab
    /// change so stale indices don't bleed across tabs.
    selected_artifact: Option<usize>,
    /// Row item that currently has keyboard / controller focus (the
    /// arrow-key / DPad cursor). `None` until the player first uses
    /// directional input. Confirm presses lift the focused item onto
    /// the inspection pedestal (sets `selected_artifact`).
    focused_row: Option<usize>,
    /// Continuous vertical scroll offset of the grid in *rows*. 0.0
    /// means row 0 sits at the top of the visible band. Eased toward
    /// `target_scroll_rows` each frame so wheel ticks and focus-driven
    /// scroll glide rather than snap. Stored in a `Cell` so the
    /// `&self draw_frame` can advance the easing on every frame even
    /// when no input arrived this tick.
    scroll_rows: std::cell::Cell<f32>,
    /// Where `scroll_rows` is easing toward. Set by mouse wheel, focus
    /// follow, and tab changes (which reset to 0.0).
    target_scroll_rows: std::cell::Cell<f32>,
    /// Last instant the scroll easing was advanced.
    scroll_last_tick: std::cell::Cell<Instant>,
    /// Corridor camera tween. Eye/target ride an eased (col, row)
    /// toward the focused cell so arrow-key navigation glides instead
    /// of snapping. Stored as floats in cell units so window resize
    /// doesn't desync the tween. `None` means the camera is parked at
    /// `cam_target` (no tween in flight).
    cam_anim: std::cell::Cell<Option<CamAnim>>,
    /// Last `run_history.len()` reported to persist "seen" chronicle hints.
    chronicle_seen_cursor: Option<u32>,
}

#[derive(Clone, Copy)]
struct CamAnim {
    start_col: f32,
    start_row: f32,
    target_col: f32,
    target_row: f32,
    t0: Instant,
    duration: f32,
}

/// Cubic ease-in-out on t ∈ [0,1]. Matches the score-popup formula.
fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let u = -2.0 * t + 2.0;
        1.0 - u * u * u * 0.5
    }
}

impl CollectionScene {
    pub fn new() -> Self {
        Self {
            tree: TreeState::new(),
            positions: crate::ui::scene_layout::load_collection_positions(),
            active_tab: Tab::Relics,
            selected_artifact: None,
            focused_row: Some(0),
            scroll_rows: std::cell::Cell::new(0.0),
            target_scroll_rows: std::cell::Cell::new(0.0),
            scroll_last_tick: std::cell::Cell::new(Instant::now()),
            cam_anim: std::cell::Cell::new(None),
            chronicle_seen_cursor: None,
        }
    }

    /// Resolve the camera's current (col, row) focus, advancing any
    /// in-flight cubic tween. If `discrete` differs from the tween's
    /// target, re-anchor a fresh tween from the current eased position
    /// so mid-flight direction changes don't jerk.
    fn tick_cam_focus(&self, discrete_col: f32, discrete_row: f32) -> (f32, f32) {
        const DURATION: f32 = 0.22;
        let now = Instant::now();
        let current = self.cam_anim.get();
        let (cur_col, cur_row, target_changed) = match current {
            Some(anim) => {
                let elapsed = now.saturating_duration_since(anim.t0).as_secs_f32();
                let t = (elapsed / anim.duration).clamp(0.0, 1.0);
                let e = ease_in_out_cubic(t);
                let col = anim.start_col + (anim.target_col - anim.start_col) * e;
                let row = anim.start_row + (anim.target_row - anim.start_row) * e;
                let changed = (anim.target_col - discrete_col).abs() > 1e-4
                    || (anim.target_row - discrete_row).abs() > 1e-4;
                if t >= 1.0 && !changed {
                    self.cam_anim.set(None);
                }
                (col, row, changed)
            }
            None => (discrete_col, discrete_row, false),
        };
        if current.is_none() || target_changed {
            let start_col = cur_col;
            let start_row = cur_row;
            if (start_col - discrete_col).abs() > 1e-4 || (start_row - discrete_row).abs() > 1e-4 {
                self.cam_anim.set(Some(CamAnim {
                    start_col,
                    start_row,
                    target_col: discrete_col,
                    target_row: discrete_row,
                    t0: now,
                    duration: DURATION,
                }));
            }
        }
        (cur_col, cur_row)
    }

    fn cycle_tab(&mut self, forward: bool) {
        let idx = TABS.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        let next = if forward {
            (idx + 1) % TABS.len()
        } else {
            (idx + TABS.len() - 1) % TABS.len()
        };
        self.active_tab = TABS[next];
        self.selected_artifact = None;
        self.focused_row = Some(0);
        self.scroll_rows.set(0.0);
        self.target_scroll_rows.set(0.0);
        self.cam_anim.set(None);
    }

    /// Advance `scroll_rows` toward `target_scroll_rows` with the same
    /// exponential-ease shape as `tick_yaw`. Called from both `update`
    /// (so focus-follow scrolling reads a fresh value) and `draw_frame`
    /// (so wheel-only scrolling animates without input). Returns the
    /// post-tick scroll position.
    fn tick_scroll(&self) -> f32 {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.scroll_last_tick.get())
            .as_secs_f32()
            .min(0.10);
        self.scroll_last_tick.set(now);
        let target = self.target_scroll_rows.get();
        let current = self.scroll_rows.get();
        let next = current + (target - current) * (1.0 - (-dt * 14.0).exp());
        self.scroll_rows.set(next);
        next
    }

    /// Whether the scene needs continuous redraws to animate 3D content.
    /// During the rebuild the whole scene is 3D, so this is always true.
    #[allow(dead_code)] // Was used for redraw gating when collection mixed 2D/3D tabs.
    pub fn has_3d_tab(&self) -> bool {
        true
    }

    fn flat_items(
        &self,
        w: f32,
        h: f32,
        progress: &crate::core::progression::PlayerProgress,
    ) -> Vec<FlatItem<CollectionAction>> {
        let ch = archive_chrome_layout(w, h);
        let scale = ch.scale;
        let title_y = ch.title_y;
        let margin_x = ch.margin_x;
        let back_w = ch.back_w;
        let back_h = ch.chrome_btn_h;
        let switch_w = ch.switch_w;
        let switch_x = w - margin_x - switch_w;
        let arrow_w = ch.arrow_w;
        let arrow_h = back_h;
        // Footer-centered Prev/Next so the player can drive the cabinet
        // spin with the mouse, not just the keyboard.
        let center_x = w * 0.5;
        let arrow_y = h - arrow_h - h * 0.02;
        let prev_x = center_x - arrow_w * 1.5;
        let next_x = center_x + arrow_w * 0.5;

        let mut items = vec![
            FlatItem::new(
                CollectionAction::Back.id(),
                [margin_x, title_y, back_w, back_h],
                CollectionAction::Back,
            ),
            FlatItem::new(
                CollectionAction::SwitchSave.id(),
                [switch_x, title_y, switch_w, back_h],
                CollectionAction::SwitchSave,
            ),
            FlatItem::new(
                CollectionAction::PrevTab.id(),
                [prev_x, arrow_y, arrow_w, arrow_h],
                CollectionAction::PrevTab,
            ),
            FlatItem::new(
                CollectionAction::NextTab.id(),
                [next_x, arrow_y, arrow_w, arrow_h],
                CollectionAction::NextTab,
            ),
        ];

        // Compute the same layout draw_frame uses so hit rects line up
        // with the visuals.
        let all = tab_artifacts(self.active_tab, progress);
        let layout = compute_layout(w, h, scale, self.active_tab, all.len());

        // Per-artifact hit rects. Apply current scroll offset and clip
        // to the visible band so off-screen rows can't catch clicks.
        let scroll = self.scroll_rows.get();
        let view_proj = camera_view_proj(w, h, &layout.camera);
        let cell_h_screen = layout.grid_row_pitch * 0.95;
        let cell_half = layout.grid_cell_w * 0.5;
        for idx in 0..all.len() {
            let row = (idx as u32) / layout.grid_cols;
            let col = (idx as u32) % layout.grid_cols;
            let cx = layout.grid_x_start + col as f32 * layout.grid_cell_w;
            let cy = layout.grid_y_top + (row as f32 - scroll) * layout.grid_row_pitch;
            // Clip rows whose center lies more than half a cell outside
            // the band — those are off-screen and not meaningfully
            // clickable.
            if cy + cell_half < layout.band_top_y || cy - cell_half > layout.band_bottom_y {
                continue;
            }
            let world = pixel_to_world_xy(w, h, cx, cy, layout.shelf_top_lift);
            let (sx, sy) = world_to_screen(view_proj, w, h, world);
            let rect_w = layout.grid_cell_w * 0.95;
            let rect_h = cell_h_screen;
            items.push(FlatItem::new(
                CollectionAction::SelectArtifact(idx).id(),
                [sx - rect_w * 0.5, sy - rect_h * 0.5, rect_w, rect_h],
                CollectionAction::SelectArtifact(idx),
            ));
        }

        items
    }

    /// World-space pedestal anchor for [`ItemInspectScene`] orbit (matches HUD close-up).
    fn collection_inspect_target_world(
        &self,
        w: f32,
        h: f32,
        bosses: &[Artifact],
        layout: &crate::ui::layout::LayoutResult,
    ) -> Option<[f32; 3]> {
        if bosses.is_empty() {
            return None;
        }
        let cell = (w * 0.12).min(h * 0.18);
        let cell_gap = cell * 0.22;
        let cell_pitch = cell + cell_gap;
        let cab_px_x = w * 0.5;
        let cab_px_y = h * 0.5;
        let cols: i32 = 6;
        let total_cells = bosses.len() as i32;
        let default_focus = total_cells.min(2);
        let focus_flat = self
            .focused_row
            .map(|i| i as i32)
            .unwrap_or(default_focus)
            .clamp(0, total_cells.saturating_sub(1).max(0));
        let focus_col = focus_flat % cols;
        let focus_row = focus_flat / cols;
        let (cam_col, cam_row) = self.tick_cam_focus(focus_col as f32, focus_row as f32);
        let cam_world_x = (cam_col - (cols as f32 - 1.0) * 0.5) * cell_pitch;
        let cam_world_z = -(cam_row * cell_pitch);
        let hud_world_y_offset = -h * 0.45;
        let hud_py = cab_px_y - hud_world_y_offset;
        let closeup_wx = cam_world_x - h * 0.22;
        let hud_wz = cam_world_z - h * 0.18;
        let closeup_px = cab_px_x + closeup_wx;
        let closeup_anchor = crate::ui::placement::PlacementAnchor::new(
            [closeup_px, hud_py, hud_wz],
            rot_fixed_axes_deg(90.0, 0.0, 0.0),
            &self.positions.featured_artifact,
            "collection.featured_artifact",
            layout,
        );
        let v = pixel_to_world_xy(
            w,
            h,
            closeup_anchor.pos[0],
            closeup_anchor.pos[1],
            closeup_anchor.pos[2],
        );
        Some(v.to_array())
    }

    fn collection_inspect_orbit_for_focus(
        &self,
        w: f32,
        h: f32,
        bosses: &[Artifact],
        layout: &crate::ui::layout::LayoutResult,
    ) -> Option<ItemInspectOrbitState> {
        let tw = self.collection_inspect_target_world(w, h, bosses, layout)?;
        Some(ItemInspectOrbitState {
            target_world: tw,
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
        })
    }

    /// Headless screenshot: orbit for pushdown [`ItemInspectScene`] from the
    /// active tab and current grid focus (same math as the in-game overlay).
    pub fn item_inspect_orbit_for_screenshot(
        &self,
        w: f32,
        h: f32,
        layout: &crate::ui::layout::LayoutResult,
        progress: &crate::core::progression::PlayerProgress,
    ) -> Option<ItemInspectOrbitState> {
        let bosses = tab_artifacts(self.active_tab, progress);
        self.collection_inspect_orbit_for_focus(w, h, &bosses, layout)
    }

    /// Build the draw frame for the Bosses tab — a procedural infinite
    /// corridor in place of the vitrine+grid shelf. Each bay holds one
    /// boss nameplate between two lacquered uprights; extra bays loop
    /// past the last entry into darkness to sell the "recedes to the
    /// vanishing point" effect. The camera sits low in the hallway
    /// looking straight along +Y so the fisheye post-pass bends the
    /// framing into a lens-like warp. Scroll dollies the camera forward
    /// through the corridor instead of sliding rows.
    fn build_corridor_frame(
        &self,
        mut frame: UiFrame,
        quads: Vec<GpuInstance>,
        text_labels: Vec<TextLabel>,
        bosses: &[Artifact],
        ctx: DrawCtx<'_>,
        inspect: Option<&ItemInspectOrbitState>,
    ) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;

        // ── Infinite curio-cabinet wall ───────────────────────────────
        // Camera faces a flat cabinet grid head-on. Cells extend far
        // beyond the window in every direction; the fisheye post-pass
        // bends the edges toward the vanishing corners so the grid
        // reads as "recedes into infinity." The player steers the
        // viewport with L/R/U/D (mapped in `update()` via
        // `focused_row`); the camera chases the focused cell so it
        // stays near screen-centre, and a description card pops up for
        // the focused boss.
        //
        // Object `pos` values in `Object3d` are pixel-space (the
        // renderer calls `pixel_to_world` on them). The camera
        // override, however, takes world coordinates directly. To keep
        // the two consistent we:
        //   • Place objects relative to a pixel-space cabinet anchor
        //     (`cab_px_x`, `cab_px_y`) and lift them in world-space Z.
        //   • Derive the camera world-space eye/target from the same
        //     pixel anchor via `pixel_to_world`-equivalent arithmetic.
        let cell = (w * 0.12).min(h * 0.18);
        let cell_gap = cell * 0.22;
        let cell_pitch = cell + cell_gap;
        // Pixel-space anchor for the cabinet center. This is where the
        // focused cell lands on-screen before fisheye warp. Cabinet
        // plane sits at world Y = 0 (i.e. pixel_y = h/2) so objects
        // with pos.1 = h/2 end up at world Y = 0.
        let cab_px_x = w * 0.5;
        let cab_px_y = h * 0.5;
        // Camera world-Y pulled back so the cabinet fills ~⅔ of the
        // frame at the default FOV. Camera looks along +Y toward the
        // cabinet plane (world Y = 0).
        let cam_world_y = -h * 1.1;

        // Grid dimensions. Column count is 6 (matches the original
        // vitrine's grid). Rows span the full boss catalogue; we
        // render a window of rows around the focused cell so
        // draw-calls stay bounded while the player can scroll
        // forever across the logical grid.
        let cols: i32 = 6;
        let total_cells = bosses.len() as i32;
        let total_rows = (total_cells + cols - 1).max(1) / cols;
        // Focused cell (col, row). Maps from the existing focused_row
        // index so the in-place keyboard navigation (arrows / DPad)
        // continues to work without new UI plumbing. Default focus is
        // the middle-ish cell (col 2, row 0) so the initial view is
        // already centred on a boss rather than parked on the left
        // edge.
        let default_focus = total_cells.min(2);
        let focus_flat = self
            .focused_row
            .map(|i| i as i32)
            .unwrap_or(default_focus)
            .clamp(0, total_cells.saturating_sub(1).max(0));
        let focus_col = focus_flat % cols;
        let focus_row = focus_flat / cols;

        // Camera target rides the focused cell so it sits near screen
        // centre. Camera eye/target are in WORLD coordinates (the
        // renderer does not pixel-convert camera params).
        //   world_x = pixel_x - w*0.5
        //   world_y = h*0.5 - pixel_y
        //   world_z = lift
        // The focused cell's world X is the offset from the cabinet
        // center; world Z is the focused row's lift. World Y = 0 for
        // all cells (they lie on the cabinet plane at pixel_y = h/2).
        let focus_world_x = (focus_col as f32 - (cols as f32 - 1.0) * 0.5) * cell_pitch;
        let focus_world_z = -(focus_row as f32 * cell_pitch);
        // Camera rides an eased (col, row) so arrow-key moves glide via
        // a cubic ease-in-out instead of snapping. Visuals (lights,
        // focused-cell highlight) continue to use the discrete focus so
        // the player still gets immediate confirmation their input
        // registered.
        let (cam_col, cam_row) = self.tick_cam_focus(focus_col as f32, focus_row as f32);
        let cam_world_x = (cam_col - (cols as f32 - 1.0) * 0.5) * cell_pitch;
        let cam_world_z = -(cam_row * cell_pitch);
        let base_cam = CameraParams {
            eye: [cam_world_x, cam_world_y, cam_world_z],
            target: [cam_world_x, 0.0, cam_world_z],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 48.0,
        };
        let coll_rig = InspectRig::collection(h);
        if let Some(ins) = inspect {
            apply_inspect_view_to_frame(
                &mut frame,
                w,
                h,
                ins,
                &coll_rig,
                ins.target_world,
                InspectFrameEnv::Neutral,
            );
        } else {
            frame.camera_override = Some(base_cam);
        }

        frame.fisheye_strength = 0.0;

        let focus_px_x = cab_px_x + focus_world_x;
        let focus_px_y = cab_px_y; // cabinet plane
        let focus_px_z = focus_world_z;

        if inspect.is_none() {
            // Three warm key-lights in front of the cabinet. Light `pos`
            // is pixel-space (renderer converts via `pixel_to_world`), so
            // pixel_x + pixel_y define the (X, Y) world position and the
            // third coordinate is the world-Z lift directly.
            frame.scene_lighting.set_smooth_points(vec![
                PointLight {
                    pos: [focus_px_x, cab_px_y + h * 0.5, focus_px_z + cell * 0.5],
                    radius: cell_pitch * 14.0,
                    color: [1.0, 0.88, 0.62],
                    intensity: 2.4,
                },
                PointLight {
                    pos: [
                        focus_px_x - cell_pitch * 4.0,
                        focus_px_y + h * 0.4,
                        focus_px_z + cell_pitch * 3.0,
                    ],
                    radius: cell_pitch * 10.0,
                    color: [0.85, 0.70, 1.0],
                    intensity: 1.3,
                },
                PointLight {
                    pos: [
                        focus_px_x + cell_pitch * 4.0,
                        focus_px_y + h * 0.4,
                        focus_px_z - cell_pitch * 3.0,
                    ],
                    radius: cell_pitch * 10.0,
                    color: [1.0, 0.65, 0.55],
                    intensity: 1.3,
                },
            ]);
        }

        // Grid window size — how many cells we actually push as 3D
        // objects. Larger = more infinite-looking, slower to render.
        // Bounded around the focused cell so the draw count stays
        // constant as the player scrolls.
        let window_cols: i32 = 11;
        let window_rows: i32 = 9;
        let col_min = focus_col - window_cols;
        let col_max = focus_col + window_cols;
        let row_min = focus_row - window_rows;
        let row_max = focus_row + window_rows;

        let mut plaques: Vec<Object3d> = Vec::with_capacity(
            ((col_max - col_min + 1) * (row_max - row_min + 1)) as usize * 2 + 8,
        );

        // Back-wall slab: a single dark lacquered panel behind the
        // whole cell window. Sits BEHIND the cabinet plane in world Y
        // (i.e. smaller pixel_y than cab_px_y maps to world Y > 0,
        // which is farther from camera than the plane). Gaps between
        // cells show this panel rather than the clear colour.
        let backing_w = (window_cols as f32 * 2.5) * cell_pitch;
        let backing_h = (window_rows as f32 * 2.5) * cell_pitch;
        plaques.push(Object3d {
            // pixel_y = cab_px_y - cell*0.5 → world Y = +0.5*cell (behind plane).
            pos: [focus_px_x, cab_px_y - cell * 0.5, focus_world_z],
            extents: [backing_w, backing_h, cell * 0.1],
            rotation: euler_xyz_rad_from_deg(90.0, 0.0, 0.0),
            color: [0.10, 0.07, 0.04, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::BeveledSlab,
                material: crate::render::primitive::MaterialSpec::lacquered_wood_flat(),
                pick_id: None,
                shadow_caster: false,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: Some("collection.cabinet_backing"),
        });

        // Cell frames + nameplates. For each (col, row) in the window
        // we push a frame (lacquered wood) and — if that cell maps to
        // a real boss — a tinted nameplate sitting slightly proud of
        // the frame. Cells beyond the catalogue fill the grid with
        // empty frames so the cabinet extends into darkness on all
        // sides rather than having a ragged edge.
        for row in row_min..=row_max {
            for col in col_min..=col_max {
                // Pixel X: cabinet center + per-col offset.
                let cx = cab_px_x + (col as f32 - (cols as f32 - 1.0) * 0.5) * cell_pitch;
                // World Z: focused row is 0, lower rows go negative.
                let cz = -row as f32 * cell_pitch;

                // Distance from focus, measured in cells, drives a
                // depth-like fade so outer cells dim toward black.
                let dc = (col - focus_col) as f32;
                let dr = (row - focus_row) as f32;
                let focus_d2 = dc * dc + dr * dr;
                let fade = (1.0 - focus_d2 / ((window_cols.max(window_rows) as f32).powi(2) * 0.9))
                    .clamp(0.08, 1.0);

                // (Frame geometry removed — the dark backing slab
                // plus the nameplate gap reads as a grid without
                // competing wood tiles in every cell.)

                // Map (col, row) to boss index; skip if this cell is
                // outside the catalogue (architecture stays, content
                // doesn't).
                if col < 0 || col >= cols || row < 0 || row >= total_rows {
                    continue;
                }
                let boss_i = row * cols + col;
                if boss_i < 0 || boss_i >= total_cells {
                    continue;
                }
                let boss = &bosses[boss_i as usize];

                // Nameplate: a smaller, saturated-accent tile inset
                // inside the wood frame so the frame reads as a
                // picture-frame around coloured content. Using the
                // boss accent directly (not tinted toward wood) so the
                // cells pop as distinct boss cards rather than looking
                // like more cabinet. Focused cell is larger and lifted
                // farther forward so it stands out against the wall.
                // Nameplate pixel_y: ABOVE cab_px_y (smaller pixel_y)
                // gives world Y > 0 (farther from camera); BELOW gives
                // world Y < 0 (closer). We want nameplates CLOSER than
                // the cabinet plane, so add to cab_px_y.
                let is_focus = col == focus_col && row == focus_row;
                let nameplate_py = cab_px_y + if is_focus { cell * 0.5 } else { cell * 0.15 };
                let plate_w = cell * if is_focus { 0.78 } else { 0.62 };
                let plate_h = cell * if is_focus { 0.78 } else { 0.62 };
                let plate_thick = cell * 0.06;
                // Use the boss accent as-is (not lift_floor'd toward
                // white) so distinct tiers stay visually distinct.
                // Outer cells still dim toward black via `fade`.
                // Push the accent up by ~1.8x so lit-mesh shading
                // doesn't swallow it into the wood background. Clamp
                // alpha to 1.0; channels above 1.0 feed bloom for a
                // subtle glow on the focused cell.
                let bright = {
                    let k = if is_focus { 2.0 } else { 1.6 };
                    let f = fade.max(if is_focus { 1.0 } else { 0.55 });
                    [
                        boss.accent[0] * k * f,
                        boss.accent[1] * k * f,
                        boss.accent[2] * k * f,
                        1.0,
                    ]
                };
                match &boss.kind {
                    ArtifactKind::Relic(relic_id) => {
                        // Render the actual silhouette-extruded relic model
                        // instead of a tinted plaque. Locked relics render
                        // as a dim rarity-accent silhouette so the row still
                        // reads as a ladder of unlockables. The extents
                        // square (plate_w × plate_w × thick) fills the cell
                        // from the plaque layout so focus-scaling stays
                        // consistent with non-relic cells.
                        let silhouette = !boss.unlocked;
                        let visual = crate::core::relic::relic_visual(*relic_id);
                        let face = plate_w;
                        let thick = face * 0.12 * visual.thickness_scale;
                        let color = if silhouette {
                            // Muted rarity tint: locked entries still carry
                            // a dim hint of their accent so Common / Rare /
                            // Legendary remain visually distinct in the
                            // cabinet ladder.
                            [
                                boss.accent[0] * 0.22 + 0.02,
                                boss.accent[1] * 0.22 + 0.02,
                                boss.accent[2] * 0.22 + 0.02,
                                1.0,
                            ]
                        } else {
                            boss.accent
                        };
                        plaques.push(Object3d {
                            pos: [cx, nameplate_py, cz],
                            extents: [face, thick, face],
                            rotation: euler_xyz_rad_from_deg(
                                180.0 + visual.ui_tilt_x_deg,
                                0.0,
                                0.0,
                            ),
                            color,
                            kind: Object3dKind::Relic {
                                relic_id: *relic_id,
                                glow: if is_focus && !silhouette { 0.6 } else { 0.0 },
                                silhouette,
                                debuffed: false,
                                pick_id: Some(boss_i as u32),
                            },
                            hover_target: if is_focus { 1.0 } else { 0.0 },
                            anim_id: boss_i as u64,
                            arrange_name: None,
                        });
                    }
                    ArtifactKind::Talisman(tk) => {
                        // Hang the tablet pendant-style: the mesh UVs
                        // assume the shop's Rx(-90°) peg orientation,
                        // so Rx(+90°) here was flipping the art upside
                        // down. A small Ry tilt breaks the dead-flat
                        // pose so jade/pearl/foil materials catch the
                        // cabinet key-lights at a glancing angle.
                        plaques.push(Object3d {
                            pos: [cx, nameplate_py, cz],
                            extents: [plate_w * 0.70, plate_w, plate_w * 0.18],
                            rotation: euler_xyz_rad_from_deg(-90.0, 14.0, 0.0),
                            color: bright,
                            kind: Object3dKind::Talisman { kind: *tk },
                            hover_target: if is_focus { 1.0 } else { 0.0 },
                            anim_id: boss_i as u64,
                            arrange_name: None,
                        });
                    }
                    ArtifactKind::Zodiac(zk) => {
                        // Silken ribbon draped along the cell's long
                        // axis. Width × 0.15 = thickness per renderer.
                        let rib_w = plate_w * 0.34;
                        plaques.push(Object3d {
                            pos: [cx, nameplate_py, cz],
                            extents: [rib_w, plate_w, rib_w * 0.15],
                            rotation: euler_xyz_rad_from_deg(90.0, 0.0, 0.0),
                            color: [1.0, 1.0, 1.0, 1.0],
                            kind: Object3dKind::ZodiacRibbon { kind: Some(*zk) },
                            hover_target: if is_focus { 1.0 } else { 0.0 },
                            anim_id: boss_i as u64,
                            arrange_name: None,
                        });
                    }
                    ArtifactKind::PlaqueOnly => {
                        use crate::render::primitive::{
                            DecalLayout, DecalSpec, MaterialSpec, MeshId,
                        };
                        plaques.push(Object3d {
                            pos: [cx, nameplate_py, cz],
                            extents: [plate_w, plate_h, plate_thick],
                            rotation: euler_xyz_rad_from_deg(90.0, 0.0, 0.0),
                            color: bright,
                            kind: Object3dKind::Primitive {
                                shape: MeshId::BeveledSlab,
                                material: MaterialSpec::lacquered_wood_flat().with_decal(
                                    DecalSpec {
                                        text: boss.name.clone(),
                                        layout: DecalLayout::Fit {
                                            target_short_edge:
                                                crate::render::decal::PLAQUE_DECAL_HEIGHT,
                                        },
                                    },
                                ),
                                pick_id: None,
                                shadow_caster: false,
                                silhouette: false,
                            },
                            hover_target: if is_focus { 1.0 } else { 0.0 },
                            anim_id: boss_i as u64,
                            arrange_name: None,
                        });
                    }
                    ArtifactKind::ChronicleRun(_) => {
                        use crate::render::primitive::{
                            DecalLayout, DecalSpec, MaterialSpec, MeshId,
                        };
                        plaques.push(Object3d {
                            pos: [cx, nameplate_py, cz],
                            extents: [plate_w, plate_h, plate_thick],
                            rotation: euler_xyz_rad_from_deg(90.0, 0.0, 0.0),
                            color: bright,
                            kind: Object3dKind::Primitive {
                                shape: MeshId::BeveledSlab,
                                material: MaterialSpec::lacquered_wood_flat().with_decal(
                                    DecalSpec {
                                        text: boss.name.clone(),
                                        layout: DecalLayout::Fit {
                                            target_short_edge:
                                                crate::render::decal::PLAQUE_DECAL_HEIGHT,
                                        },
                                    },
                                ),
                                pick_id: None,
                                shadow_caster: false,
                                silhouette: false,
                            },
                            hover_target: if is_focus { 1.0 } else { 0.0 },
                            anim_id: boss_i as u64,
                            arrange_name: None,
                        });
                    }
                }
            }
        }

        // ── Foreground HUD: close-up + description plaque ──────────────
        // Both sit in front of the cabinet plane, anchored to the eased
        // camera so they hold a fixed on-screen position as the player
        // scrolls the grid. The close-up goes on the left, description
        // on the right; the focused cell is visible between/behind them.
        // HUD close-up + description + stats are drawn as a second 3D
        // batch after the grid so the gradient backers (drawn between
        // the two batches) compose correctly — grid behind, backers on
        // top of grid, HUD panels on top of backers.
        let mut hud_plaques: Vec<Object3d> = Vec::new();
        let mut gradient_backers: Vec<GradientQuadInstance> = Vec::new();

        if let Some(boss) = bosses.get(focus_flat as usize) {
            // World-space offsets from the camera target. View direction
            // is +Y (eye at world_y = -h*1.1, target at world_y = 0), so
            // X and Z offsets slide the object across the view plane.
            // Pulling world_y toward the eye pushes the object forward
            // (closer) under the fisheye warp.
            // Horizontal and vertical offsets anchored to the eased
            // camera so both panels hold a fixed screen-relative
            // position as the player scrolls. Math: passing
            // `cab_px_x + (cam_world_x + off_x)` as pixel_x yields
            // world X = cam_world_x + off_x. Camera FOV 48° + distance
            // h*1.1 means the view plane at world_y=0 is ~h wide; HUD
            // sits closer (smaller world_y) so its view plane is ~0.6h.
            // Offsets below are tuned to keep both panels comfortably
            // inside the frame.
            let hud_world_y_offset = -h * 0.45; // in front of cabinet plane
            let hud_py = cab_px_y - hud_world_y_offset; // py = h/2 - world_y ⇒ world_y<0 → py>h/2
            let closeup_wx = cam_world_x - h * 0.22;
            let card_wx = cam_world_x + h * 0.20;
            let hud_wz = cam_world_z - h * 0.18;

            // ── Close-up ─────────────────────────────────────────────
            // Render the focused artifact large in the HUD using the
            // same kind-aware path the grid uses, so each category
            // reads the same visually (relic silhouette/medallion vs.
            // tinted plaque). No `glow` on the relic here — the grid
            // cell already carries the pulsing selection halo.
            let closeup_size = h * 0.28;
            let closeup_px = cab_px_x + closeup_wx;
            let closeup_anchor = crate::ui::placement::PlacementAnchor::new(
                [closeup_px, hud_py, hud_wz],
                rot_fixed_axes_deg(90.0, 0.0, 0.0),
                &self.positions.featured_artifact,
                "collection.featured_artifact",
                ctx.layout,
            );
            let closeup_bright = {
                let k = 2.2;
                [
                    boss.accent[0] * k,
                    boss.accent[1] * k,
                    boss.accent[2] * k,
                    1.0,
                ]
            };
            match &boss.kind {
                ArtifactKind::Relic(relic_id) => {
                    let silhouette = !boss.unlocked;
                    let visual = crate::core::relic::relic_visual(*relic_id);
                    let face = closeup_size;
                    let thick = face * 0.12 * visual.thickness_scale;
                    let color = if silhouette {
                        [
                            boss.accent[0] * 0.22 + 0.02,
                            boss.accent[1] * 0.22 + 0.02,
                            boss.accent[2] * 0.22 + 0.02,
                            1.0,
                        ]
                    } else {
                        boss.accent
                    };
                    hud_plaques.push(Object3d {
                        pos: closeup_anchor.pos,
                        extents: [face, thick, face],
                        rotation: euler_xyz_rad_from_deg(180.0 + visual.ui_tilt_x_deg, 0.0, 0.0),
                        color,
                        kind: Object3dKind::Relic {
                            relic_id: *relic_id,
                            glow: 0.0,
                            silhouette,
                            debuffed: false,
                            pick_id: None,
                        },
                        hover_target: 1.0,
                        anim_id: 0xC105E0,
                        arrange_name: Some(closeup_anchor.arrange_name),
                    });
                }
                ArtifactKind::Talisman(tk) => {
                    // Pendant orientation matches the grid cells; see the
                    // grid-side Rx(-90°) comment. 14° Ry tilt keeps the
                    // material sheen readable on the featured tablet too.
                    hud_plaques.push(Object3d {
                        pos: closeup_anchor.pos,
                        extents: [closeup_size * 0.70, closeup_size, closeup_size * 0.18],
                        rotation: euler_xyz_rad_from_deg(-90.0, 14.0, 0.0),
                        color: closeup_bright,
                        kind: Object3dKind::Talisman { kind: *tk },
                        hover_target: 1.0,
                        anim_id: 0xC105E0,
                        arrange_name: Some(closeup_anchor.arrange_name),
                    });
                }
                ArtifactKind::Zodiac(zk) => {
                    let rib_w = closeup_size * 0.34;
                    hud_plaques.push(Object3d {
                        pos: closeup_anchor.pos,
                        extents: [rib_w, closeup_size, rib_w * 0.15],
                        rotation: closeup_anchor.object3d_rotation(),
                        color: [1.0, 1.0, 1.0, 1.0],
                        kind: Object3dKind::ZodiacRibbon { kind: Some(*zk) },
                        hover_target: 1.0,
                        anim_id: 0xC105E0,
                        arrange_name: Some(closeup_anchor.arrange_name),
                    });
                }
                ArtifactKind::PlaqueOnly => {
                    let label = if boss.unlocked {
                        boss.name.clone()
                    } else {
                        String::from("???")
                    };
                    let color = if boss.unlocked {
                        closeup_bright
                    } else {
                        [
                            boss.accent[0] * 0.22 + 0.02,
                            boss.accent[1] * 0.22 + 0.02,
                            boss.accent[2] * 0.22 + 0.02,
                            1.0,
                        ]
                    };
                    use crate::render::primitive::{DecalLayout, DecalSpec, MaterialSpec, MeshId};
                    let closeup_silhouette = !boss.unlocked;
                    let closeup_material = if closeup_silhouette {
                        // Silhouette overrides decal anyway, but keep
                        // the spec tidy.
                        MaterialSpec::lacquered_wood_flat()
                    } else {
                        MaterialSpec::lacquered_wood_flat().with_decal(DecalSpec {
                            text: label,
                            layout: DecalLayout::Fit {
                                target_short_edge: crate::render::decal::PLAQUE_DECAL_HEIGHT,
                            },
                        })
                    };
                    hud_plaques.push(Object3d {
                        pos: closeup_anchor.pos,
                        extents: [closeup_size, closeup_size, closeup_size * 0.1],
                        rotation: closeup_anchor.object3d_rotation(),
                        color,
                        kind: Object3dKind::Primitive {
                            shape: MeshId::BeveledSlab,
                            material: closeup_material,
                            pick_id: None,
                            shadow_caster: false,
                            silhouette: closeup_silhouette,
                        },
                        hover_target: 1.0,
                        anim_id: 0xC105E0,
                        arrange_name: Some(closeup_anchor.arrange_name),
                    });
                }
                ArtifactKind::ChronicleRun(_) => {
                    let label = boss.name.clone();
                    use crate::render::primitive::{DecalLayout, DecalSpec, MaterialSpec, MeshId};
                    let closeup_material = MaterialSpec::lacquered_wood_flat().with_decal(DecalSpec {
                        text: label,
                        layout: DecalLayout::Fit {
                            target_short_edge: crate::render::decal::PLAQUE_DECAL_HEIGHT,
                        },
                    });
                    hud_plaques.push(Object3d {
                        pos: closeup_anchor.pos,
                        extents: [closeup_size, closeup_size, closeup_size * 0.1],
                        rotation: closeup_anchor.object3d_rotation(),
                        color: closeup_bright,
                        kind: Object3dKind::Primitive {
                            shape: MeshId::BeveledSlab,
                            material: closeup_material,
                            pick_id: None,
                            shadow_caster: false,
                            silhouette: false,
                        },
                        hover_target: 1.0,
                        anim_id: 0xC105E0,
                        arrange_name: Some(closeup_anchor.arrange_name),
                    });
                }
            }

            // ── Description plaque ───────────────────────────────────
            let card_w = h * 0.22;
            let card_h = h * 0.16;
            let card_px = cab_px_x + card_wx;
            let body = description_for(boss, ctx.run, ctx.progress);
            let card_text = if body.is_empty() {
                boss.name.clone()
            } else {
                format!("{}\n\n{}", boss.name, body)
            };
            let anchor = crate::ui::placement::PlacementAnchor::new(
                [card_px, hud_py, hud_wz],
                rot_fixed_axes_deg(90.0, 0.0, 0.0),
                &self.positions.focus_card,
                "collection.focus_card",
                ctx.layout,
            );
            {
                use crate::render::primitive::{DecalLayout, DecalSpec, MaterialSpec, MeshId};
                hud_plaques.push(Object3d {
                    pos: anchor.pos,
                    extents: [card_w, card_h, card_h * 0.06],
                    rotation: anchor.object3d_rotation(),
                    color: [0.94, 0.86, 0.56, 1.0],
                    kind: Object3dKind::Primitive {
                        shape: MeshId::BeveledSlab,
                        material: MaterialSpec::lacquered_wood_flat().with_decal(DecalSpec {
                            text: card_text,
                            layout: DecalLayout::Fit {
                                target_short_edge: crate::render::decal::PLAQUE_DECAL_HEIGHT,
                            },
                        }),
                        pick_id: None,
                        shadow_caster: false,
                        silhouette: false,
                    },
                    hover_target: 1.0,
                    anim_id: 0xC0DE,
                    arrange_name: Some(anchor.arrange_name),
                });
            }

            // ── Stats plaque ─────────────────────────────────────────
            // Tucked under the description card, same width + darker
            // slate so it reads as an annex rather than a peer panel.
            // Content is per-artifact-kind counters drawn from
            // PlayerProgress (profile-wide tallies, not run-scoped).
            let stats_text = stats_for(boss, ctx.progress);
            if !stats_text.is_empty() {
                let stats_h = h * 0.08;
                let stats_wz = hud_wz - card_h * 0.65 - stats_h * 0.55;
                use crate::render::primitive::{DecalLayout, DecalSpec, MaterialSpec, MeshId};
                hud_plaques.push(Object3d {
                    pos: [card_px, hud_py, stats_wz],
                    extents: [card_w, stats_h, stats_h * 0.06],
                    rotation: euler_xyz_rad_from_deg(90.0, 0.0, 0.0),
                    color: [0.26, 0.22, 0.16, 1.0],
                    kind: Object3dKind::Primitive {
                        shape: MeshId::BeveledSlab,
                        material: MaterialSpec::lacquered_wood_flat().with_decal(DecalSpec {
                            text: stats_text,
                            layout: DecalLayout::Fit {
                                target_short_edge: crate::render::decal::PLAQUE_DECAL_HEIGHT,
                            },
                        }),
                        pick_id: None,
                        shadow_caster: false,
                        silhouette: false,
                    },
                    hover_target: 1.0,
                    anim_id: 0xC0DF,
                    arrange_name: Some("collection.stats_plaque"),
                });
            }

            // Soft dark backers behind the two HUD panels. Collected
            // into `gradient_backers` and emitted between the grid 3D
            // batch and the HUD 3D batch below, so the grid sits
            // behind the gradient and the HUD panels compose on top.
            // Projects each panel's world-space visible face (Rx(90°)
            // makes the face span XZ) through the camera view-proj,
            // then pads the screen-space bbox so the gradient feathers
            // beyond the panel edges.
            if let Some(cam) = frame.camera_override {
                let vp = camera_view_proj(w, h, &cam);
                let closeup_half_face = closeup_size * 0.65;
                let card_half_w = card_w * 0.65;
                let card_half_h = card_h * 0.80;
                for (center_wx, half_w, half_h) in [
                    (closeup_wx, closeup_half_face, closeup_half_face),
                    (card_wx, card_half_w, card_half_h),
                ] {
                    let corners = [
                        glam::Vec3::new(center_wx - half_w, hud_world_y_offset, hud_wz - half_h),
                        glam::Vec3::new(center_wx + half_w, hud_world_y_offset, hud_wz - half_h),
                        glam::Vec3::new(center_wx + half_w, hud_world_y_offset, hud_wz + half_h),
                        glam::Vec3::new(center_wx - half_w, hud_world_y_offset, hud_wz + half_h),
                    ];
                    let mut min_x = f32::INFINITY;
                    let mut min_y = f32::INFINITY;
                    let mut max_x = f32::NEG_INFINITY;
                    let mut max_y = f32::NEG_INFINITY;
                    for c in corners {
                        let (sx, sy) = world_to_screen(vp, w, h, c);
                        min_x = min_x.min(sx);
                        min_y = min_y.min(sy);
                        max_x = max_x.max(sx);
                        max_y = max_y.max(sy);
                    }
                    let pad = h * 0.05;
                    let rx = min_x - pad;
                    let ry = min_y - pad;
                    let rw = (max_x - min_x) + 2.0 * pad;
                    let rh = (max_y - min_y) + 2.0 * pad;
                    gradient_backers.push(GradientQuadInstance {
                        rect: [rx, ry, rw, rh],
                        color: [0.0, 0.0, 0.0, 0.88],
                        feather: [0.35, 0.0, 0.0, 0.0],
                    });
                }
            }
        }

        // Assemble the frame. 2D UI (title, back button, footer arrows,
        // hint text) was already built by the caller and is passed in
        // via `quads` / `text_labels` so this corridor path shares all
        // the chrome with the vitrine layout.
        frame.quads(quads);
        frame.object3d_batch(plaques);
        if !gradient_backers.is_empty() {
            frame.gradient_quads(gradient_backers);
        }
        if !hud_plaques.is_empty() {
            frame.object3d_batch(hud_plaques);
        }
        frame.texts(text_labels);

        // Hit rects for 2D chrome — skipped while [`ItemInspectScene`] owns input.
        if inspect.is_none() {
            let items = self.flat_items(w, h, ctx.progress);
            self.tree.register_flat_buttons(&items, &mut frame.buttons);
        }

        frame.window_title = format!("Mahjuro — Archive ({})", self.active_tab.label());
        frame
    }

    pub(crate) fn draw_collection_frame(
        &self,
        ctx: DrawCtx<'_>,
        inspect: Option<&ItemInspectOrbitState>,
    ) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ch = archive_chrome_layout(w, h);
        let scale = ch.scale;
        let margin_x = ch.margin_x;
        let title_y = ch.title_y;

        let frame = UiFrame::new();

        let mut quads: Vec<GpuInstance> = Vec::new();
        let mut text_labels: Vec<TextLabel> = Vec::new();

        // Title — pinned font so long couch viewing doesn't auto-shrink glyphs.
        let title_font_px = typography::size(typography::TITLE, h).max(30.0);
        let title_h = (title_font_px / 0.55).ceil() + 8.0;
        text_labels.push(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: format!("Archive — {}", self.active_tab.label()),
            color: color::CHAMPAGNE,
            font_px: Some(title_font_px),
            ..Default::default()
        });

        // Career frieze: body tier + generous line height (TV / wide couch).
        let frieze_font_px = typography::size(typography::BODY, h).max(22.0);
        let frieze_line_h = (frieze_font_px / 0.55).ceil() + 6.0;
        let frieze_gap = (h * 0.018).max(12.0);
        let frieze_top = title_y + title_h + h * 0.012;
        for (i, line) in archive_career::career_frieze_lines(ctx.progress)
            .into_iter()
            .take(4)
            .enumerate()
        {
            text_labels.push(TextLabel {
                rect: [
                    margin_x,
                    frieze_top + i as f32 * (frieze_line_h + frieze_gap),
                    w - margin_x * 2.0,
                    frieze_line_h,
                ],
                text: line,
                color: [0.86, 0.84, 0.78, 0.96],
                font_px: Some(frieze_font_px),
                align: TextAlign::Left,
                ..Default::default()
            });
        }
        if matches!(self.active_tab, Tab::Chronicle) {
            let note_y =
                frieze_top + 4.0 * (frieze_line_h + frieze_gap) + (h * 0.014).max(10.0);
            text_labels.push(TextLabel {
                rect: [margin_x, note_y, w - margin_x * 2.0, frieze_line_h],
                text: archive_career::CHRONICLE_TUTORIAL_NOTE.into(),
                color: [0.74, 0.76, 0.84, 0.92],
                font_px: Some(frieze_font_px),
                align: TextAlign::Left,
                ..Default::default()
            });
        }

        // Back button.
        let back_w = ch.back_w;
        let back_h = ch.chrome_btn_h;
        let btn_label_px = typography::size(typography::BODY, h).max(21.0);
        quads.push(GpuInstance {
            rect: [margin_x, title_y, back_w, back_h],
            color: [0.18, 0.20, 0.30, 0.92],
        });
        text_labels.push(TextLabel {
            rect: [margin_x, title_y, back_w, back_h],
            text: "< Back".into(),
            color: [0.92, 0.92, 0.98, 1.0],
            font_px: Some(btn_label_px),
            ..Default::default()
        });

        let switch_w = ch.switch_w;
        let switch_x = w - margin_x - switch_w;
        quads.push(GpuInstance {
            rect: [switch_x, title_y, switch_w, back_h],
            color: [0.18, 0.20, 0.30, 0.92],
        });
        text_labels.push(TextLabel {
            rect: [switch_x, title_y, switch_w, back_h],
            text: "Switch save".into(),
            color: [0.92, 0.92, 0.98, 1.0],
            font_px: Some(btn_label_px),
            ..Default::default()
        });

        // Footer Prev/Next tab arrows. Match the rects from `flat_items`
        // so the click hit-testing and the visual button line up.
        let arrow_w = ch.arrow_w;
        let arrow_h = back_h;
        let arrow_y = h - arrow_h - h * 0.02;
        let prev_x = w * 0.5 - arrow_w * 1.5;
        let next_x = w * 0.5 + arrow_w * 0.5;
        for (x, sym) in [(prev_x, "<"), (next_x, ">")] {
            quads.push(GpuInstance {
                rect: [x, arrow_y, arrow_w, arrow_h],
                color: [0.18, 0.20, 0.30, 0.92],
            });
            text_labels.push(TextLabel {
                rect: [x, arrow_y, arrow_w, arrow_h],
                text: sym.into(),
                color: [0.92, 0.92, 0.98, 1.0],
                font_px: Some(btn_label_px * 1.15),
                ..Default::default()
            });
        }

        // Control hints — sits just above the tab arrows so keyboard /
        // controller bindings are discoverable without a separate help
        // overlay. The grid scroll affordance only mentions PgUp/PgDn
        // when the active tab actually has rows beyond the viewport;
        // otherwise the arrow-only hint is enough.
        let all_count_hint = tab_artifacts(self.active_tab, ctx.progress).len();
        let tab_scrollable = (total_rows_for(all_count_hint) as usize) > 0 && {
            let probe = compute_layout(w, h, scale, self.active_tab, all_count_hint);
            (probe.grid_rows as usize) > probe.visible_rows as usize
        };
        // TV: pinned body size + multi-line copy so width-based auto-shrink
        // never drives hints to microtext.
        let hint_font_px = typography::size(typography::BODY, h).max(22.0);
        let hint_line_h = (hint_font_px / 0.55).ceil() + 4.0;
        let hint_text: String = if inspect.is_some() {
            "Right stick: orbit camera\nTriggers / scroll: zoom   ·   E / North: close   ·   Esc: menu"
                .to_string()
        } else if matches!(self.active_tab, Tab::Chronicle) && all_count_hint == 0 {
            "Finish a non-tutorial run to add folios here.\nTab / Shift+Tab: tabs   ·   Esc: back"
                .to_string()
        } else if tab_scrollable {
            "Tab / Shift+Tab: cycle tab   ·   \u{2190}\u{2192}\u{2191}\u{2193}: focus   ·   Enter: pedestal   ·   E/North: inspect   ·   Esc: back\nScroll: mouse wheel or PgUp / PgDn"
                .to_string()
        } else {
            "Tab / Shift+Tab: cycle tab   ·   \u{2190}\u{2192}\u{2191}\u{2193}: focus\nEnter: pedestal   ·   E/North: inspect   ·   Esc: back"
                .to_string()
        };
        let hint_lines = hint_text.lines().count().max(1) as f32;
        let hint_h = hint_line_h * hint_lines + 10.0;
        let hint_y = arrow_y - hint_h - (h * 0.014).max(10.0);
        text_labels.push(TextLabel {
            rect: [margin_x * 0.5, hint_y, w - margin_x, hint_h],
            text: hint_text,
            color: [0.78, 0.80, 0.88, 0.92],
            font_px: Some(hint_font_px),
            align: TextAlign::Center,
            ..Default::default()
        });

        // Stake ladder readout: highest stake cleared per tile material.
        // Uses the same caption size as the hint line and sits just above
        // it. Materials the player has never cleared even Spring on are
        // omitted — the line is decorative, not a checklist.
        let ladder_line = stake_ladder_summary(ctx.progress);
        if !ladder_line.is_empty() {
            let ladder_band_h = hint_line_h + 8.0;
            let ladder_y = hint_y - ladder_band_h - (h * 0.014).max(10.0);
            text_labels.push(TextLabel {
                rect: [margin_x * 0.5, ladder_y, w - margin_x, ladder_band_h],
                text: ladder_line.into(),
                color: [0.90, 0.80, 0.52, 0.95],
                font_px: Some(hint_font_px),
                align: TextAlign::Center,
                ..Default::default()
            });
        }

        // Gather active-tab artifacts once — feeds the grid layout, the
        // pedestal featured item, and the description plaque.
        let all_artifacts = tab_artifacts(self.active_tab, ctx.progress);

        // Infinite curio corridor: every tab renders as a procedural
        // cabinet grid with a tinted-accent nameplate per entry and a
        // description card floating in front of the focused cell.
        self.build_corridor_frame(frame, quads, text_labels, &all_artifacts, ctx, inspect)
    }
}

impl SceneBehavior for CollectionScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let len = ctx.progress.run_history.len() as u32;
        if self.chronicle_seen_cursor != Some(len) {
            self.chronicle_seen_cursor = Some(len);
            *ctx.bump_archive_chronicle_seen = Some(len);
        }

        let items = self.flat_items(
            ctx.layout.window_w,
            ctx.layout.window_h,
            ctx.progress,
        );
        // Keyboard / directional / Confirm are handled below by the
        // scene's own 2D grid navigator, which reads `self.focused_row`.
        // The tree only hit-tests mouse clicks and tracks hover focus —
        // forwarding `ctx.actions` here would let the tree's 1D focus
        // race the scene's 2D focus, so Confirm would lift whatever the
        // tree happened to focus (usually not the visibly-focused cell).
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: &[],
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }

        // Keyboard / controller / wheel navigation:
        //   - Triggers / Tab / Shift+Tab → cycle tabs (outer axis)
        //   - L/R arrows + DPad L/R → move focus across columns
        //   - Up/Down arrows + DPad U/D → move focus across rows;
        //     auto-scrolls the viewport when focus exits the band
        //   - Mouse wheel → scroll one row per line tick
        //   - Confirm (A / Space / Enter) → lift focused item onto pedestal
        let all_count = tab_artifacts(self.active_tab, ctx.progress).len();
        let cols = GRID_COLS as usize;
        let total_rows = total_rows_for(all_count) as usize;
        let visible_rows = {
            // Recompute the same visible-row count `compute_layout`
            // uses so focus-follow scrolling matches what's drawn.
            let layout = compute_layout(
                ctx.layout.window_w,
                ctx.layout.window_h,
                1.0,
                self.active_tab,
                all_count,
            );
            layout.visible_rows as usize
        };
        let max_scroll = total_rows.saturating_sub(visible_rows) as f32;

        // Mouse wheel: 1 row per line tick. Negative scroll_lines (wheel
        // up) moves the content down → reduce scroll_rows.
        if ctx.scroll_lines.abs() > 0.001 && max_scroll > 0.0 {
            let next = (self.target_scroll_rows.get() + ctx.scroll_lines).clamp(0.0, max_scroll);
            self.target_scroll_rows.set(next);
        }

        // Resolve the focused item's global (row, col) so directional
        // navigation has a starting cell. Defaults to (0, 0) when the
        // player hasn't focused anything yet.
        let cur_row_col = self.focused_row.and_then(|i| {
            if i < all_count {
                Some((i / cols, i % cols))
            } else {
                None
            }
        });
        // Scroll the viewport so `row` sits inside the visible band.
        // No-op when already visible. Used after every directional move.
        let scroll_row_into_view = |scene: &Self, row: usize| {
            if max_scroll <= 0.0 {
                return;
            }
            let top = scene.target_scroll_rows.get();
            let bottom = top + visible_rows.saturating_sub(1) as f32;
            let row_f = row as f32;
            let next = if row_f < top {
                row_f
            } else if row_f > bottom {
                row_f - visible_rows.saturating_sub(1) as f32
            } else {
                top
            };
            scene.target_scroll_rows.set(next.clamp(0.0, max_scroll));
        };
        // Clamp a (row, col) candidate to the last actually-present
        // artifact (the final row may be partially filled).
        let global_idx = |row: usize, col: usize| -> Option<usize> {
            if all_count == 0 {
                return None;
            }
            let cand = row * cols + col;
            Some(cand.min(all_count - 1))
        };

        for a in ctx.actions {
            match a {
                UiAction::Cancel | UiAction::Pause | UiAction::CommitDiscard => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    return Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()));
                }
                UiAction::TabNext => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    self.cycle_tab(true);
                }
                UiAction::TabPrev => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    self.cycle_tab(false);
                }
                // PageUp/PageDown still scroll the viewport — by one
                // visible-page worth. Useful as a power-user shortcut
                // even though the primary navigation is the arrows.
                UiAction::PageNext => {
                    if max_scroll > 0.0 {
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                        let next = (self.target_scroll_rows.get() + visible_rows as f32)
                            .clamp(0.0, max_scroll);
                        self.target_scroll_rows.set(next);
                    }
                }
                UiAction::PagePrev => {
                    if max_scroll > 0.0 {
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                        let next = (self.target_scroll_rows.get() - visible_rows as f32)
                            .clamp(0.0, max_scroll);
                        self.target_scroll_rows.set(next);
                    }
                }
                UiAction::FocusNext => {
                    if all_count == 0 {
                        continue;
                    }
                    let (row, col) = cur_row_col.unwrap_or((0, 0));
                    let next_col = (col + 1).min(cols - 1);
                    if next_col != col
                        && let Some(i) = global_idx(row, next_col)
                        && Some(i) != self.focused_row
                    {
                        ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                        self.focused_row = Some(i);
                        scroll_row_into_view(self, i / cols);
                    }
                }
                UiAction::FocusPrev => {
                    if all_count == 0 {
                        continue;
                    }
                    let (row, col) = cur_row_col.unwrap_or((0, 0));
                    if col > 0
                        && let Some(i) = global_idx(row, col - 1)
                    {
                        ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                        self.focused_row = Some(i);
                        scroll_row_into_view(self, i / cols);
                    }
                }
                UiAction::FocusUp => {
                    if all_count == 0 || total_rows <= 1 {
                        continue;
                    }
                    let (row, col) = cur_row_col.unwrap_or((0, 0));
                    if row > 0
                        && let Some(i) = global_idx(row - 1, col)
                    {
                        ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                        self.focused_row = Some(i);
                        scroll_row_into_view(self, i / cols);
                    }
                }
                UiAction::FocusDown => {
                    if all_count == 0 || total_rows <= 1 {
                        continue;
                    }
                    let (row, col) = cur_row_col.unwrap_or((0, 0));
                    let next_row = (row + 1).min(total_rows - 1);
                    if next_row != row
                        && let Some(i) = global_idx(next_row, col)
                        && Some(i) != self.focused_row
                    {
                        ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                        self.focused_row = Some(i);
                        scroll_row_into_view(self, i / cols);
                    }
                }
                UiAction::Confirm => {
                    if let Some(i) = self.focused_row {
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                        self.selected_artifact = Some(i);
                        push_relic_stinger_for(ctx.bus, self.active_tab, ctx.progress, i);
                    }
                }
                UiAction::NorthFacePress => {
                    let w = ctx.layout.window_w;
                    let h = ctx.layout.window_h;
                    let bosses = tab_artifacts(self.active_tab, ctx.progress);
                    if bosses.is_empty() {
                        continue;
                    }
                    if let Some(orbit) =
                        self.collection_inspect_orbit_for_focus(w, h, &bosses, ctx.layout)
                    {
                        *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(
                            Scene::Showcase(crate::scenes::ShowcaseScene::new(
                                crate::scenes::ShowcasePresenter::CollectionInspect(
                                    crate::scenes::CollectionInspectPresenter::new(orbit),
                                ),
                            )),
                        )));
                    }
                }
                _ => {}
            }
        }

        match action {
            Some(CollectionAction::Back) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                return Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()));
            }
            Some(CollectionAction::SwitchSave) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                return Some(Scene::ProfileSelect(
                    ProfileSelectScene::from_archive_switch_save(),
                ));
            }
            Some(CollectionAction::PrevTab) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                self.cycle_tab(false);
            }
            Some(CollectionAction::NextTab) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                self.cycle_tab(true);
            }
            Some(CollectionAction::SelectArtifact(idx)) => {
                // Relic cells have a per-triangle trimesh picker running
                // each frame; when it reports a hit, prefer that index
                // over the loose cell rect so clicks that land in the
                // empty space around a relic's silhouette don't select
                // the wrong artifact. For non-relic cells (talismans /
                // zodiacs / plaques) the trimesh picker stays silent
                // and the flat cell rect remains the source of truth.
                let resolved = ctx
                    .picked_collection_object
                    .map(|pid| pid as usize)
                    .unwrap_or(idx);
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                self.selected_artifact = Some(resolved);
                push_relic_stinger_for(ctx.bus, self.active_tab, ctx.progress, resolved);
                // Mouse click also moves keyboard focus so subsequent
                // arrow-key navigation continues from the clicked item
                // instead of teleporting back to position 0.
                self.focused_row = Some(resolved);
                // Keep the viewport synced to the clicked row so the
                // focus halo and the visible slot agree.
                scroll_row_into_view(self, resolved / cols);
            }
            None => {}
        }
        // Advance the scroll easing every tick so wheel/key inputs
        // glide rather than snap.
        let _ = self.tick_scroll();
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        self.draw_collection_frame(ctx, None)
    }
}

// ── Stake ladder readout ────────────────────────────────────────────

/// Format a single-line "highest stake cleared per material" summary for
/// the footer of the Collection scene. Materials with no cleared stakes
/// are omitted so the line stays compact on fresh profiles.
fn stake_ladder_summary(progress: &crate::core::progression::PlayerProgress) -> String {
    let materials = [
        crate::persistence::TileMaterial::Bamboo,
        crate::persistence::TileMaterial::Plastic,
        crate::persistence::TileMaterial::TortoiseShell,
    ];
    let mut parts: Vec<String> = Vec::new();
    for m in materials {
        let cleared = progress.stakes_cleared_for(m);
        if let Some(highest) = cleared.iter().max() {
            parts.push(format!("{}: {}", m.label(), highest.label()));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("Stakes cleared — {}", parts.join("   ·   "))
    }
}

// ── Tab artifact enumeration ────────────────────────────────────────

/// Build the list of artifacts for one tab. Reads
/// [`PlayerProgress`] to mark which items are unlocked; everything in
/// the universe of that category is returned (locked items show as
/// placeholders in the row so the player can see what's still to find).
fn push_relic_stinger_for(
    bus: &mut crate::game::event_bus::EventBus,
    tab: Tab,
    progress: &crate::core::progression::PlayerProgress,
    idx: usize,
) {
    if !matches!(tab, Tab::Relics) {
        return;
    }
    let arts = tab_artifacts(tab, progress);
    if let Some(art) = arts.get(idx)
        && art.unlocked
        && let ArtifactKind::Relic(rid) = art.kind
    {
        bus.push(GameEvent::PlayRelicStinger(rid));
    }
}

fn tab_artifacts(tab: Tab, progress: &crate::core::progression::PlayerProgress) -> Vec<Artifact> {
    match tab {
        Tab::Relics => {
            let defs = all_relic_defs();
            let available = progress.available_relics();
            defs.iter()
                .filter(|d| progress.transformation_successor_visible(d.id))
                .map(|d| Artifact {
                    name: d.name.to_string(),
                    unlocked: available.contains(&d.id),
                    kind: ArtifactKind::Relic(d.id),
                    accent: rarity_accent(d.rarity),
                })
                .collect()
        }
        Tab::Yaku => YakuKind::all()
            .iter()
            .filter(|yk| progress.yaku_times_scored.contains_key(yk))
            .map(|&yk| Artifact {
                name: yk.name().to_string(),
                unlocked: true,
                kind: ArtifactKind::Zodiac(zodiac_for_yaku(yk)),
                accent: color::CHAMPAGNE,
            })
            .collect(),
        Tab::Bosses => all_bosses()
            .iter()
            .chain(final_bosses().iter())
            .filter(|def| progress.boss_times_encountered.contains_key(&def.kind))
            .map(|def| Artifact {
                name: def.name.to_string(),
                unlocked: true,
                kind: ArtifactKind::PlaqueOnly,
                accent: def.tier.halo_color(),
            })
            .collect(),
        Tab::Talismans => TalismanKind::all()
            .iter()
            .filter(|tk| progress.talisman_times_purchased.contains_key(tk))
            .map(|&tk| Artifact {
                name: tk.name().to_string(),
                unlocked: true,
                kind: ArtifactKind::Talisman(tk),
                accent: tk.accent_color(),
            })
            .collect(),
        Tab::Chronicle => archive_career::chronicle_indices_recent_first(progress)
            .into_iter()
            .filter_map(|idx| {
                let rec = progress.run_history.get(idx)?;
                Some(Artifact {
                    name: archive_career::chronicle_folio_title(progress, rec),
                    unlocked: true,
                    kind: ArtifactKind::ChronicleRun(idx),
                    accent: color::PARCHMENT,
                })
            })
            .collect(),
    }
}

/// Map a yaku to its matching zodiac ribbon. Every yaku has exactly one
/// ribbon that levels it up, so this is total. Inverse of
/// `ZodiacKind::yaku`.
fn zodiac_for_yaku(yk: YakuKind) -> ZodiacKind {
    ZodiacKind::all()
        .iter()
        .copied()
        .find(|z| z.yaku() == yk)
        .expect("every YakuKind has a matching ZodiacKind")
}

/// Pull a human-readable description for the description plaque. Locked
/// artifacts show a mystery prompt; unlocked ones defer to each core
/// module's own description method (with live-counter support for
/// relics). Yaku entries describe the hand + the zodiac ribbon that
/// levels it up — the two live on one catalog card now.
fn description_for(
    art: &Artifact,
    run: &crate::game::run::RunState,
    progress: &crate::core::progression::PlayerProgress,
) -> String {
    if !art.unlocked {
        return "Locked — keep playing to reveal this entry.".to_string();
    }
    match &art.kind {
        ArtifactKind::Relic(id) => crate::core::relic::relic_description_live(
            *id,
            &run.relic_counters,
            run.total_score_earned,
            None,
            Some(run.ghost_hand_preview_chips()),
        ),
        ArtifactKind::Talisman(kind) => kind.description().to_string(),
        ArtifactKind::Zodiac(kind) => format!(
            "Levelled by the {} zodiac ribbon (+0.5 mult, +20 chips per level).",
            kind.name()
        ),
        ArtifactKind::PlaqueOnly => {
            // Bosses route here (yaku now render as ribbons). Bosses
            // carry their own description text.
            boss_by_name(&art.name)
                .map(str::to_string)
                .unwrap_or_default()
        }
        ArtifactKind::ChronicleRun(idx) => progress
            .run_history
            .get(*idx)
            .map(archive_career::chronicle_run_description)
            .unwrap_or_else(|| "Missing run record.".to_string()),
    }
}

fn boss_by_name(name: &str) -> Option<&'static str> {
    all_bosses()
        .iter()
        .chain(final_bosses().iter())
        .find(|def| def.name == name)
        .map(|def| def.description)
}

/// Profile-wide counters for the focused artifact, formatted for the
/// stats plaque. Lines are `Label: value` pairs separated by newlines;
/// counters at zero are omitted so fresh-profile entries don't render a
/// wall of "Times X: 0". Empty string → no plaque.
fn stats_for(art: &Artifact, progress: &crate::core::progression::PlayerProgress) -> String {
    let mut lines: Vec<String> = Vec::new();
    match &art.kind {
        ArtifactKind::Relic(id) => {
            if let Some(n) = progress.relic_times_activated.get(id).copied() {
                lines.push(format!("Activated: {}", n));
            }
        }
        ArtifactKind::Zodiac(zk) => {
            let yk = zk.yaku();
            if let Some(n) = progress.yaku_times_scored.get(&yk).copied() {
                lines.push(format!("Scored: {}", n));
            }
        }
        ArtifactKind::Talisman(tk) => {
            if let Some(n) = progress.talisman_times_purchased.get(tk).copied() {
                lines.push(format!("Purchased: {}", n));
            }
            if let Some(n) = progress.talisman_times_used.get(tk).copied() {
                lines.push(format!("Used: {}", n));
            }
        }
        ArtifactKind::PlaqueOnly => {
            // Bosses route here. Look up the kind by name since the
            // Artifact doesn't carry it for plaque-only entries.
            if let Some(def) = all_bosses()
                .iter()
                .chain(final_bosses().iter())
                .find(|d| d.name == art.name)
            {
                if let Some(n) = progress.boss_times_encountered.get(&def.kind).copied() {
                    lines.push(format!("Encountered: {}", n));
                }
                if let Some(n) = progress.boss_times_defeated.get(&def.kind).copied() {
                    lines.push(format!("Defeated: {}", n));
                }
            }
        }
        ArtifactKind::ChronicleRun(idx) => {
            if let Some(rec) = progress.run_history.get(*idx) {
                return archive_career::chronicle_run_stats(rec);
            }
        }
    }
    lines.join("\n")
}

// ── Camera projection helpers ───────────────────────────────────────

/// Mirror of `crate::render::world_space::pixel_to_world` for the
/// scene's local use — keeps the scene independent of the renderer
/// module while ensuring positions agree with what the renderer emits.
fn pixel_to_world_xy(w: f32, h: f32, px: f32, py: f32, lift: f32) -> glam::Vec3 {
    glam::Vec3::new(px - w * 0.5, h * 0.5 - py, lift)
}

/// Compose the same view-projection matrix the renderer uses (must
/// match `WgpuRenderer`'s perspective + look_at_rh path: near=1.0,
/// far=h*12.0). Drift here makes hit rects misalign with visible 3D.
fn camera_view_proj(w: f32, h: f32, cam: &CameraParams) -> glam::Mat4 {
    let aspect = w / h;
    let view = glam::Mat4::look_at_rh(
        glam::Vec3::from_array(cam.eye),
        glam::Vec3::from_array(cam.target),
        glam::Vec3::from_array(cam.up),
    );
    let proj = glam::Mat4::perspective_rh(cam.fovy_deg.to_radians(), aspect, 1.0, h * 12.0);
    proj * view
}

/// Project a world-space point to screen pixels.
fn world_to_screen(view_proj: glam::Mat4, w: f32, h: f32, world: glam::Vec3) -> (f32, f32) {
    let clip = view_proj * glam::Vec4::new(world.x, world.y, world.z, 1.0);
    let inv_w = 1.0 / clip.w.max(1e-6);
    let nx = clip.x * inv_w;
    let ny = clip.y * inv_w;
    let sx = (nx * 0.5 + 0.5) * w;
    let sy = (1.0 - (ny * 0.5 + 0.5)) * h;
    (sx, sy)
}

/// Single source of truth for the scene's spatial layout. Both
/// `flat_items` (hit rects) and `draw_frame` (visual placement) read
/// from this so clicks line up with what's drawn.
struct SceneLayout {
    /// Total grid rows (= ceil(item_count / 6)). The grid is always 6-wide.
    grid_rows: u32,
    /// Always `GRID_COLS`. Kept as a layout field so consumers don't
    /// need to know the constant exists.
    grid_cols: u32,
    /// Number of *fully* visible rows in the band. Half a row peeks
    /// above and below this when scrolled, signalling more content.
    visible_rows: u32,
    /// Pixel x of column 0 center.
    grid_x_start: f32,
    /// Pixel y of the *unscrolled* row-0 center (i.e. row 0 sitting at
    /// the top of the band). `draw_frame` and `flat_items` apply
    /// `scroll_rows * row_pitch` on top of this when placing each row.
    grid_y_top: f32,
    /// Grid cell pitch (width = height, square-ish).
    grid_cell_w: f32,
    grid_row_pitch: f32,
    /// Pixel-y top/bottom of the visible scroll band. Cells outside
    /// this range (with a half-cell margin) are clipped from both the
    /// hit-test list and the draw call.
    band_top_y: f32,
    band_bottom_y: f32,
    /// World-Z lift each artifact's shelf surface sits at.
    shelf_top_lift: f32,
    /// Camera params used by the scene this frame.
    camera: CameraParams,
}

/// Fixed column count for every tab. The grid scrolls vertically — see
/// `compute_layout` for how visible row count is derived from the band.
const GRID_COLS: u32 = 6;

/// Total rows the active tab's grid spans given its artifact count.
/// Always 6-wide; the row count grows to fit the universe.
fn total_rows_for(count: usize) -> u32 {
    if count == 0 {
        0
    } else {
        (count as u32).div_ceil(GRID_COLS)
    }
}

fn compute_layout(w: f32, h: f32, scale: f32, _tab: Tab, item_count: usize) -> SceneLayout {
    let _ = scale;
    let title_y = h * 0.02;
    let title_h = ((24.0_f32 * (w.min(h) / 600.0)).max(14.0) / 0.55).ceil();
    let arrow_h = ((24.0 * (w.min(h) / 600.0)).max(18.0)).max(18.0);
    let arrow_y = h - arrow_h - h * 0.02;

    // ── Shelf band: foreground scroll viewport ───────────────────────
    // The grid is always 6 columns wide and grows vertically with the
    // tab's universe. The visible band shows ~3.5 rows so the
    // half-row above/below is the affordance that more content
    // exists. Cells are square; their size is the smaller of "fit 6
    // across" or "fit visible_rows + 0.5 stacked".
    let grid_rows_total = total_rows_for(item_count);
    let grid_cols = GRID_COLS;
    let margin_x = w * 0.06;
    // Tight top margin + smaller pedestal band → taller scroll
    // band → larger cells. Browsing is the primary verb on this
    // screen; the inspection pedestal supports it but shouldn't
    // dominate.
    let grid_y_top_band = title_y + title_h + h * 0.14;
    let pedestal_band_h = h * 0.10;
    let grid_y_bottom_band = arrow_y - pedestal_band_h;
    let band_h = (grid_y_bottom_band - grid_y_top_band).max(1.0);
    let usable_w = w - margin_x * 2.0;
    // Visible-row count + cell size. We want at least ~3 rows visible
    // with a 0.3-row peek below to signal scrollable content. If the
    // band is tall enough to fit more rows at the width-driven cell
    // size, expand visible_rows so cells stay square and the band
    // doesn't have empty space above the grid.
    let target_visible_rows: u32 = 3;
    let cell_from_width = usable_w / grid_cols as f32;
    // 0.3-row bottom peek when scrollable, none when the whole tab
    // fits. Computed against the *target* row count first so the
    // expansion below knows whether to leave peek room.
    let peek_target = if grid_rows_total > target_visible_rows {
        0.3
    } else {
        0.0
    };
    // How many rows fit at the width-driven cell size (with peek).
    let max_rows_at_width = ((band_h / cell_from_width) - peek_target).floor() as u32;
    let mut visible_rows_u = target_visible_rows.max(max_rows_at_width).max(1);
    if grid_rows_total > 0 && grid_rows_total <= visible_rows_u {
        visible_rows_u = grid_rows_total;
    }
    let peek = if grid_rows_total > visible_rows_u {
        0.3
    } else {
        0.0
    };
    let cell_from_height = band_h / (visible_rows_u as f32 + peek);
    let cell_w = cell_from_width.min(cell_from_height);
    let grid_cell_w = cell_w;
    let grid_row_pitch = cell_w;
    let total_grid_w = cell_w * grid_cols as f32;
    let visible_grid_h = cell_w * (visible_rows_u as f32 + peek);
    let grid_x_start = w * 0.5 - total_grid_w * 0.5 + cell_w * 0.5;
    // Place row 0 at the top of the visible band, accounting for the
    // peek margin so the top half-row is visible above row 0 only
    // when scrolled (scroll offset > 0). When unscrolled, row 0
    // sits flush with `grid_y_top_band + cell*0.5`.
    let grid_y_top = grid_y_top_band + cell_w * 0.5;
    let band_top_y = grid_y_top_band;
    let band_bottom_y = grid_y_top_band + visible_grid_h;

    // Anchor only kept for the hit-test ray cast in `flat_items`. The
    // corridor scene draws its own stage — this value is never used
    // for 3D placement there.
    let shelf_top_lift = cell_w * 0.15;

    let cam_dist = h * 1.6;
    let cam_height = h * 1.3;
    let camera = CameraParams {
        eye: [0.0, -cam_dist, cam_height],
        target: [0.0, h * 0.05, h * 0.10],
        up: [0.0, 0.0, 1.0],
        fovy_deg: 36.0,
    };

    SceneLayout {
        grid_rows: grid_rows_total,
        grid_cols,
        visible_rows: visible_rows_u,
        grid_x_start,
        grid_y_top,
        grid_cell_w,
        grid_row_pitch,
        band_top_y,
        band_bottom_y,
        shelf_top_lift,
        camera,
    }
}

fn rarity_accent(r: Rarity) -> [f32; 4] {
    match r {
        Rarity::Common => [0.65, 0.65, 0.70, 1.0],
        Rarity::Uncommon => [0.55, 0.85, 0.65, 1.0],
        Rarity::Rare => [0.55, 0.70, 0.95, 1.0],
        Rarity::Legendary => [0.95, 0.75, 0.45, 1.0],
    }
}
