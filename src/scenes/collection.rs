//! Archive — five tabs (Relics / Yaku / Bosses / Talismans / Chronicle).
//! Each tab is a scrolling grid of artifacts on a backdrop plane; focus
//! shows a close-up and description on the room's sign boards (or a floating card when
//! inspecting or using the procedural layout). E / North opens orbit inspect.

use std::time::Instant;

use crate::audio::SfxId;
use crate::core::boss::{all_bosses, final_bosses};
use crate::core::progression::is_transformation_successor_relic;
use crate::core::relic::{RelicId, all_relic_defs};
use crate::core::talisman::TalismanKind;
use crate::core::yaku::YakuKind;
use crate::core::zodiac::ZodiacKind;
use crate::game::event_bus::GameEvent;
use crate::render::archive_glb;
use crate::render::draw_cmd::{
    CameraParams, Object3d, Object3dKind, ScenePunctualLight, UiFrame, camera_facing_euler_xyz_rad,
};
use crate::render::ribbon_mesh::{ZodiacRibbonSpec, zodiac_ribbon_object3d};
use crate::render::room_glb;
use crate::render::table_transform::{euler_xyz_rad_from_deg, mat4_to_euler_xyz_rad, rot_fixed_axes_deg};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{
    GpuInstance, GradientQuadInstance, PointLight, TextAlign, TextLabel,
};
use crate::render::world_space::{
    object3d_pos_for_screen_at_world_z, surface_anchor_from_world_xyz, world_on_camera_ray_plane_z,
};
use crate::ui::focus_nav::{FocusDir, pick_neighbor, push_focus_ring};
use crate::ui::input::{InputMode, UiAction};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};
use glam::{Mat4, Quat, Vec3};

use super::archive_career;
use super::main_menu_exterior::MainMenuExteriorScene;
use super::profile_select::ProfileSelectScene;
use super::{DrawCtx, OverlayRequest, Scene, SceneBehavior, SceneTransition, UpdateCtx};
use crate::scenes::object3d_inspect::{
    InspectDolly, InspectFrameEnv, InspectRig, ItemInspectOrbitState, apply_inspect_view_to_frame,
    ease_in_out_cubic, inspect_orbit_camera, lerp_camera, prepend_inspect_orbit_subject_rotation,
    tick_inspect_dolly,
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
    /// Aggregate career view (run-log row 0).
    ChronicleSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CollectionAction {
    Back,
    SwitchSave,
    PrevTab,
    NextTab,
    /// Direct tab pick when `archive.glb` provides section button bounds.
    SelectTab(usize),
    /// Click on an artifact in the current tab's row → set as the
    /// featured item for [`ItemInspectScene`] orbit. Indexes into the
    /// active tab's artifact list globally so the selection survives
    /// scroll position changes.
    SelectArtifact(usize),
    /// Footer arrow paginates the archive cabinet by one page (only used when the
    /// `archive.glb` room is loaded; procedural fallback uses the arrows for tabs).
    PrevPage,
    NextPage,
}

impl CollectionAction {
    fn id(self) -> FocusId {
        match self {
            CollectionAction::Back => FocusId(20),
            CollectionAction::SwitchSave => FocusId(23),
            CollectionAction::PrevTab => FocusId(21),
            CollectionAction::NextTab => FocusId(22),
            CollectionAction::PrevPage => FocusId(24),
            CollectionAction::NextPage => FocusId(25),
            CollectionAction::SelectTab(i) => FocusId(400 + i as u32),
            // SelectArtifact IDs start at 200. The widget tree just needs
            // unique IDs per hit target — the values themselves don't matter.
            CollectionAction::SelectArtifact(i) => FocusId(200 + i as u32),
        }
    }
}

pub struct CollectionScene {
    tree: TreeState,
    /// Arrange-mode-tunable placements for the grid, focus plaques, and
    /// inspect-orbit anchor pose.
    pub positions: crate::ui::scene_layout::CollectionPositions,
    /// Currently-selected tab. Determines which content row sits on the
    /// table.
    active_tab: Tab,
    /// Index into the active tab's visible row of the artifact featured for
    /// orbit inspect. `None` falls back to the first unlocked artifact for the
    /// tab. Set by mouse click or by pressing Confirm while a row item is
    /// focused; reset on tab change so stale indices don't bleed across tabs.
    selected_artifact: Option<usize>,
    /// Row item that currently has keyboard / controller focus (the
    /// arrow-key / DPad cursor). `None` until the player first uses
    /// directional input. Confirm sets the focused row as the orbit-inspect
    /// target (`selected_artifact`).
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
    /// Current page index when the `archive.glb` room is loaded. The cabinet shows
    /// `ARCHIVE_SLOT_COUNT` items per page; navigation pages flip rather than
    /// horizontally / vertically scroll a sliding window. Procedural fallback
    /// ignores this and uses [`Self::target_scroll_rows`] instead.
    archive_page: usize,
    /// Last drawn [`DrawCtx::room_gltf_height_scale`] — matches GPU glTF room scale when `flat_items` /
    /// `update` build marker hit rects (possibly one frame behind).
    drawn_room_gltf_height_scale: std::cell::Cell<f32>,
    /// Cubic ease-in-out blend between the resting grid camera and the orbit-inspect
    /// camera. `phase` advances linearly toward `1.0` while inspect is active and back
    /// toward `0.0` once the overlay pops; the lerp into [`build_archive_grid_frame`]
    /// uses [`ease_in_out_cubic`] to dolly without snapping in either direction.
    inspect_dolly: std::cell::Cell<InspectDolly>,
    /// Most recent inspect-orbit camera. Frozen each frame the inspect overlay is alive
    /// so the dolly-out can keep sampling the inspect endpoint after the presenter (and
    /// its [`ItemInspectOrbitState`]) has been dropped.
    last_inspect_cam: std::cell::Cell<Option<CameraParams>>,
    /// When `Some`, controller / keyboard focus is parked on a chrome button
    /// (Back / Switch save / footer Prev / Next) rather than on an artifact.
    /// Pressing Up from the top row of the cabinet enters the title bar;
    /// pressing Down from the bottom row enters the footer. Cleared by tab /
    /// page changes and any move back into the artifact grid. The artifact
    /// selection in `focused_row` is preserved so direction-reversal lands on
    /// the same cell.
    focused_chrome: Option<CollectionAction>,
    /// Vertical scroll (px) for the Chronicle career pane (right).
    chronicle_dashboard_scroll: std::cell::Cell<f32>,
    /// Vertical scroll (px) for the Chronicle run log (left).
    chronicle_run_log_scroll: std::cell::Cell<f32>,
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

impl CollectionScene {
    pub fn new() -> Self {
        Self::with_active_tab(Tab::Relics)
    }

    /// Headless screenshot: open the Chronicle tab with a clean scroll/camera state.
    pub fn prepare_chronicle_for_screenshot(&mut self) {
        *self = Self::with_active_tab(Tab::Chronicle);
    }

    fn with_active_tab(active_tab: Tab) -> Self {
        Self {
            tree: TreeState::new(),
            positions: crate::ui::scene_layout::CollectionPositions::default(),
            active_tab,
            selected_artifact: None,
            focused_row: Some(0),
            scroll_rows: std::cell::Cell::new(0.0),
            target_scroll_rows: std::cell::Cell::new(0.0),
            scroll_last_tick: std::cell::Cell::new(Instant::now()),
            cam_anim: std::cell::Cell::new(None),
            chronicle_seen_cursor: None,
            archive_page: 0,
            drawn_room_gltf_height_scale: std::cell::Cell::new(1.0),
            inspect_dolly: std::cell::Cell::new(InspectDolly {
                phase: 0.0,
                last_tick: Instant::now(),
            }),
            last_inspect_cam: std::cell::Cell::new(None),
            focused_chrome: None,
            chronicle_dashboard_scroll: std::cell::Cell::new(0.0),
            chronicle_run_log_scroll: std::cell::Cell::new(0.0),
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

    /// Chrome action that should currently render a focus ring. Controller /
    /// keyboard mode uses `focused_chrome`; cursor mode reads the tree's hover
    /// target so mouse users see the same brass ring when they're over a
    /// chrome button. Returns `None` when no chrome button should be ringed.
    fn chrome_focus_for_draw(&self, input_mode: InputMode) -> Option<CollectionAction> {
        if let Some(c) = self.focused_chrome {
            return Some(c);
        }
        if input_mode != InputMode::Cursor {
            return None;
        }
        let f = self.tree.focused()?;
        [
            CollectionAction::Back,
            CollectionAction::SwitchSave,
            CollectionAction::PrevPage,
            CollectionAction::NextPage,
            CollectionAction::PrevTab,
            CollectionAction::NextTab,
        ]
        .into_iter()
        .find(|&chrome| chrome.id() == f)
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
        self.archive_page = 0;
        self.cam_anim.set(None);
        self.chronicle_dashboard_scroll.set(0.0);
        self.chronicle_run_log_scroll.set(0.0);
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
        env_h: f32,
    ) -> Vec<FlatItem<CollectionAction>> {
        let env_h = collection_sanitized_room_gltf_height_scale(env_h);
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
        // Footer-centered Prev/Next pair. Archive room → page prev/next (cabinet
        // spans more catalog entries than slots). Procedural fallback → tab cycle
        // (legacy mouse affordance for keyboardless scenes).
        let center_x = w * 0.5;
        let arrow_y = h - arrow_h - h * 0.02;
        let prev_x = center_x - arrow_w * 1.5;
        let next_x = center_x + arrow_w * 0.5;
        let archive_path_for_arrows = archive_glb::archive_room_draw_ready();
        let (footer_prev_action, footer_next_action) = if archive_path_for_arrows {
            (CollectionAction::PrevPage, CollectionAction::NextPage)
        } else {
            (CollectionAction::PrevTab, CollectionAction::NextTab)
        };

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
                footer_prev_action.id(),
                [prev_x, arrow_y, arrow_w, arrow_h],
                footer_prev_action,
            ),
            FlatItem::new(
                footer_next_action.id(),
                [next_x, arrow_y, arrow_w, arrow_h],
                footer_next_action,
            ),
        ];

        // Compute the same layout draw_frame uses so hit rects line up
        // with the visuals.
        let all = tab_artifacts(self.active_tab, progress);
        let layout = compute_layout(w, h, scale, self.active_tab, all.len());

        if archive_glb::archive_room_draw_ready() {
            let cam = archive_glb::archive_camera_base(w, h, env_h);
            if let Some(tab_rects) = archive_section_tab_hit_rects(w, h, env_h, &cam) {
                for (ti, rect) in tab_rects {
                    if !flat_rect_xywh_is_finite(rect) {
                        continue;
                    }
                    items.push(FlatItem::new(
                        CollectionAction::SelectTab(ti).id(),
                        rect,
                        CollectionAction::SelectTab(ti),
                    ));
                }
            }
            if !matches!(self.active_tab, Tab::Chronicle) && !all.is_empty() {
                let anchors: Vec<Option<[f32; 3]>> = archive_glb::with_archive_glb_cpu(|opt| {
                    let mut out = vec![None; archive_glb::ARCHIVE_SLOT_COUNT];
                    let Some(cpu) = opt else {
                        return out;
                    };
                    let model = room_glb::room_env_model_matrix_from_cpu(h, env_h, cpu);
                    for (slot, anchor_slot) in out.iter_mut().enumerate() {
                        let name = archive_glb::archive_spawn_item_marker_name(slot);
                        let Some(node) = cpu.markers.get(name) else {
                            continue;
                        };
                        let p = model.transform_point3(node.transform_point3(Vec3::ZERO));
                        *anchor_slot = Some(surface_anchor_from_world_xyz(w, h, p));
                    }
                    out
                });
                let page_size = archive_page_size();
                let page_count = archive_page_count(all.len());
                let page = self.archive_page.min(page_count.saturating_sub(1));
                let cell = (w * 0.12).min(h * 0.18);
                let cell_gap = cell * 0.22;
                let cell_pitch = cell + cell_gap;
                let rect_w = cell_pitch * 0.95;
                let rect_h = cell_pitch * 0.95;
                let vp = camera_view_proj(w, h, &cam);
                for (slot, anchor) in anchors.iter().enumerate().take(page_size) {
                    let Some(anchor) = anchor else {
                        continue;
                    };
                    let global_idx = page * page_size + slot;
                    if global_idx >= all.len() {
                        continue;
                    }
                    let world = pixel_to_world_xy(w, h, anchor[0], anchor[1], anchor[2]);
                    let (sx, sy) = world_to_screen(vp, w, h, world);
                    if !screen_hit_anchor_is_finite(sx, sy, rect_w, rect_h) {
                        continue;
                    }
                    items.push(FlatItem::new(
                        CollectionAction::SelectArtifact(global_idx).id(),
                        [sx - rect_w * 0.5, sy - rect_h * 0.5, rect_w, rect_h],
                        CollectionAction::SelectArtifact(global_idx),
                    ));
                }
            }
        } else if matches!(self.active_tab, Tab::Chronicle) {
            let panel = chronicle_panel_rect(w, h, progress);
            let scroll = self.chronicle_run_log_scroll.get();
            let entry_count = archive_career::chronicle_list_entry_count(progress);
            for (i, rect) in crate::ui::chronicle_dashboard::chronicle_run_log_hit_rects(
                w,
                h,
                panel,
                scroll,
                entry_count,
            )
            .into_iter()
            .enumerate()
            {
                if !flat_rect_xywh_is_finite(rect) {
                    continue;
                }
                items.push(FlatItem::new(
                    CollectionAction::SelectArtifact(i).id(),
                    rect,
                    CollectionAction::SelectArtifact(i),
                ));
            }
        } else {
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
                let rect_w = layout.grid_cell_w * 0.95;
                let rect_h = cell_h_screen;
                let (sx, sy) = world_to_screen(view_proj, w, h, world);
                if !screen_hit_anchor_is_finite(sx, sy, rect_w, rect_h) {
                    continue;
                }
                items.push(FlatItem::new(
                    CollectionAction::SelectArtifact(idx).id(),
                    [sx - rect_w * 0.5, sy - rect_h * 0.5, rect_w, rect_h],
                    CollectionAction::SelectArtifact(idx),
                ));
            }
        }

        items
    }

    /// World-space anchor for [`ItemInspectScene`] orbit (matches HUD close-up).
    fn collection_inspect_target_world(
        &self,
        w: f32,
        h: f32,
        bosses: &[Artifact],
        layout: &crate::ui::layout::LayoutResult,
        env_h: f32,
    ) -> Option<[f32; 3]> {
        let env_h = collection_sanitized_room_gltf_height_scale(env_h);
        if bosses.is_empty() {
            return None;
        }
        if archive_glb::archive_room_draw_ready() {
            let world: Option<Vec3> = archive_glb::with_archive_glb_cpu(|opt| {
                let cpu = opt?;
                let m = archive_glb::archive_marker_world_mat4(
                    h,
                    env_h,
                    cpu,
                    archive_glb::ARCHIVE_SPAWN_FOCUSED_ITEM,
                )?;
                Some(m.transform_point3(Vec3::ZERO))
            });
            if let Some(p) = world {
                return Some(p.to_array());
            }
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
        env_h: f32,
    ) -> Option<ItemInspectOrbitState> {
        let tw = self.collection_inspect_target_world(w, h, bosses, layout, env_h)?;
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
        self.collection_inspect_orbit_for_focus(
            w,
            h,
            &bosses,
            layout,
            crate::render::room_glb::SHOP_ENV_HEIGHT_SCALE,
        )
    }

    /// Build the 3D frame for the active Archive tab: grid on a plane, camera
    /// eased to the focused cell, plus close-up and description on GLB signs (or a floating card
    /// when inspecting / procedural layout).
    fn build_archive_grid_frame(
        &self,
        mut frame: UiFrame,
        quads: Vec<GpuInstance>,
        text_labels: &mut Vec<TextLabel>,
        bosses: &[Artifact],
        ctx: DrawCtx<'_>,
        inspect: Option<&ItemInspectOrbitState>,
    ) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let env_scale = collection_sanitized_room_gltf_height_scale(ctx.room_gltf_height_scale);
        let use_archive = archive_glb::archive_room_draw_ready();
        let chronicle_dashboard = matches!(self.active_tab, Tab::Chronicle) && inspect.is_none();
        if use_archive {
            frame.archive_environment();
        }
        frame.archive_sign_description_decal_text = None;

        // Grid + backdrop plane. Object `pos` in `Object3d` is pixel-space
        // (renderer `pixel_to_world`); camera override uses world coords
        // derived from the same anchor (`cab_px_x`, `cab_px_y`).
        let cell = (w * 0.12).min(h * 0.18);
        let cell_gap = cell * 0.22;
        let cell_pitch = cell + cell_gap;
        // Pixel-space anchor for the grid center; plane at world Y = 0
        // (pixel_y = h/2 for cells on the plane).
        let cab_px_x = w * 0.5;
        let cab_px_y = h * 0.5;
        // Camera sits back along −Y, looking toward the grid plane (world Y = 0).
        let cam_world_y = -h * 1.1;

        // Grid: 6 columns. Rows span the full tab catalogue; we
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
        // already centred on an entry rather than parked on the left edge.
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
        // Focused cell world X is offset from grid center; world Z is row lift.
        // All cells share world Y = 0 on the plane.
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
        // Resting grid camera, regardless of inspect — used as one endpoint of the
        // inspect dolly lerp below, and (when the archive room is loaded and inspect is
        // off) as the anchor plane for the description / stats cards.
        let base_cam = if use_archive {
            archive_glb::archive_camera_base(w, h, env_scale)
        } else {
            CameraParams {
                eye: [cam_world_x, cam_world_y, cam_world_z],
                target: [cam_world_x, 0.0, cam_world_z],
                up: [0.0, 0.0, 1.0],
                fovy_deg: 48.0,
                clip_near: None,
                clip_far: None,
            }
        };
        let archive_feat_plane_z = if use_archive && inspect.is_none() {
            archive_glb::with_archive_glb_cpu(|opt| {
                let cpu = opt?;
                let m = archive_glb::archive_marker_world_mat4(
                    h,
                    env_scale,
                    cpu,
                    archive_glb::ARCHIVE_SPAWN_FOCUSED_ITEM,
                )?;
                Some(m.transform_point3(Vec3::ZERO).z)
            })
        } else {
            None
        };
        let coll_rig = InspectRig::collection(h);
        // Compute the inspect-orbit camera for this frame (when inspect is on) and cache
        // it so the dolly-out lerp can keep sampling that endpoint after the overlay
        // pops and `ItemInspectOrbitState` is gone.
        let inspect_cam_now: Option<CameraParams> =
            inspect.map(|ins| inspect_orbit_camera(ins, &coll_rig));
        if let Some(ic) = inspect_cam_now {
            self.last_inspect_cam.set(Some(ic));
        }
        // Procedural archive (no GLB) leans on `apply_inspect_view_to_frame` for its
        // synthetic three-point rig; the archive room handles its own lighting block
        // below, so for that path we only need to clear the description-sign flag and
        // let the dolly own `camera_override`.
        match (inspect, use_archive) {
            (Some(_), true) => {
                // Sign side selection happens unconditionally below (after `final_cam`) so the
                // chosen visible sign reflects whichever camera is in use. Don't pre-clear the
                // flag here; the decal text is set in the description-routing block.
            }
            (Some(ins), false) => {
                apply_inspect_view_to_frame(
                    &mut frame,
                    w,
                    h,
                    ins,
                    &coll_rig,
                    ins.target_world,
                    InspectFrameEnv::Neutral,
                );
                frame.archive_description_sign_use_left = None;
            }
            (None, _) => {}
        }
        // Cubic-eased dolly between grid (`base_cam`) and orbit (`inspect_cam_*`).
        // `target_phase = 1.0` while inspect is on, `0.0` once it has popped; the eased
        // blend factor drives a per-component camera lerp so eye / target / fovy all
        // glide together.
        let target_phase = if inspect.is_some() { 1.0 } else { 0.0 };
        let eased = tick_inspect_dolly(&self.inspect_dolly, target_phase);
        let inspect_cam_for_lerp = inspect_cam_now.or_else(|| self.last_inspect_cam.get());
        let final_cam = match (inspect_cam_for_lerp, eased > 1e-4) {
            (Some(ic), true) => lerp_camera(&base_cam, &ic, eased, h),
            _ => base_cam,
        };
        frame.camera_override = Some(final_cam);

        frame.fisheye_strength = 0.0;

        let focus_px_x = cab_px_x + focus_world_x;
        let focus_px_y = cab_px_y; // grid plane
        let focus_px_z = focus_world_z;

        if use_archive {
            // Archive room lighting applies whether or not the player is in orbit
            // inspect — the room stays lit by the same embedded `KHR_lights_punctual`
            // either way, so opening inspect doesn't change the exposure or wipe the
            // GLB spot/point lights.
            //
            // Sign side selection runs in both grid and inspect modes: in inspect we want exactly
            // one description board visible (decal carries the flavor copy) so the close-up never
            // sits between two competing signs. The chosen side projects against `final_cam` so
            // the side picked still tracks the camera once the dolly settles on the orbit pose.
            //
            // Reference X for "place the sign opposite this point" math. In cursor mode that is
            // the pointer; in keyboard / controller mode the cursor is intentionally ignored
            // (mouse can sit anywhere on screen without disturbing the layout) and we project the
            // focused archive slot's marker instead so the sign always sits on the opposite side
            // of whatever the player is selecting.
            if chronicle_dashboard {
                frame.archive_description_sign_use_left = None;
            } else {
                let ref_x = match ctx.input_mode {
                    crate::ui::input::InputMode::Cursor => ctx.cursor_pos.0,
                    crate::ui::input::InputMode::Keyboard
                    | crate::ui::input::InputMode::Controller => {
                        let focused_slot_in_page =
                            (focus_flat as usize) % archive_page_size().max(1);
                        archive_glb::with_archive_glb_cpu(|opt| {
                            let cpu = opt?;
                            archive_glb::archive_marker_screen_x(
                                w,
                                h,
                                env_scale,
                                &final_cam,
                                cpu,
                                archive_glb::archive_spawn_item_marker_name(focused_slot_in_page),
                            )
                        })
                        .unwrap_or(w * 0.5)
                    }
                };
                frame.archive_description_sign_use_left = Some(
                    archive_glb::with_archive_glb_cpu(|opt| {
                        let cpu = opt?;
                        archive_glb::archive_description_sign_use_left_for_ref_x(
                            w, h, env_scale, &final_cam, ref_x, cpu,
                        )
                    })
                    .unwrap_or(ref_x >= w * 0.5),
                );
            }
            let room_glb = archive_glb::archive_glb_has_embedded_lights();
            frame.scene_lighting.embedded_gltf_punctual = room_glb;
            frame.scene_lighting.room_glb_brdf = room_glb;
            frame.scene_lighting.spot_lights = if room_glb {
                archive_glb::archive_embedded_spot_lights_runtime(
                    w,
                    h,
                    env_scale,
                    &ctx.shop_env_lighting,
                )
            } else {
                Vec::new()
            };
            let inverse_punctual: Vec<ScenePunctualLight> = if room_glb {
                archive_glb::archive_embedded_point_lights_runtime(
                    w,
                    h,
                    env_scale,
                    &ctx.shop_env_lighting,
                )
                .into_iter()
                .map(ScenePunctualLight::InverseSquare)
                .collect()
            } else {
                Vec::new()
            };
            if room_glb {
                frame.scene_lighting.punctual = inverse_punctual;
            } else {
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
        } else if inspect.is_none() {
            frame.archive_description_sign_use_left = None;
            // Three warm key-lights in front of the grid. Light `pos` is
            // pixel-space (renderer converts via `pixel_to_world`), so
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

        // Grid window size — procedural infinite corridor only.
        let mut plaques: Vec<Object3d> = if chronicle_dashboard {
            Vec::new()
        } else if use_archive {
            Vec::with_capacity(archive_glb::ARCHIVE_SLOT_COUNT + 8)
        } else {
            let window_cols: i32 = 11;
            let window_rows: i32 = 9;
            let col_min = focus_col - window_cols;
            let col_max = focus_col + window_cols;
            let row_min = focus_row - window_rows;
            let row_max = focus_row + window_rows;
            Vec::with_capacity(((col_max - col_min + 1) * (row_max - row_min + 1)) as usize * 2 + 8)
        };

        if !chronicle_dashboard && !use_archive {
            let window_cols: i32 = 11;
            let window_rows: i32 = 9;
            let col_min = focus_col - window_cols;
            let col_max = focus_col + window_cols;
            let row_min = focus_row - window_rows;
            let row_max = focus_row + window_rows;

            // Back-wall slab: a single dark lacquered panel behind the
            // whole cell window. Sits behind the grid plane in world Y
            // (i.e. smaller pixel_y than cab_px_y maps to world Y > 0,
            // which is farther from camera than the plane). Gaps between
            // cells show this panel rather than the clear colour.
            let backing_w = (window_cols as f32 * 2.5) * cell_pitch;
            let backing_h = (window_rows as f32 * 2.5) * cell_pitch;
            let cabinet_anchor = crate::ui::placement::PlacementAnchor::new(
                [focus_px_x, cab_px_y - cell * 0.5, focus_world_z],
                glam::Mat4::from_rotation_x(90.0_f32.to_radians()),
                &self.positions.cabinet,
                "collection.cabinet",
                ctx.layout,
            );
            plaques.push(Object3d {
                pos: cabinet_anchor.pos,
                extents: [backing_w, backing_h, cell * 0.1],
                rotation: cabinet_anchor.object3d_rotation(),
                color: color::WALNUT_DEEP,
                kind: Object3dKind::Primitive {
                    shape: crate::render::primitive::MeshId::BeveledSlab,
                    material: crate::render::primitive::MaterialSpec::lacquered_wood_flat(),
                    pick_id: None,
                    shadow_caster: false,
                    silhouette: false,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: Some(cabinet_anchor.arrange_name),
            });

            // Cell frames + nameplates. For each (col, row) in the window we push
            // lacquered framing and — if that cell maps to a real catalog entry — a
            // tinted nameplate slightly proud of the frame. Cells beyond the catalogue
            // fill the grid with empty frames so it extends into darkness on all sides
            // rather than a ragged edge.
            for row in row_min..=row_max {
                for col in col_min..=col_max {
                    // Pixel X: grid center + per-col offset.
                    let cx = cab_px_x + (col as f32 - (cols as f32 - 1.0) * 0.5) * cell_pitch;
                    // World Z: focused row is 0, lower rows go negative.
                    let cz = -row as f32 * cell_pitch;

                    // Distance from focus, measured in cells, drives a
                    // depth-like fade so outer cells dim toward black.
                    let dc = (col - focus_col) as f32;
                    let dr = (row - focus_row) as f32;
                    let focus_d2 = dc * dc + dr * dr;
                    let fade = (1.0
                        - focus_d2 / ((window_cols.max(window_rows) as f32).powi(2) * 0.9))
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
                    // cells pop as distinct cards rather than looking
                    // like more backing. Focused cell is larger and lifted
                    // farther forward so it stands out against the wall.
                    // Nameplate pixel_y: ABOVE cab_px_y (smaller pixel_y)
                    // gives world Y > 0 (farther from camera); BELOW gives
                    // world Y < 0 (closer). We want nameplates CLOSER than
                    // the grid plane, so add to cab_px_y.
                    let is_focus = col == focus_col && row == focus_row;
                    collection_push_grid_cell_object3d(CollectionGridCellObject3d {
                        plaques: &mut plaques,
                        boss,
                        anchor: [cx, cab_px_y, cz],
                        cell,
                        fade,
                        is_focus,
                        boss_i,
                        cubby_zodiac: &self.positions.cubby_zodiac,
                        layout: ctx.layout,
                    });
                }
            }
        } else if !chronicle_dashboard {
            let anchors: Vec<Option<[f32; 3]>> = archive_glb::with_archive_glb_cpu(|opt| {
                let mut out = vec![None; archive_glb::ARCHIVE_SLOT_COUNT];
                let Some(cpu) = opt else {
                    return out;
                };
                let model = room_glb::room_env_model_matrix_from_cpu(h, env_scale, cpu);
                for (slot, anchor_slot) in out.iter_mut().enumerate() {
                    let name = archive_glb::archive_spawn_item_marker_name(slot);
                    let Some(node) = cpu.markers.get(name) else {
                        continue;
                    };
                    let p = model.transform_point3(node.transform_point3(Vec3::ZERO));
                    *anchor_slot = Some(surface_anchor_from_world_xyz(w, h, p));
                }
                out
            });
            let page_size = archive_page_size();
            let page_count = archive_page_count(bosses.len());
            let page = self.archive_page.min(page_count.saturating_sub(1));
            for (slot, anchor) in anchors.iter().enumerate().take(page_size) {
                let Some(anchor) = anchor else {
                    continue;
                };
                let global_idx = page * page_size + slot;
                if global_idx >= bosses.len() {
                    continue;
                }
                let boss = &bosses[global_idx];
                let is_focus = focus_flat as usize == global_idx;
                collection_push_grid_cell_object3d(CollectionGridCellObject3d {
                    plaques: &mut plaques,
                    boss,
                    anchor: *anchor,
                    cell,
                    fade: 1.0,
                    is_focus,
                    boss_i: global_idx as i32,
                    cubby_zodiac: &self.positions.cubby_zodiac,
                    layout: ctx.layout,
                });
            }
        }

        // ── Foreground: close-up + description plaque ───────────────
        // In front of the grid plane; camera easing keeps them stable on screen.
        let mut hud_plaques: Vec<Object3d> = Vec::new();
        let mut gradient_backers: Vec<GradientQuadInstance> = Vec::new();

        if !chronicle_dashboard && let Some(boss) = bosses.get(focus_flat as usize) {
            let with_inspect_spin = |o: Object3d| -> Object3d {
                if let Some(ins) = inspect {
                    prepend_inspect_orbit_subject_rotation(o, ins, &coll_rig)
                } else {
                    o
                }
            };
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
            let hud_world_y_offset = -h * 0.45; // in front of grid plane
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
            // Anchor the close-up to the `ARCHIVE_SPAWN_FOCUSED_ITEM` marker whenever
            // the archive room is loaded. Orbit inspect uses the same marker for
            // `target_world` (`collection_inspect_target_world`); pinning the mesh here
            // keeps the inspected item under the inspect camera instead of drifting to
            // the procedural fallback (which made it disappear off-frame).
            let (closeup_ax, closeup_ay, closeup_az) = if use_archive {
                let anchor: Option<[f32; 3]> = archive_glb::with_archive_glb_cpu(|opt| {
                    let cpu = opt?;
                    let m = archive_glb::archive_marker_world_mat4(
                        h,
                        env_scale,
                        cpu,
                        archive_glb::ARCHIVE_SPAWN_FOCUSED_ITEM,
                    )?;
                    let p = m.transform_point3(Vec3::ZERO);
                    Some(surface_anchor_from_world_xyz(w, h, p))
                });
                anchor
                    .map(|a| (a[0], a[1], a[2]))
                    .unwrap_or((closeup_px, hud_py, hud_wz))
            } else {
                (closeup_px, hud_py, hud_wz)
            };
            let closeup_anchor = crate::ui::placement::PlacementAnchor::new(
                [closeup_ax, closeup_ay, closeup_az],
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
                    let thick = face * 0.06 * visual.thickness_scale;
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
                    hud_plaques.push(with_inspect_spin(Object3d {
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
                    }));
                }
                ArtifactKind::Talisman(tk) => {
                    // Pendant orientation matches the grid cells; see the
                    // grid-side Rx(-90°) comment. 14° Ry tilt keeps the
                    // material sheen readable on the featured tablet too.
                    hud_plaques.push(with_inspect_spin(Object3d {
                        pos: closeup_anchor.pos,
                        extents: [closeup_size * 1.40, closeup_size * 2.0, closeup_size * 0.36],
                        rotation: euler_xyz_rad_from_deg(-90.0, 14.0, 0.0),
                        color: closeup_bright,
                        kind: Object3dKind::Talisman { kind: *tk },
                        hover_target: 1.0,
                        anim_id: 0xC105E0,
                        arrange_name: Some(closeup_anchor.arrange_name),
                    }));
                }
                ArtifactKind::Zodiac(zk) => {
                    hud_plaques.push(with_inspect_spin(zodiac_ribbon_object3d(
                        ZodiacRibbonSpec {
                            pos: closeup_anchor.pos,
                            length: closeup_size,
                            rotation: closeup_anchor.object3d_rotation(),
                            color: [1.0, 1.0, 1.0, 1.0],
                            kind: Some(*zk),
                            hover_target: 1.0,
                            anim_id: 0xC105E0,
                            arrange_name: Some(closeup_anchor.arrange_name),
                        },
                    )));
                }
                ArtifactKind::PlaqueOnly => {
                    let label = if boss.unlocked {
                        boss.name.clone()
                    } else {
                        String::from("???")
                    };
                    let color = if boss.unlocked {
                        // Match gameplay hanging plaque: neutral tint so lacquer +
                        // gilded decal read correctly (tier halo was washing the wood).
                        [1.0, 1.0, 1.0, 1.0]
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
                    hud_plaques.push(with_inspect_spin(Object3d {
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
                    }));
                }
                ArtifactKind::ChronicleSummary => {}
                ArtifactKind::ChronicleRun(_) => {
                    let label = boss.name.clone();
                    use crate::render::primitive::{DecalLayout, DecalSpec, MaterialSpec, MeshId};
                    let closeup_material =
                        MaterialSpec::lacquered_wood_flat().with_decal(DecalSpec {
                            text: label,
                            layout: DecalLayout::Fit {
                                target_short_edge: crate::render::decal::PLAQUE_DECAL_HEIGHT,
                            },
                        });
                    hud_plaques.push(with_inspect_spin(Object3d {
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
                    }));
                }
            }

            // ── Description plaque ───────────────────────────────────
            let card_w = h * 0.22;
            let card_h = h * 0.16;
            let card_px_proc = cab_px_x + card_wx;
            const ARCHIVE_DESC_SX_FRAC: f32 = 0.705_f32;
            const ARCHIVE_DESC_SY_FRAC: f32 = 0.50_f32;
            // Relic: grid focus plaque shows mechanical rules; orbit inspect shows flavor only.
            let body = if boss.unlocked
                && inspect.is_some()
                && let ArtifactKind::Relic(rid) = &boss.kind
            {
                all_relic_defs()
                    .iter()
                    .find(|d| d.id == *rid)
                    .map(|d| d.flavor.iter().fold(String::new(), |acc, s| acc + s.text))
                    .unwrap_or_default()
            } else {
                description_for(boss, ctx.run, ctx.progress)
            };
            let card_text = if body.is_empty() {
                boss.name.clone()
            } else {
                format!("{}\n\n{}", boss.name, body)
            };
            // Description routing matrix:
            // - Archive room (grid or inspect): rasterize copy onto the visible GLB sign decal.
            //   For unlocked relics in inspect we use the styled `flavor_spans` flattened to
            //   plain text (the decal pipeline takes a single string; bold / italic styling is
            //   surrendered, but the player gets the flavor on the sign as expected). All other
            //   states (grid mode, locked relics, non-relic artifacts in inspect) use
            //   `card_text` (name + body) as in the prior grid-only path. The sign carries the
            //   single readable surface in both modes — no bottom-centred floating flavor band
            //   or top tooltip panel runs alongside it, which would double up the copy.
            // - Procedural archive (no GLB) or non-archive: floating 3D description card
            //   (existing behaviour, sized for that camera).
            if use_archive {
                let sign_text = if inspect.is_some()
                    && boss.unlocked
                    && let ArtifactKind::Relic(rid) = &boss.kind
                    && let Some(def) = all_relic_defs().iter().find(|d| d.id == *rid)
                    && !def.flavor.is_empty()
                {
                    def.flavor.iter().fold(String::new(), |acc, s| acc + s.text)
                } else {
                    card_text.clone()
                };
                frame.archive_sign_description_decal_text = Some(sign_text);
            } else {
                // Procedural archive layout / non-GLB room: floating 3D description card.
                let card_base: [f32; 3] = if let Some(pz) = archive_feat_plane_z {
                    collection_hud_anchor_on_cam_plane(
                        w,
                        h,
                        &base_cam,
                        w * ARCHIVE_DESC_SX_FRAC,
                        h * ARCHIVE_DESC_SY_FRAC,
                        pz,
                    )
                } else {
                    [card_px_proc, hud_py, hud_wz]
                };
                let anchor = crate::ui::placement::PlacementAnchor::new(
                    card_base,
                    rot_fixed_axes_deg(90.0, 0.0, 0.0),
                    &self.positions.focus_card,
                    "collection.focus_card",
                    ctx.layout,
                );
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
                let stats_base: [f32; 3] = if let Some(pz) = archive_feat_plane_z {
                    let dz = card_h * 0.65 + stats_h * 0.55;
                    collection_hud_anchor_on_cam_plane(
                        w,
                        h,
                        &base_cam,
                        w * ARCHIVE_DESC_SX_FRAC,
                        h * ARCHIVE_DESC_SY_FRAC,
                        pz - dz,
                    )
                } else {
                    let stats_wz = hud_wz - card_h * 0.65 - stats_h * 0.55;
                    [card_px_proc, hud_py, stats_wz]
                };
                use crate::render::primitive::{DecalLayout, DecalSpec, MaterialSpec, MeshId};
                let stats_anchor = crate::ui::placement::PlacementAnchor::new(
                    stats_base,
                    glam::Mat4::from_rotation_x(90.0_f32.to_radians()),
                    &self.positions.stats_plaque,
                    "collection.stats_plaque",
                    ctx.layout,
                );
                hud_plaques.push(Object3d {
                    pos: stats_anchor.pos,
                    extents: [card_w, stats_h, stats_h * 0.06],
                    rotation: stats_anchor.object3d_rotation(),
                    color: color::WALNUT_BRIGHT,
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
                    arrange_name: Some(stats_anchor.arrange_name),
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
            if let Some(cam) = frame.camera_override
                && archive_feat_plane_z.is_none()
            {
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

        // Assemble the frame. 2D chrome from the caller (`quads` / `text_labels`)
        // is merged here with the grid and focus plaques.
        frame.quads(quads);
        frame.object3d_batch(plaques);
        if chronicle_dashboard {
            let ch = archive_chrome_layout(w, h);
            let layout = compute_layout(w, h, ch.scale, self.active_tab, bosses.len().max(1));
            let panel = [
                ch.margin_x,
                layout.band_top_y,
                w - ch.margin_x * 2.0,
                layout.band_bottom_y - layout.band_top_y,
            ];
            let dim = crate::ui::chronicle_dashboard::chronicle_dim_gradient(panel);
            let mut chart_quads: Vec<GpuInstance> = Vec::new();
            crate::ui::chronicle_dashboard::push_chronicle_dashboard(
                w,
                h,
                panel,
                crate::ui::chronicle_dashboard::ChronicleView {
                    focused_run: self.focused_row,
                    run_log_scroll: self.chronicle_run_log_scroll.get(),
                    career_scroll: self.chronicle_dashboard_scroll.get(),
                },
                ctx.progress,
                &mut chart_quads,
                text_labels,
            );
            frame.gradient_quads(std::iter::once(dim));
            frame.quads(chart_quads);
        } else if !gradient_backers.is_empty() {
            frame.gradient_quads(gradient_backers);
        }
        if !hud_plaques.is_empty() {
            frame.object3d_batch(hud_plaques);
        }
        // Title-bar wood tablets last so they depth-test in front of the room + HUD props.
        {
            let ch = archive_chrome_layout(w, h);
            let margin_x = ch.margin_x;
            let title_y = ch.title_y;
            let back_rect = [margin_x, title_y, ch.back_w, ch.chrome_btn_h];
            let switch_rect = [
                w - margin_x - ch.switch_w,
                title_y,
                ch.switch_w,
                ch.chrome_btn_h,
            ];
            let ring_focus = self.chrome_focus_for_draw(ctx.input_mode);
            let chrome_plane_z = collection_chrome_tablet_plane_z(&final_cam);
            frame.object3d_batch(vec![
                collection_chrome_wood_tablet(
                    w,
                    h,
                    back_rect,
                    &final_cam,
                    chrome_plane_z,
                    ring_focus == Some(CollectionAction::Back),
                    "collection.chrome.back",
                ),
                collection_chrome_wood_tablet(
                    w,
                    h,
                    switch_rect,
                    &final_cam,
                    chrome_plane_z,
                    ring_focus == Some(CollectionAction::SwitchSave),
                    "collection.chrome.switch_save",
                ),
            ]);
        }
        frame.texts(std::mem::take(text_labels));

        // Hit rects for 2D chrome — skipped while [`ItemInspectScene`] owns input.
        if inspect.is_none() {
            let items = self.flat_items(w, h, ctx.progress, env_scale);
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
        self.drawn_room_gltf_height_scale
            .set(collection_sanitized_room_gltf_height_scale(
                ctx.room_gltf_height_scale,
            ));
        let ch = archive_chrome_layout(w, h);
        let scale = ch.scale;
        let margin_x = ch.margin_x;
        let title_y = ch.title_y;

        let frame = UiFrame::new();

        let mut quads: Vec<GpuInstance> = Vec::new();
        let mut text_labels: Vec<TextLabel> = Vec::new();

        // Title — pinned font so long couch viewing doesn't auto-shrink glyphs.
        let title_font_px = typography::size(typography::H20, h);
        let title_h = (title_font_px / 0.55).ceil() + 8.0;
        text_labels.push(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: format!("Archive — {}", self.active_tab.label()),
            color: color::CHAMPAGNE,
            font_px: Some(title_font_px),
            ..Default::default()
        });

        // Back / Switch save — flat fills + labels always read on every display
        // (3D lacquered wood tablets in `build_archive_grid_frame` add depth when they
        // depth-test over the archive room; the room shell can still hide them edge-on).
        let back_w = ch.back_w;
        let back_h = ch.chrome_btn_h;
        let ring_focus = self.chrome_focus_for_draw(ctx.input_mode);
        let back_rect = [margin_x, title_y, back_w, back_h];
        let back_focused = ring_focus == Some(CollectionAction::Back);
        quads.push(GpuInstance {
            rect: back_rect,
            color: chrome_btn_color(back_focused),
            user: 0,
        });

        let switch_w = ch.switch_w;
        let switch_x = w - margin_x - switch_w;
        let switch_rect = [switch_x, title_y, switch_w, back_h];
        let switch_focused = ring_focus == Some(CollectionAction::SwitchSave);
        quads.push(GpuInstance {
            rect: switch_rect,
            color: chrome_btn_color(switch_focused),
            user: 0,
        });

        let chrome_label_px = typography::size(typography::H24, h);
        text_labels.push(TextLabel {
            rect: back_rect,
            text: "Back".into(),
            color: color::PARCHMENT,
            font_px: Some(chrome_label_px),
            align: TextAlign::Center,
            ..Default::default()
        });
        text_labels.push(TextLabel {
            rect: switch_rect,
            text: "Switch save".into(),
            color: color::PARCHMENT,
            font_px: Some(chrome_label_px),
            align: TextAlign::Center,
            ..Default::default()
        });

        if back_focused {
            push_focus_ring(back_rect, scale, w, h, &mut quads);
        }

        if switch_focused {
            push_focus_ring(switch_rect, scale, w, h, &mut quads);
        }

        // Footer Prev/Next tab arrows. Match the rects from `flat_items`
        // so the click hit-testing and the visual button line up. The
        // active footer action depends on whether the archive room is loaded
        // (page) vs. the procedural grid (tab cycle), so the focus ring
        // checks both variants.
        let arrow_w = ch.arrow_w;
        let arrow_h = back_h;
        let arrow_y = h - arrow_h - h * 0.02;
        let prev_x = w * 0.5 - arrow_w * 1.5;
        let next_x = w * 0.5 + arrow_w * 0.5;
        for (x, sym, focused) in [
            (
                prev_x,
                "<",
                matches!(
                    ring_focus,
                    Some(CollectionAction::PrevPage) | Some(CollectionAction::PrevTab)
                ),
            ),
            (
                next_x,
                ">",
                matches!(
                    ring_focus,
                    Some(CollectionAction::NextPage) | Some(CollectionAction::NextTab)
                ),
            ),
        ] {
            let rect = [x, arrow_y, arrow_w, arrow_h];
            quads.push(GpuInstance {
                rect,
                color: chrome_btn_color(focused),
                user: 0,
            });
            text_labels.push(TextLabel {
                rect,
                text: sym.into(),
                color: color::PARCHMENT,
                font_px: Some(typography::size(typography::H28, h)),
                ..Default::default()
            });
            if focused {
                push_focus_ring(rect, scale, w, h, &mut quads);
            }
        }

        // Control hints — sits just above the tab arrows so keyboard /
        // controller bindings are discoverable without a separate help
        // overlay. The page / scroll affordance line is omitted when the
        // tab fits in one page so the hint stays compact on small catalogs.
        let all_count_hint = tab_artifacts(self.active_tab, ctx.progress).len();
        let tab_scrollable = (total_rows_for(all_count_hint) as usize) > 0 && {
            let probe = compute_layout(w, h, scale, self.active_tab, all_count_hint);
            (probe.grid_rows as usize) > probe.visible_rows as usize
        };
        let archive_path = archive_glb::archive_room_draw_ready();
        let archive_page_count_now = archive_page_count(all_count_hint);
        let archive_multi_page = archive_path && archive_page_count_now > 1;
        // TV: pinned body size + multi-line copy so width-based auto-shrink
        // never drives hints to microtext.
        let hint_font_px = typography::size(typography::H36, h);
        let hint_line_h = (hint_font_px / 0.55).ceil() + 4.0;
        let hint_text: String = if inspect.is_some() {
            "Right stick or WASD / arrows: orbit item\nTriggers / scroll or Shift+W/↑ / Shift+S/↓: zoom   ·   E / North: close   ·   Esc: menu"
                .to_string()
        } else if matches!(self.active_tab, Tab::Chronicle) && all_count_hint == 0 {
            "Finish a non-tutorial run to add folios here.\nTab / Shift+Tab: tabs   ·   Esc: back"
                .to_string()
        } else if matches!(self.active_tab, Tab::Chronicle) && all_count_hint > 0 {
            "Tab / Shift+Tab: section   ·   ↑↓: run log   ·   PgUp / PgDn: detail pane   ·   E / North: inspect run   ·   Esc: back"
                .to_string()
        } else if archive_path {
            if archive_multi_page {
                "Tab / Shift+Tab: section   ·   \u{2190}\u{2192}\u{2191}\u{2193}: focus   ·   Enter / E / North: inspect   ·   Esc: back\nPgUp / PgDn or mouse wheel: page   ·   Footer arrows or page-edge \u{2190} / \u{2192}: page"
                    .to_string()
            } else {
                "Tab / Shift+Tab: section   ·   \u{2190}\u{2192}\u{2191}\u{2193}: focus   ·   Enter / E / North: inspect   ·   Esc: back"
                    .to_string()
            }
        } else if tab_scrollable {
            "Tab / Shift+Tab: cycle tab   ·   \u{2190}\u{2192}\u{2191}\u{2193}: focus   ·   Enter: select   ·   E/North: inspect   ·   Esc: back\nScroll: mouse wheel or PgUp / PgDn"
                .to_string()
        } else {
            "Tab / Shift+Tab: cycle tab   ·   \u{2190}\u{2192}\u{2191}\u{2193}: focus\nEnter: select   ·   E/North: inspect   ·   Esc: back"
                .to_string()
        };
        let hint_lines = hint_text.lines().count().max(1) as f32;
        let hint_h = hint_line_h * hint_lines + 10.0;
        // Layout (bottom → top): footer arrows → page indicator (multi-page only)
        // → hint copy → ladder line. Compute the indicator band first so the hint
        // can stack above it without overlap.
        let page_band_h = if archive_multi_page && inspect.is_none() {
            hint_line_h + 8.0
        } else {
            0.0
        };
        let page_band_y = arrow_y
            - page_band_h
            - if archive_multi_page && inspect.is_none() {
                (h * 0.006).max(4.0)
            } else {
                0.0
            };
        let hint_y = page_band_y - hint_h - (h * 0.014).max(10.0);
        text_labels.push(TextLabel {
            rect: [margin_x * 0.5, hint_y, w - margin_x, hint_h],
            text: hint_text,
            color: [0.78, 0.80, 0.88, 0.92],
            font_px: Some(hint_font_px),
            align: TextAlign::Center,
            ..Default::default()
        });

        // Page indicator — `Page X / Y · ● ○ ○ …` centred between the hint and
        // the footer arrows, only when archive multi-page. Gives the player a
        // glanceable cue that the catalog continues past the visible cabinet.
        if archive_multi_page && inspect.is_none() {
            let cur_page = self.archive_page.min(archive_page_count_now - 1);
            // Cap the dot row to keep the indicator readable on very large
            // catalogs; show numeric only when dots would crowd the line.
            const MAX_DOTS: usize = 12;
            let dots_text = if archive_page_count_now <= MAX_DOTS {
                let mut s = String::new();
                for i in 0..archive_page_count_now {
                    if i > 0 {
                        s.push(' ');
                    }
                    s.push(if i == cur_page {
                        '\u{25CF}'
                    } else {
                        '\u{25CB}'
                    });
                }
                s
            } else {
                String::new()
            };
            let label = if dots_text.is_empty() {
                format!("Page {} / {}", cur_page + 1, archive_page_count_now)
            } else {
                format!(
                    "Page {} / {}   {}",
                    cur_page + 1,
                    archive_page_count_now,
                    dots_text
                )
            };
            text_labels.push(TextLabel {
                rect: [margin_x * 0.5, page_band_y, w - margin_x, page_band_h],
                text: label,
                color: [0.95, 0.86, 0.56, 0.95],
                font_px: Some(hint_font_px),
                align: TextAlign::Center,
                ..Default::default()
            });
        }

        let all_artifacts = tab_artifacts(self.active_tab, ctx.progress);

        // Grid layout, focus close-up, and description plaque.
        self.build_archive_grid_frame(frame, quads, &mut text_labels, &all_artifacts, ctx, inspect)
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
            self.drawn_room_gltf_height_scale.get(),
        );
        // Keyboard / directional / Confirm: grid moves use each artifact's
        // screen AABB (`flat_items`) + spatial neighbors first, then fall
        // back to column/row index rules (horizontal archive window scroll,
        // etc.). The tree handles hover + clicks and stays in sync via
        // `apply_artifact_focus` (`set_focus` on `SelectArtifact`).
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
        //   - Archive room: ←/↓ advance one slot forward along the full
        //     catalog (row-major); →/↑ go backward (wrap). Viewport scroll
        //     and column window follow focus.
        //   - Procedural grid: spatial neighbor moves from cell rects, with
        //     L/R and U/D as fallback axes; wheel scrolls one row per tick.
        //   - Confirm (A / Space / Enter) → open inspect orbit on the archive path;
        //     procedural fallback uses Confirm to set the focused item as the plinth target.
        let all_count = tab_artifacts(self.active_tab, ctx.progress).len();
        let archive_path = archive_glb::archive_room_draw_ready();
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

        // Mouse wheel:
        //   - archive room: 1 page per tick (matches the cabinet-as-page model)
        //   - procedural grid: 1 row per tick (legacy scroll behaviour)
        if ctx.scroll_lines.abs() > 0.001 {
            if matches!(self.active_tab, Tab::Chronicle) {
                let w = ctx.layout.window_w;
                let h = ctx.layout.window_h;
                let panel = chronicle_panel_rect(w, h, ctx.progress);
                let max_s = crate::ui::chronicle_dashboard::chronicle_right_pane_scroll_max(
                    w,
                    h,
                    panel,
                    ctx.progress,
                    self.focused_row,
                );
                let next = (self.chronicle_dashboard_scroll.get() - ctx.scroll_lines * 42.0)
                    .clamp(0.0, max_s);
                self.chronicle_dashboard_scroll.set(next);
            } else if archive_path {
                let dir: i32 = if ctx.scroll_lines > 0.0 { 1 } else { -1 };
                let from = archive_focus_row_col_in_page(self.focused_row);
                archive_page_step(self, ctx.bus, dir, from, all_count);
            } else if max_scroll > 0.0 {
                let next =
                    (self.target_scroll_rows.get() + ctx.scroll_lines).clamp(0.0, max_scroll);
                self.target_scroll_rows.set(next);
            }
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
        // Clamp a (row, col) candidate to the last actually-present
        // artifact (the final row may be partially filled).
        let global_idx = |row: usize, col: usize| -> Option<usize> {
            if all_count == 0 {
                return None;
            }
            let cand = row * cols + col;
            Some(cand.min(all_count - 1))
        };
        let apply_artifact_focus = |scene: &mut CollectionScene,
                                    bus: &mut crate::game::event_bus::EventBus,
                                    idx: usize| {
            bus.push(GameEvent::UiSound(SfxId::TilePlace));
            collection_sync_artifact_focus_to_idx(scene, idx, cols, max_scroll, visible_rows);
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
                // PageUp/PageDown:
                //   - archive room: flip cabinet page (matches the wheel + footer arrows)
                //   - procedural grid: scroll viewport by one visible-page worth
                UiAction::PageNext => {
                    if matches!(self.active_tab, Tab::Chronicle) {
                        let w = ctx.layout.window_w;
                        let h = ctx.layout.window_h;
                        let panel = chronicle_panel_rect(w, h, ctx.progress);
                        let max_s = crate::ui::chronicle_dashboard::chronicle_right_pane_scroll_max(
                            w,
                            h,
                            panel,
                            ctx.progress,
                            self.focused_row,
                        );
                        let step = h * 0.22;
                        let next = (self.chronicle_dashboard_scroll.get() + step).min(max_s);
                        self.chronicle_dashboard_scroll.set(next);
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    } else if archive_path {
                        let from = archive_focus_row_col_in_page(self.focused_row);
                        archive_page_step(self, ctx.bus, 1, from, all_count);
                    } else if max_scroll > 0.0 {
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                        let next = (self.target_scroll_rows.get() + visible_rows as f32)
                            .clamp(0.0, max_scroll);
                        self.target_scroll_rows.set(next);
                    }
                }
                UiAction::PagePrev => {
                    if matches!(self.active_tab, Tab::Chronicle) {
                        let w = ctx.layout.window_w;
                        let h = ctx.layout.window_h;
                        let panel = chronicle_panel_rect(w, h, ctx.progress);
                        let max_s = crate::ui::chronicle_dashboard::chronicle_right_pane_scroll_max(
                            w,
                            h,
                            panel,
                            ctx.progress,
                            self.focused_row,
                        );
                        let step = h * 0.22;
                        let next = (self.chronicle_dashboard_scroll.get() - step)
                            .max(0.0)
                            .min(max_s);
                        self.chronicle_dashboard_scroll.set(next);
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    } else if archive_path {
                        let from = archive_focus_row_col_in_page(self.focused_row);
                        archive_page_step(self, ctx.bus, -1, from, all_count);
                    } else if max_scroll > 0.0 {
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                        let next = (self.target_scroll_rows.get() - visible_rows as f32)
                            .clamp(0.0, max_scroll);
                        self.target_scroll_rows.set(next);
                    }
                }
                UiAction::FocusNext => {
                    if self.focused_chrome.is_some() {
                        collection_chrome_directional(self, ctx.bus, &items, FocusDir::Right);
                        continue;
                    }
                    if all_count == 0 {
                        continue;
                    }
                    if archive_path {
                        archive_directional_step(self, ctx.bus, &items, FocusDir::Right, all_count);
                        continue;
                    }
                    if let Some(ni) =
                        collection_spatial_artifact_step(&items, self.focused_row, FocusDir::Right)
                        && Some(ni) != self.focused_row
                    {
                        apply_artifact_focus(self, ctx.bus, ni);
                        continue;
                    }
                    let (row, col) = cur_row_col.unwrap_or((0, 0));
                    let next_col = (col + 1).min(cols - 1);
                    if next_col != col
                        && let Some(i) = global_idx(row, next_col)
                        && Some(i) != self.focused_row
                    {
                        apply_artifact_focus(self, ctx.bus, i);
                    }
                }
                UiAction::FocusPrev => {
                    if self.focused_chrome.is_some() {
                        collection_chrome_directional(self, ctx.bus, &items, FocusDir::Left);
                        continue;
                    }
                    if all_count == 0 {
                        continue;
                    }
                    if archive_path {
                        archive_directional_step(self, ctx.bus, &items, FocusDir::Left, all_count);
                        continue;
                    }
                    if let Some(ni) =
                        collection_spatial_artifact_step(&items, self.focused_row, FocusDir::Left)
                        && Some(ni) != self.focused_row
                    {
                        apply_artifact_focus(self, ctx.bus, ni);
                        continue;
                    }
                    let (row, col) = cur_row_col.unwrap_or((0, 0));
                    if col > 0
                        && let Some(i) = global_idx(row, col - 1)
                    {
                        apply_artifact_focus(self, ctx.bus, i);
                    }
                }
                UiAction::FocusUp => {
                    if self.focused_chrome.is_some() {
                        collection_chrome_directional(self, ctx.bus, &items, FocusDir::Up);
                        continue;
                    }
                    if matches!(self.active_tab, Tab::Chronicle) && all_count > 0 {
                        let idx = self.focused_row.unwrap_or(0);
                        if idx > 0 {
                            apply_artifact_focus(self, ctx.bus, idx - 1);
                            chronicle_sync_run_log_scroll(
                                self,
                                ctx.layout.window_w,
                                ctx.layout.window_h,
                                ctx.progress,
                            );
                        } else {
                            collection_enter_chrome(self, ctx.bus, &items, FocusDir::Up);
                        }
                        continue;
                    }
                    if archive_path {
                        if all_count == 0
                            || !archive_directional_step(
                                self,
                                ctx.bus,
                                &items,
                                FocusDir::Up,
                                all_count,
                            )
                        {
                            collection_enter_chrome(self, ctx.bus, &items, FocusDir::Up);
                        }
                        continue;
                    }
                    if all_count == 0 || total_rows <= 1 {
                        collection_enter_chrome(self, ctx.bus, &items, FocusDir::Up);
                        continue;
                    }
                    if let Some(ni) =
                        collection_spatial_artifact_step(&items, self.focused_row, FocusDir::Up)
                        && Some(ni) != self.focused_row
                    {
                        apply_artifact_focus(self, ctx.bus, ni);
                        continue;
                    }
                    let (row, col) = cur_row_col.unwrap_or((0, 0));
                    if row > 0
                        && let Some(i) = global_idx(row - 1, col)
                    {
                        apply_artifact_focus(self, ctx.bus, i);
                    } else {
                        collection_enter_chrome(self, ctx.bus, &items, FocusDir::Up);
                    }
                }
                UiAction::FocusDown => {
                    if self.focused_chrome.is_some() {
                        collection_chrome_directional(self, ctx.bus, &items, FocusDir::Down);
                        continue;
                    }
                    if matches!(self.active_tab, Tab::Chronicle) && all_count > 0 {
                        let idx = self.focused_row.unwrap_or(0);
                        if idx + 1 < all_count {
                            apply_artifact_focus(self, ctx.bus, idx + 1);
                            chronicle_sync_run_log_scroll(
                                self,
                                ctx.layout.window_w,
                                ctx.layout.window_h,
                                ctx.progress,
                            );
                        } else {
                            collection_enter_chrome(self, ctx.bus, &items, FocusDir::Down);
                        }
                        continue;
                    }
                    if archive_path {
                        if all_count == 0
                            || !archive_directional_step(
                                self,
                                ctx.bus,
                                &items,
                                FocusDir::Down,
                                all_count,
                            )
                        {
                            collection_enter_chrome(self, ctx.bus, &items, FocusDir::Down);
                        }
                        continue;
                    }
                    if all_count == 0 || total_rows <= 1 {
                        collection_enter_chrome(self, ctx.bus, &items, FocusDir::Down);
                        continue;
                    }
                    if let Some(ni) =
                        collection_spatial_artifact_step(&items, self.focused_row, FocusDir::Down)
                        && Some(ni) != self.focused_row
                    {
                        apply_artifact_focus(self, ctx.bus, ni);
                        continue;
                    }
                    let (row, col) = cur_row_col.unwrap_or((0, 0));
                    let next_row = (row + 1).min(total_rows - 1);
                    if next_row != row
                        && let Some(i) = global_idx(next_row, col)
                        && Some(i) != self.focused_row
                    {
                        apply_artifact_focus(self, ctx.bus, i);
                    } else {
                        collection_enter_chrome(self, ctx.bus, &items, FocusDir::Down);
                    }
                }
                UiAction::Confirm => {
                    if let Some(chrome) = self.focused_chrome {
                        match chrome {
                            CollectionAction::Back => {
                                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                                return Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()));
                            }
                            CollectionAction::SwitchSave => {
                                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                                return Some(Scene::ProfileSelect(
                                    ProfileSelectScene::from_archive_switch_save(),
                                ));
                            }
                            CollectionAction::PrevPage => {
                                if matches!(self.active_tab, Tab::Chronicle) {
                                    chronicle_apply_footer_page_scroll(
                                        self,
                                        ctx.layout.window_w,
                                        ctx.layout.window_h,
                                        ctx.progress,
                                        -1.0,
                                    );
                                } else {
                                    let from = archive_focus_row_col_in_page(self.focused_row);
                                    archive_page_step(self, ctx.bus, -1, from, all_count);
                                }
                            }
                            CollectionAction::NextPage => {
                                if matches!(self.active_tab, Tab::Chronicle) {
                                    chronicle_apply_footer_page_scroll(
                                        self,
                                        ctx.layout.window_w,
                                        ctx.layout.window_h,
                                        ctx.progress,
                                        1.0,
                                    );
                                } else {
                                    let from = archive_focus_row_col_in_page(self.focused_row);
                                    archive_page_step(self, ctx.bus, 1, from, all_count);
                                }
                            }
                            CollectionAction::PrevTab => {
                                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                                self.cycle_tab(false);
                            }
                            CollectionAction::NextTab => {
                                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                                self.cycle_tab(true);
                            }
                            _ => {}
                        }
                        continue;
                    }
                    if (archive_path && !matches!(self.active_tab, Tab::Chronicle))
                        || (matches!(self.active_tab, Tab::Chronicle)
                            && self.focused_row.is_some_and(|i| i > 0))
                    {
                        let w = ctx.layout.window_w;
                        let h = ctx.layout.window_h;
                        let bosses = tab_artifacts(self.active_tab, ctx.progress);
                        if !bosses.is_empty()
                            && let Some(orbit) = self.collection_inspect_orbit_for_focus(
                                w,
                                h,
                                &bosses,
                                ctx.layout,
                                ctx.room_gltf_height_scale,
                            )
                        {
                            ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                            *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(
                                Scene::Showcase(crate::scenes::ShowcaseScene::new(
                                    crate::scenes::ShowcasePresenter::CollectionInspect(
                                        crate::scenes::CollectionInspectPresenter::new(orbit),
                                    ),
                                )),
                            )));
                        }
                    } else if let Some(i) = self.focused_row {
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
                    if matches!(self.active_tab, Tab::Chronicle)
                        && self.focused_row.is_some_and(|i| i == 0)
                    {
                        continue;
                    }
                    if let Some(orbit) = self.collection_inspect_orbit_for_focus(
                        w,
                        h,
                        &bosses,
                        ctx.layout,
                        ctx.room_gltf_height_scale,
                    ) {
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
            Some(CollectionAction::PrevPage) => {
                if matches!(self.active_tab, Tab::Chronicle) {
                    chronicle_apply_footer_page_scroll(
                        self,
                        ctx.layout.window_w,
                        ctx.layout.window_h,
                        ctx.progress,
                        -1.0,
                    );
                } else {
                    let from = archive_focus_row_col_in_page(self.focused_row);
                    archive_page_step(self, ctx.bus, -1, from, all_count);
                }
            }
            Some(CollectionAction::NextPage) => {
                if matches!(self.active_tab, Tab::Chronicle) {
                    chronicle_apply_footer_page_scroll(
                        self,
                        ctx.layout.window_w,
                        ctx.layout.window_h,
                        ctx.progress,
                        1.0,
                    );
                } else {
                    let from = archive_focus_row_col_in_page(self.focused_row);
                    archive_page_step(self, ctx.bus, 1, from, all_count);
                }
            }
            Some(CollectionAction::SelectTab(i)) => {
                if i < TABS.len() {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    self.active_tab = TABS[i];
                    self.selected_artifact = None;
                    self.focused_row = Some(0);
                    self.focused_chrome = None;
                    self.scroll_rows.set(0.0);
                    self.target_scroll_rows.set(0.0);
                    self.archive_page = 0;
                    self.cam_anim.set(None);
                    self.chronicle_dashboard_scroll.set(0.0);
                    self.chronicle_run_log_scroll.set(0.0);
                }
            }
            Some(CollectionAction::SelectArtifact(idx)) => {
                if all_count > 0 {
                    let idx = idx.min(all_count.saturating_sub(1));
                    // Relic cells have a per-triangle trimesh picker running
                    // each frame; when it reports a hit, prefer that index
                    // over the loose cell rect so clicks that land in the
                    // empty space around a relic's silhouette don't select
                    // the wrong artifact. For non-relic cells (talismans /
                    // zodiacs / plaques) the trimesh picker stays silent
                    // and the flat cell rect remains the source of truth.
                    let resolved_raw = ctx
                        .picked_collection_object
                        .map(|pid| pid as usize)
                        .unwrap_or(idx);
                    let resolved = resolved_raw.min(all_count - 1);
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    self.selected_artifact = Some(resolved);
                    self.focused_chrome = None;
                    push_relic_stinger_for(ctx.bus, self.active_tab, ctx.progress, resolved);
                    // Mouse click also moves keyboard focus so subsequent
                    // arrow-key navigation continues from the clicked item
                    // instead of teleporting back to position 0.
                    // Keep the viewport synced to the clicked row so the
                    // focus halo and the visible slot agree.
                    collection_sync_artifact_focus_to_idx(
                        self,
                        resolved,
                        cols,
                        max_scroll,
                        visible_rows,
                    );
                    if matches!(self.active_tab, Tab::Chronicle) {
                        chronicle_sync_run_log_scroll(
                            self,
                            ctx.layout.window_w,
                            ctx.layout.window_h,
                            ctx.progress,
                        );
                    }
                }
            }
            None => {}
        }

        if matches!(self.active_tab, Tab::Chronicle) {
            let w = ctx.layout.window_w;
            let h = ctx.layout.window_h;
            let panel = chronicle_panel_rect(w, h, ctx.progress);
            let entry_count = archive_career::chronicle_list_entry_count(ctx.progress);
            let right_max = crate::ui::chronicle_dashboard::chronicle_right_pane_scroll_max(
                w,
                h,
                panel,
                ctx.progress,
                self.focused_row,
            );
            self.chronicle_dashboard_scroll
                .set(self.chronicle_dashboard_scroll.get().clamp(0.0, right_max));
            let run_max = crate::ui::chronicle_dashboard::chronicle_run_log_scroll_max(
                w,
                h,
                panel,
                entry_count,
            );
            let panes = crate::ui::chronicle_dashboard::chronicle_pane_layout(w, h, panel);
            let scroll = crate::ui::chronicle_dashboard::chronicle_clamp_run_log_scroll(
                self.chronicle_run_log_scroll.get(),
                self.focused_row,
                entry_count,
                panes,
            );
            self.chronicle_run_log_scroll
                .set(scroll.clamp(0.0, run_max));
        }

        if archive_glb::archive_room_draw_ready() {
            let n = tab_artifacts(self.active_tab, ctx.progress).len();
            let pc = archive_page_count(n);
            self.archive_page = self.archive_page.min(pc.saturating_sub(1));
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

// ── Tab artifact enumeration ────────────────────────────────────────

/// Build the list of artifacts for one tab. Reads [`PlayerProgress`].
/// Relics: level-gated entries from [`PlayerProgress::available_relics`], plus
/// any transformation successor that is [`PlayerProgress::transformation_successor_visible`]
/// (discovered via burn — those ids are intentionally omitted from `available_relics`).
/// Other tabs list only content the player has already surfaced (yaku scored,
/// bosses met, talismans bought, chronicle runs).
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
                .filter(|d| available.contains(&d.id) || is_transformation_successor_relic(d.id))
                .map(|d| Artifact {
                    name: d.name.to_string(),
                    unlocked: true,
                    kind: ArtifactKind::Relic(d.id),
                    accent: color::rarity(d.rarity.tier()),
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
        Tab::Chronicle => {
            let indices = archive_career::chronicle_indices_recent_first(progress);
            let mut out = Vec::with_capacity(indices.len() + 1);
            out.push(Artifact {
                name: "Summary".into(),
                unlocked: true,
                kind: ArtifactKind::ChronicleSummary,
                accent: color::CHAMPAGNE,
            });
            for (list_i, &idx) in indices.iter().enumerate() {
                let Some(rec) = progress.run_history.get(idx) else {
                    continue;
                };
                let display =
                    archive_career::chronicle_display_run_number(list_i + 1, progress).unwrap_or(0);
                out.push(Artifact {
                    name: archive_career::chronicle_run_log_title(progress, display, rec),
                    unlocked: true,
                    kind: ArtifactKind::ChronicleRun(idx),
                    accent: color::PARCHMENT,
                });
            }
            out
        }
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
            run.gold,
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
        ArtifactKind::ChronicleSummary => {
            let n = archive_career::chronicle_indices_recent_first(progress).len();
            format!(
                "Career overview across {n} recorded run{}.\nUse ↑↓ to select a run for its full record.",
                if n == 1 { "" } else { "s" }
            )
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
        ArtifactKind::ChronicleSummary => {}
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

/// Horizontal `world_z` for title-bar wood tablets — lerp from the look target
/// toward the camera so the HUD clears the cabinet depth buffer (lit mesh uses
/// `Less`; a shallow step left the tablets fighting the room shell at top-of-screen
/// pixels, so they depth-tested away while 2D focus rings still drew).
#[inline]
fn collection_chrome_tablet_plane_z(cam: &CameraParams) -> f32 {
    let eye_z = cam.eye[2];
    let target_z = cam.target[2];
    target_z + (eye_z - target_z) * 0.42
}

/// Orients the wood tablet's local **+Y** face (decal side) toward the camera. Archive
/// cameras can yaw off the table axis; [`camera_facing_euler_xyz_rad`] is pitch-only and
/// can leave the tablet edge-on (a vanishingly thin silhouette) for oblique views.
#[inline]
fn collection_chrome_tablet_rotation(
    w: f32,
    h: f32,
    cam: &CameraParams,
    cx: f32,
    cy: f32,
    plane_z: f32,
) -> [f32; 3] {
    let center = world_on_camera_ray_plane_z(w, h, cam, cx, cy, plane_z);
    let eye = Vec3::from_array(cam.eye);
    let mut forward = eye - center;
    if forward.length_squared() < 1e-8 {
        return camera_facing_euler_xyz_rad(cam.eye, cam.target);
    }
    forward = forward.normalize();
    // `from_rotation_arc` is unstable when `forward` ≈ ±Y (see `wood_tablet_mesh`).
    if forward.y.abs() > 0.97 {
        return camera_facing_euler_xyz_rad(cam.eye, cam.target);
    }
    let q = Quat::from_rotation_arc(Vec3::Y, forward);
    mat4_to_euler_xyz_rad(Mat4::from_quat(q.normalize()))
}

/// Lacquered wood push-button for Archive title-bar chrome (`Back`, `Switch save`).
/// Copy is drawn as 2D text in [`CollectionScene::draw_collection_frame`]; the mesh is
/// label-free so engraved decals are not doubled when the tablet is visible.
#[inline]
fn collection_chrome_wood_tablet(
    w: f32,
    h: f32,
    rect: [f32; 4],
    cam: &CameraParams,
    plane_z: f32,
    focused: bool,
    arrange_name: &'static str,
) -> Object3d {
    let cx = rect[0] + rect[2] * 0.5;
    let cy = rect[1] + rect[3] * 0.5;
    let rw = rect[2];
    let rh = rect[3];
    let thickness = (rh * 0.35).max(8.0);
    Object3d {
        pos: object3d_pos_for_screen_at_world_z(w, h, cam, cx, cy, plane_z),
        extents: [rw, thickness, rh],
        rotation: collection_chrome_tablet_rotation(w, h, cam, cx, cy, plane_z),
        color: [1.0, 1.0, 1.0, 1.0],
        kind: Object3dKind::WoodTablet {
            label: std::borrow::Cow::Borrowed(""),
            pick_id: None,
        },
        hover_target: if focused { 1.0 } else { 0.0 },
        anim_id: 0,
        arrange_name: Some(arrange_name),
    }
}

/// Screen pixel → world on `z = plane_z` using the same view-proj as the frame, then packed for
/// [`Object3d::pos`] (see [`surface_anchor_from_world_xyz`]). Used when the Archive draws with the
/// embedded `archive.glb` camera so HUD plaques track the perspective view instead of the virtual
/// grid-ease camera offsets.
#[inline]
fn collection_hud_anchor_on_cam_plane(
    w: f32,
    h: f32,
    cam: &CameraParams,
    screen_px: f32,
    screen_py: f32,
    plane_z: f32,
) -> [f32; 3] {
    let p = world_on_camera_ray_plane_z(w, h, cam, screen_px, screen_py, plane_z);
    surface_anchor_from_world_xyz(w, h, p)
}

/// Push one catalog cell's 3D prop at a [`WorldSurfaceAnchor`](crate::render::draw_cmd::WorldSurfaceAnchor).
///
/// `cubby_zodiac` is folded in for `ArtifactKind::Zodiac` only so the ribbon
/// can be re-centred inside its cubby via arrange mode (shared target so
/// every cubby ribbon moves together).
struct CollectionGridCellObject3d<'a> {
    plaques: &'a mut Vec<Object3d>,
    boss: &'a Artifact,
    anchor: [f32; 3],
    cell: f32,
    fade: f32,
    is_focus: bool,
    boss_i: i32,
    cubby_zodiac: &'a crate::ui::placement::Placement,
    layout: &'a crate::ui::layout::LayoutResult,
}

fn collection_push_grid_cell_object3d(p: CollectionGridCellObject3d<'_>) {
    let CollectionGridCellObject3d {
        plaques,
        boss,
        anchor,
        cell,
        fade,
        is_focus,
        boss_i,
        cubby_zodiac,
        layout,
    } = p;
    let cx = anchor[0];
    let cz = anchor[2];
    let nameplate_py = anchor[1] + if is_focus { cell * 0.5 } else { cell * 0.15 };
    let plate_w = cell * if is_focus { 0.78 } else { 0.62 };
    let plate_h = cell * if is_focus { 0.78 } else { 0.62 };
    let plate_thick = cell * 0.06;
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
            let silhouette = !boss.unlocked;
            let visual = crate::core::relic::relic_visual(*relic_id);
            let face = plate_w;
            let thick = face * 0.06 * visual.thickness_scale;
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
            plaques.push(Object3d {
                pos: [cx, nameplate_py, cz],
                extents: [face, thick, face],
                rotation: euler_xyz_rad_from_deg(180.0 + visual.ui_tilt_x_deg, 0.0, 0.0),
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
            plaques.push(Object3d {
                pos: [cx, nameplate_py, cz],
                extents: [plate_w * 1.40, plate_w * 2.0, plate_w * 0.36],
                rotation: euler_xyz_rad_from_deg(-90.0, 14.0, 0.0),
                color: bright,
                kind: Object3dKind::Talisman { kind: *tk },
                hover_target: if is_focus { 1.0 } else { 0.0 },
                anim_id: boss_i as u64,
                arrange_name: None,
            });
        }
        ArtifactKind::Zodiac(zk) => {
            let zodiac_anchor = crate::ui::placement::PlacementAnchor::new(
                [cx, nameplate_py, cz],
                rot_fixed_axes_deg(90.0, 0.0, 0.0),
                cubby_zodiac,
                "collection.cubby_zodiac",
                layout,
            );
            plaques.push(zodiac_ribbon_object3d(ZodiacRibbonSpec {
                pos: zodiac_anchor.pos,
                length: plate_w,
                rotation: zodiac_anchor.object3d_rotation(),
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Some(*zk),
                hover_target: if is_focus { 1.0 } else { 0.0 },
                anim_id: boss_i as u64,
                arrange_name: Some(zodiac_anchor.arrange_name),
            }));
        }
        ArtifactKind::PlaqueOnly => {
            use crate::render::primitive::{DecalLayout, DecalSpec, MaterialSpec, MeshId};
            let lum = fade.max(if is_focus { 1.0 } else { 0.55 }).min(1.0);
            let wood_tint = [lum, lum, lum, 1.0];
            plaques.push(Object3d {
                pos: [cx, nameplate_py, cz],
                extents: [plate_w, plate_h, plate_thick],
                rotation: euler_xyz_rad_from_deg(90.0, 0.0, 0.0),
                color: wood_tint,
                kind: Object3dKind::Primitive {
                    shape: MeshId::BeveledSlab,
                    material: MaterialSpec::lacquered_wood_flat().with_decal(DecalSpec {
                        text: boss.name.clone(),
                        layout: DecalLayout::Fit {
                            target_short_edge: crate::render::decal::PLAQUE_DECAL_HEIGHT,
                        },
                    }),
                    pick_id: None,
                    shadow_caster: false,
                    silhouette: false,
                },
                hover_target: if is_focus { 1.0 } else { 0.0 },
                anim_id: boss_i as u64,
                arrange_name: None,
            });
        }
        ArtifactKind::ChronicleSummary => {}
        ArtifactKind::ChronicleRun(_) => {
            use crate::render::primitive::{DecalLayout, DecalSpec, MaterialSpec, MeshId};
            plaques.push(Object3d {
                pos: [cx, nameplate_py, cz],
                extents: [plate_w, plate_h, plate_thick],
                rotation: euler_xyz_rad_from_deg(90.0, 0.0, 0.0),
                color: bright,
                kind: Object3dKind::Primitive {
                    shape: MeshId::BeveledSlab,
                    material: MaterialSpec::lacquered_wood_flat().with_decal(DecalSpec {
                        text: boss.name.clone(),
                        layout: DecalLayout::Fit {
                            target_short_edge: crate::render::decal::PLAQUE_DECAL_HEIGHT,
                        },
                    }),
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

/// Clamp / default `DrawCtx::room_gltf_height_scale` so glTF room matrices stay finite.
#[inline]
fn collection_sanitized_room_gltf_height_scale(env_h: f32) -> f32 {
    if env_h.is_finite() && env_h > 1e-4 {
        env_h.clamp(0.03, 25.0)
    } else {
        crate::render::room_glb::SHOP_ENV_HEIGHT_SCALE
    }
}

#[inline]
fn flat_rect_xywh_is_finite(rect: [f32; 4]) -> bool {
    let [x, y, rw, rh] = rect;
    x.is_finite() && y.is_finite() && rw.is_finite() && rh.is_finite() && rw > 0.0 && rh > 0.0
}

/// Screen-space AABB hit targets for catalog cells (`FlatItem` rects), for spatial D-pad / arrow
/// moves between neighbors.
fn collection_artifact_hit_rects(items: &[FlatItem<CollectionAction>]) -> Vec<(usize, [f32; 4])> {
    items
        .iter()
        .filter_map(|it| match it.action {
            CollectionAction::SelectArtifact(i) => Some((i, it.rect)),
            _ => None,
        })
        .collect()
}

/// Next artifact index when moving from `focused` in `dir` using each cell's screen rect, or
/// [`None`] when no neighbor qualifies (caller may fall back to scroll / grid index rules).
fn collection_spatial_artifact_step(
    items: &[FlatItem<CollectionAction>],
    focused: Option<usize>,
    dir: FocusDir,
) -> Option<usize> {
    let candidates = collection_artifact_hit_rects(items);
    if candidates.is_empty() {
        return None;
    }
    let cur_rect = focused
        .and_then(|fi| candidates.iter().find(|(i, _)| *i == fi).map(|(_, r)| *r))
        .or_else(|| candidates.first().map(|(_, r)| *r))?;
    pick_neighbor(cur_rect, dir, &candidates)
}

#[inline]
fn screen_hit_anchor_is_finite(sx: f32, sy: f32, rw: f32, rh: f32) -> bool {
    sx.is_finite() && sy.is_finite() && rw.is_finite() && rh.is_finite() && rw > 0.0 && rh > 0.0
}

/// Items shown per cabinet page when the `archive.glb` room is loaded.
#[inline]
fn archive_page_size() -> usize {
    archive_glb::ARCHIVE_SLOT_COUNT
}

/// Number of cabinet pages required to display `all_count` items
/// (always at least 1 so the empty-tab UI still has a page indicator).
#[inline]
fn archive_page_count(all_count: usize) -> usize {
    let ps = archive_page_size().max(1);
    if all_count == 0 {
        1
    } else {
        all_count.div_ceil(ps)
    }
}

#[inline]
fn archive_page_for_idx(idx: usize) -> usize {
    idx / archive_page_size().max(1)
}

fn collection_scroll_catalog_row_into_view(
    scene: &mut CollectionScene,
    row: usize,
    max_scroll: f32,
    visible_rows: usize,
) {
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
}

fn collection_sync_artifact_focus_to_idx(
    scene: &mut CollectionScene,
    idx: usize,
    cols: usize,
    max_scroll: f32,
    visible_rows: usize,
) {
    let prev = scene.focused_row;
    scene.focused_row = Some(idx);
    if matches!(scene.active_tab, Tab::Chronicle) && prev != Some(idx) {
        scene.chronicle_dashboard_scroll.set(0.0);
    }
    scene
        .tree
        .set_focus(CollectionAction::SelectArtifact(idx).id());
    if archive_glb::archive_room_draw_ready() {
        // Page-based browsing: snap the visible cabinet to the page that
        // contains `idx`. Vertical row-scroll state is unused on this path.
        scene.archive_page = archive_page_for_idx(idx);
    } else {
        collection_scroll_catalog_row_into_view(scene, idx / cols, max_scroll, visible_rows);
    }
}

/// (row, col) within the active archive page (0-indexed; cols < `ARCHIVE_SLOT_COLS`).
/// Returns (0, 0) when no item is focused so callers can land on slot 0.
fn archive_focus_row_col_in_page(focused: Option<usize>) -> (usize, usize) {
    let cols = archive_glb::ARCHIVE_SLOT_COLS.max(1);
    let page_size = archive_page_size().max(1);
    let slot = focused.map(|i| i % page_size).unwrap_or(0);
    (slot / cols, slot % cols)
}

/// Flip the archive cabinet by `dir` pages (positive = next, negative = prev) and land focus on
/// the slot corresponding to `target_in_page = (row, col)` within the new page. The candidate
/// slot is clamped to the last present artifact, so partial pages still land on a real item.
///
/// Caller chooses `target_in_page` based on gesture intent:
///   - **Directional edge-cross** (← / → walked off the column edge): preserve the row, swap the
///     column to the opposite edge so Right-then-Left is a no-op on full pages.
///   - **Bulk page-flip** (PgUp/PgDn, mouse wheel, footer arrows): preserve both row and column
///     so the gesture is fully reversible.
///
/// No-ops when the catalogue is empty or already at the requested edge.
fn archive_page_step(
    scene: &mut CollectionScene,
    bus: &mut crate::game::event_bus::EventBus,
    dir: i32,
    target_in_page: (usize, usize),
    all_count: usize,
) {
    if all_count == 0 || dir == 0 {
        return;
    }
    let page_count = archive_page_count(all_count);
    let cur = scene.archive_page.min(page_count.saturating_sub(1));
    let next = if dir > 0 {
        (cur + 1).min(page_count.saturating_sub(1))
    } else {
        cur.saturating_sub(1)
    };
    if next == cur {
        return;
    }
    let cols = archive_glb::ARCHIVE_SLOT_COLS.max(1);
    let page_size = archive_page_size().max(1);
    let target_row = target_in_page
        .0
        .min(archive_glb::ARCHIVE_SLOT_ROWS.saturating_sub(1));
    let target_col = target_in_page.1.min(cols - 1);
    let local_slot = (target_row * cols + target_col).min(page_size - 1);
    let candidate = next * page_size + local_slot;
    let new_focus = candidate.min(all_count - 1);
    scene.archive_page = next;
    scene.focused_row = Some(new_focus);
    scene
        .tree
        .set_focus(CollectionAction::SelectArtifact(new_focus).id());
    bus.push(GameEvent::UiSound(SfxId::UiConfirm));
}

/// Spatial neighbour move within the current archive page; if the requested
/// neighbour falls off the page horizontally we flip to the adjacent page.
/// Vertical edges (no Up / Down neighbour) hand focus to the chrome bar above
/// (Back / Switch save) or below (Prev / Next) so controller users can reach
/// every button. Returns whether the gesture consumed input — `false` means the
/// caller should consider further fallback (e.g. chrome entry on a procedural
/// page that has no spatial neighbour at all).
fn archive_directional_step(
    scene: &mut CollectionScene,
    bus: &mut crate::game::event_bus::EventBus,
    items: &[FlatItem<CollectionAction>],
    dir: FocusDir,
    all_count: usize,
) -> bool {
    if all_count == 0 {
        return false;
    }
    if let Some(ni) = collection_spatial_artifact_step(items, scene.focused_row, dir)
        && Some(ni) != scene.focused_row
    {
        bus.push(GameEvent::UiSound(SfxId::TilePlace));
        scene.focused_row = Some(ni);
        scene.archive_page = archive_page_for_idx(ni);
        scene
            .tree
            .set_focus(CollectionAction::SelectArtifact(ni).id());
        return true;
    }
    // No spatial neighbour exists in the requested direction. For Left/Right that
    // means we're at a page edge column — flip the page so navigation feels
    // continuous, preserving the row so → then ← (or vice versa) returns the
    // player to where they came from. Up/Down at top/bottom row defers to the
    // caller so the title-bar / footer chrome can claim focus.
    let (from_row, _) = archive_focus_row_col_in_page(scene.focused_row);
    let last_col = archive_glb::ARCHIVE_SLOT_COLS.saturating_sub(1);
    match dir {
        FocusDir::Right => {
            archive_page_step(scene, bus, 1, (from_row, 0), all_count);
            true
        }
        FocusDir::Left => {
            archive_page_step(scene, bus, -1, (from_row, last_col), all_count);
            true
        }
        _ => false,
    }
}

/// Chrome buttons currently in the flat-item list. Used for spatial neighbour
/// picks when the player walks off the artifact grid vertically and for
/// chrome ↔ chrome traversal once focus has parked on a button.
fn collection_chrome_rects(
    items: &[FlatItem<CollectionAction>],
) -> Vec<(CollectionAction, [f32; 4])> {
    items
        .iter()
        .filter_map(|it| match it.action {
            CollectionAction::Back
            | CollectionAction::SwitchSave
            | CollectionAction::PrevPage
            | CollectionAction::NextPage
            | CollectionAction::PrevTab
            | CollectionAction::NextTab => Some((it.action, it.rect)),
            _ => None,
        })
        .collect()
}

#[inline]
fn chronicle_panel_rect(
    w: f32,
    h: f32,
    progress: &crate::core::progression::PlayerProgress,
) -> [f32; 4] {
    let ch = archive_chrome_layout(w, h);
    let count = tab_artifacts(Tab::Chronicle, progress).len().max(1);
    let layout = compute_layout(w, h, ch.scale, Tab::Chronicle, count);
    [
        ch.margin_x,
        layout.band_top_y,
        w - ch.margin_x * 2.0,
        (layout.band_bottom_y - layout.band_top_y).max(1.0),
    ]
}

#[inline]
fn chronicle_sync_run_log_scroll(
    scene: &mut CollectionScene,
    w: f32,
    h: f32,
    progress: &crate::core::progression::PlayerProgress,
) {
    let panel = chronicle_panel_rect(w, h, progress);
    let entry_count = archive_career::chronicle_list_entry_count(progress);
    let panes = crate::ui::chronicle_dashboard::chronicle_pane_layout(w, h, panel);
    let scroll = crate::ui::chronicle_dashboard::chronicle_clamp_run_log_scroll(
        scene.chronicle_run_log_scroll.get(),
        scene.focused_row,
        entry_count,
        panes,
    );
    let max_s =
        crate::ui::chronicle_dashboard::chronicle_run_log_scroll_max(w, h, panel, entry_count);
    scene.chronicle_run_log_scroll.set(scroll.clamp(0.0, max_s));
}

#[inline]
fn chronicle_apply_footer_page_scroll(
    scene: &mut CollectionScene,
    w: f32,
    h: f32,
    progress: &crate::core::progression::PlayerProgress,
    direction: f32,
) {
    let panel = chronicle_panel_rect(w, h, progress);
    let max_s = crate::ui::chronicle_dashboard::chronicle_right_pane_scroll_max(
        w,
        h,
        panel,
        progress,
        scene.focused_row,
    );
    let step = h * 0.22 * direction;
    let next = (scene.chronicle_dashboard_scroll.get() + step).clamp(0.0, max_s);
    scene.chronicle_dashboard_scroll.set(next);
}

#[inline]
fn collection_chrome_is_top(action: CollectionAction) -> bool {
    matches!(
        action,
        CollectionAction::Back | CollectionAction::SwitchSave
    )
}

#[inline]
fn collection_chrome_is_bottom(action: CollectionAction) -> bool {
    matches!(
        action,
        CollectionAction::PrevPage
            | CollectionAction::NextPage
            | CollectionAction::PrevTab
            | CollectionAction::NextTab
    )
}

#[inline]
fn rect_center_x(rect: [f32; 4]) -> f32 {
    rect[0] + rect[2] * 0.5
}

/// Chrome button fill colour — slight lift when focused so the brass ring
/// (and the button itself) reads "active" even on low-contrast displays.
#[inline]
fn chrome_btn_color(focused: bool) -> [f32; 4] {
    if focused {
        color::alpha(color::WALNUT_BRIGHT, 0.98)
    } else {
        color::alpha(color::WALNUT_SOFT, 0.94)
    }
}

fn collection_focused_artifact_center_x(
    items: &[FlatItem<CollectionAction>],
    focused: Option<usize>,
) -> f32 {
    if let Some(fi) = focused {
        for it in items {
            if let CollectionAction::SelectArtifact(idx) = it.action
                && idx == fi
            {
                return rect_center_x(it.rect);
            }
        }
    }
    // No focused artifact rect — fall back to a chrome button's centre
    // (covers cases where the artifact grid is empty, e.g. the Chronicle
    // tab before the first run).
    items
        .iter()
        .find_map(|it| match it.action {
            CollectionAction::Back | CollectionAction::SwitchSave => Some(rect_center_x(it.rect)),
            _ => None,
        })
        .unwrap_or(0.0)
}

/// Park focus on the chrome button nearest the current artifact column.
/// `dir` selects the title bar (Up) or the footer (Down). No-op when the
/// flat item list doesn't surface the requested chrome row (e.g. archive
/// builds without the footer arrow pair).
fn collection_enter_chrome(
    scene: &mut CollectionScene,
    bus: &mut crate::game::event_bus::EventBus,
    items: &[FlatItem<CollectionAction>],
    dir: FocusDir,
) -> bool {
    let predicate: fn(CollectionAction) -> bool = match dir {
        FocusDir::Up => collection_chrome_is_top,
        FocusDir::Down => collection_chrome_is_bottom,
        _ => return false,
    };
    let candidates: Vec<(CollectionAction, [f32; 4])> = collection_chrome_rects(items)
        .into_iter()
        .filter(|(a, _)| predicate(*a))
        .collect();
    if candidates.is_empty() {
        return false;
    }
    let ref_x = collection_focused_artifact_center_x(items, scene.focused_row);
    let target = candidates
        .iter()
        .min_by(|a, b| {
            let dax = rect_center_x(a.1) - ref_x;
            let dbx = rect_center_x(b.1) - ref_x;
            dax.abs()
                .partial_cmp(&dbx.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(a, _)| *a)
        .unwrap_or(candidates[0].0);
    scene.focused_chrome = Some(target);
    scene.tree.set_focus(target.id());
    bus.push(GameEvent::UiSound(SfxId::TilePlace));
    true
}

/// Directional move while chrome is focused. Up / Down peels focus back to
/// the artifact grid; Left / Right hops between the two buttons sharing the
/// same chrome row.
fn collection_chrome_directional(
    scene: &mut CollectionScene,
    bus: &mut crate::game::event_bus::EventBus,
    items: &[FlatItem<CollectionAction>],
    dir: FocusDir,
) {
    let Some(cur) = scene.focused_chrome else {
        return;
    };
    let on_top = collection_chrome_is_top(cur);
    let on_bottom = collection_chrome_is_bottom(cur);
    match dir {
        FocusDir::Down if on_top => {
            scene.focused_chrome = None;
            if let Some(fi) = scene.focused_row {
                scene
                    .tree
                    .set_focus(CollectionAction::SelectArtifact(fi).id());
            }
            bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }
        FocusDir::Up if on_bottom => {
            scene.focused_chrome = None;
            if let Some(fi) = scene.focused_row {
                scene
                    .tree
                    .set_focus(CollectionAction::SelectArtifact(fi).id());
            }
            bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }
        FocusDir::Left | FocusDir::Right => {
            let same_row: fn(CollectionAction) -> bool = if on_top {
                collection_chrome_is_top
            } else if on_bottom {
                collection_chrome_is_bottom
            } else {
                return;
            };
            let same_row_rects: Vec<(CollectionAction, [f32; 4])> = collection_chrome_rects(items)
                .into_iter()
                .filter(|(a, _)| same_row(*a))
                .collect();
            let cur_rect = same_row_rects
                .iter()
                .find(|(a, _)| *a == cur)
                .map(|(_, r)| *r);
            if let Some(rect) = cur_rect
                && let Some(next) = pick_neighbor(rect, dir, &same_row_rects)
                && next != cur
            {
                scene.focused_chrome = Some(next);
                scene.tree.set_focus(next.id());
                bus.push(GameEvent::UiSound(SfxId::TilePlace));
            }
        }
        _ => {}
    }
}

/// Section tab hit rects from `section_buttons_*_bound` AABBs (left: Relics/Yaku; right: rest).
fn archive_section_tab_hit_rects(
    w: f32,
    h: f32,
    env_h: f32,
    cam: &CameraParams,
) -> Option<Vec<(usize, [f32; 4])>> {
    archive_glb::with_archive_glb_cpu(|opt| {
        let cpu = opt?;
        let left =
            room_glb::screen_rect_for_marker_mesh_bounds(&room_glb::MarkerScreenRectParams {
                win_w: w,
                win_h: h,
                cam,
                env_height_scale: env_h,
                cpu,
                node_name: archive_glb::SECTION_BUTTONS_LEFT_BOUND,
                min_rw: 48.0,
                min_rh: 32.0,
            })?;
        let right =
            room_glb::screen_rect_for_marker_mesh_bounds(&room_glb::MarkerScreenRectParams {
                win_w: w,
                win_h: h,
                cam,
                env_height_scale: env_h,
                cpu,
                node_name: archive_glb::SECTION_BUTTONS_RIGHT_BOUND,
                min_rw: 48.0,
                min_rh: 32.0,
            })?;
        let mut out = Vec::new();
        let distribute = |parent: [f32; 4], tabs: &[usize], out: &mut Vec<(usize, [f32; 4])>| {
            if tabs.is_empty() {
                return;
            }
            let [x, y, rw, rh] = parent;
            let n = tabs.len();
            // Wide section volumes: tabs in a horizontal row; tall narrow: vertical stack.
            if rw >= rh * 0.75 {
                let cw = rw / n as f32;
                for (k, &ti) in tabs.iter().enumerate() {
                    out.push((ti, [x + cw * k as f32, y, cw, rh]));
                }
            } else {
                let sh = rh / n as f32;
                for (k, &ti) in tabs.iter().enumerate() {
                    out.push((ti, [x, y + sh * k as f32, rw, sh]));
                }
            }
        };
        distribute(left, &[0, 1], &mut out);
        distribute(right, &[2, 3, 4], &mut out);
        Some(out)
    })
}

/// Compose the same view-projection matrix the renderer uses (must match
/// [`CameraParams::clip_planes`] + look_at_rh). Drift here makes hit rects misalign with visible 3D.
fn camera_view_proj(w: f32, h: f32, cam: &CameraParams) -> glam::Mat4 {
    let w = w.max(1.0);
    let h = h.max(1.0);
    let aspect = w / h;
    let view = glam::Mat4::look_at_rh(
        glam::Vec3::from_array(cam.eye),
        glam::Vec3::from_array(cam.target),
        glam::Vec3::from_array(cam.up),
    );
    let (near, far) = cam.clip_planes(h);
    let proj = glam::Mat4::perspective_rh(cam.fovy_deg.to_radians(), aspect, near, far);
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

    // ── Grid viewport (between title and footer) ───────────────────
    // The grid is always 6 columns wide and grows vertically with the
    // tab's universe. The visible band shows ~3.5 rows so the
    // half-row above/below is the affordance that more content
    // exists. Cells are square; their size is the smaller of "fit 6
    // across" or "fit visible_rows + 0.5 stacked".
    let grid_rows_total = total_rows_for(item_count);
    let grid_cols = GRID_COLS;
    let margin_x = w * 0.06;
    // Tight top margin + smaller footer reserve → taller scroll band →
    // larger cells. Browsing is the primary verb; orbit inspect should not
    // dominate the layout.
    let grid_y_top_band = title_y + title_h + h * 0.06;
    let footer_band_h = h * 0.10;
    let grid_y_bottom_band = arrow_y - footer_band_h;
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

    // Anchor only kept for the hit-test ray cast in `flat_items`. Unused for
    // 3D placement in [`CollectionScene::build_archive_grid_frame`].
    let shelf_top_lift = cell_w * 0.15;

    let cam_dist = h * 1.6;
    let cam_height = h * 1.3;
    let camera = CameraParams {
        eye: [0.0, -cam_dist, cam_height],
        target: [0.0, h * 0.05, h * 0.10],
        up: [0.0, 0.0, 1.0],
        fovy_deg: 36.0,
        clip_near: None,
        clip_far: None,
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
