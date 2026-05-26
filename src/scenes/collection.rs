//! Archive — five tabs (Relics → Talismans → Yaku → Bosses → Chronicle).
//! Each tab is a scrolling grid of artifacts on a backdrop plane; focus
//! shows a close-up and description on the room's sign boards (or a floating card when
//! inspecting). E / North opens orbit inspect.

use std::time::Instant;

use crate::audio::SfxId;
use crate::core::archive_seen::{self, ArchiveSeenMark, ArchiveTab};
use crate::core::ordeal::{OrdealKind, all_ordeals, final_ordeals};
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
use crate::render::table_transform::{
    euler_xyz_rad_from_deg, mat4_to_euler_xyz_rad, rot_fixed_axes_deg,
};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::render::world_space::{surface_anchor_from_world_xyz, world_on_camera_ray_plane_z};
use crate::ui::controller_hints::{
    HintKey, HintRow, HintSegment, HintStyle, inspect_camera_hint_row, inspect_dismiss_hint_row,
    push_inline_hint_rows,
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
    InspectDolly, InspectRig, ItemInspectOrbitState, inspect_orbit_camera, lerp_camera,
    prepend_inspect_orbit_subject_rotation, tick_inspect_dolly,
};
/// 2D chrome sizes shared by [`CollectionScene::draw_collection_frame`] and
/// [`CollectionScene::flat_items`] — tuned for legibility at TV distance.
#[derive(Clone, Copy)]
struct ArchiveChromeLayout {
    scale: f32,
    margin_x: f32,
    chrome_btn_h: f32,
}

fn archive_chrome_layout(w: f32, h: f32) -> ArchiveChromeLayout {
    let scale = metrics::scene_scale(w, h);
    let margin_x = w * 0.04;
    // ~5% of screen height, clamped so 720p sofas stay readable and 4K doesn't balloon.
    let chrome_btn_h = (h * 0.052).clamp(44.0, 72.0);
    ArchiveChromeLayout {
        scale,
        margin_x,
        chrome_btn_h,
    }
}

/// One catalog section. Each tab drives a separate grid of artifacts.
/// Yaku entries carry their matching Zodiac ribbon as the 3D prop — the
/// two concepts are 1:1, so keeping them as separate tabs was redundant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Relics,
    Yaku,
    Ordeals,
    Talismans,
    Chronicle,
}

/// Archive grid / featured talisman `Object3d::extents` vs `plate_w` / `closeup_size`.
/// Mesh local AABB is a regular octagon (diameter 1.0 × thickness 0.18).
const ARCHIVE_TALISMAN_EXTENTS: [f32; 3] = [1.0, 1.0, 0.36];

/// Carved face toward the archive camera; 14° Ry keeps holo sheen readable on the pedestal.
#[inline]
fn archive_talisman_rotation() -> [f32; 3] {
    crate::render::talisman_mesh::talisman_face_camera_rotation(14.0)
}

const TABS: [Tab; 5] = [
    Tab::Relics,
    Tab::Talismans,
    Tab::Yaku,
    Tab::Ordeals,
    Tab::Chronicle,
];

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Relics => "Relics",
            Tab::Yaku => "Yaku",
            Tab::Ordeals => "Ordeals",
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
    /// Unseen since surfacing in this tab's catalog.
    is_new: bool,
}

#[derive(Clone, Debug)]
enum ArtifactKind {
    Relic(RelicId),
    Talisman(TalismanKind),
    Zodiac(ZodiacKind),
    /// Ordeal encounter — flat icon from the ordeal atlas (no 3D mesh).
    Ordeal(OrdealKind),
    /// Index into [`crate::core::progression::PlayerProgress::run_history`].
    ChronicleRun(usize),
    /// Aggregate career view (run-log row 0).
    ChronicleSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CollectionAction {
    Back,
    SwitchSave,
    /// Direct tab pick when `archive.glb` provides `btn_*_tab` meshes.
    SelectTab(usize),
    /// Click on an artifact in the current tab's row → set as the
    /// featured item for [`ItemInspectScene`] orbit. Indexes into the
    /// active tab's artifact list globally so the selection survives
    /// scroll position changes.
    SelectArtifact(usize),
    /// Chronicle: focus the career / run-detail column for scroll and D-pad.
    ChronicleFocusCareer,
    /// Cabinet page step (`btn_page_left` / `btn_page_right` in `archive.glb`).
    PrevPage,
    NextPage,
}

impl CollectionAction {
    fn id(self) -> FocusId {
        match self {
            CollectionAction::Back => FocusId(20),
            CollectionAction::SwitchSave => FocusId(23),
            CollectionAction::PrevPage => FocusId(24),
            CollectionAction::NextPage => FocusId(25),
            CollectionAction::SelectTab(i) => FocusId(400 + i as u32),
            // SelectArtifact IDs start at 200. The widget tree just needs
            // unique IDs per hit target — the values themselves don't matter.
            CollectionAction::SelectArtifact(i) => FocusId(200 + i as u32),
            CollectionAction::ChronicleFocusCareer => FocusId(199),
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
    /// Settings chronicle cursor captured at Archive entry (stable for this visit).
    chronicle_last_seen: Option<u32>,
    /// Set when the player opens the Chronicle tab; gates persisting the chronicle cursor on exit.
    visited_chronicle: bool,
    /// Current page index when the `archive.glb` room is loaded. The cabinet shows
    /// `ARCHIVE_SLOT_COUNT` items per page; navigation pages flip rather than
    /// horizontally / vertically scroll a sliding window.
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
    /// (Back / Switch save / tab bar / footer Prev / Next) rather than on an
    /// artifact. Pressing Up from the top row of the cabinet enters the tab
    /// bar; another Up from a tab reaches the title bar. Down from the title
    /// bar enters tabs; Down from tabs returns to the grid. Down from the
    /// bottom grid row enters the footer. Cleared by tab / page changes and
    /// any move back into the artifact grid. The artifact selection in
    /// `focused_row` is preserved so direction-reversal lands on the same cell.
    focused_chrome: Option<CollectionAction>,
    /// Vertical scroll (px) for the Chronicle career pane (right).
    chronicle_dashboard_scroll: std::cell::Cell<f32>,
    /// Vertical scroll (px) for the Chronicle run log (left).
    chronicle_run_log_scroll: std::cell::Cell<f32>,
    /// Which Chronicle column owns directional scroll (left run log vs right career/detail).
    chronicle_focused_pane: crate::ui::chronicle_dashboard::ChronicleScrollPane,
}

impl CollectionScene {
    pub fn new() -> Self {
        Self::with_active_tab(Tab::Relics)
    }

    /// Headless screenshot: open the Chronicle tab with a clean scroll/camera state.
    pub fn prepare_chronicle_for_screenshot(&mut self) {
        *self = Self::with_active_tab(Tab::Chronicle);
        self.focused_row = Some(0);
        self.chronicle_run_log_scroll.set(0.0);
        self.chronicle_dashboard_scroll.set(0.0);
    }

    /// Headless screenshot: open the Ordeals tab on page 0 with the first entry focused.
    pub fn prepare_ordeals_for_screenshot(&mut self) {
        *self = Self::with_active_tab(Tab::Ordeals);
        self.focused_row = Some(0);
        self.archive_page = 0;
    }

    /// Headless screenshot: open the Talismans tab (featured pedestal + cubbies).
    pub fn prepare_talismans_for_screenshot(&mut self) {
        *self = Self::with_active_tab(Tab::Talismans);
        self.focused_row = None;
        self.archive_page = 0;
    }

    pub fn is_chronicle_tab(&self) -> bool {
        matches!(self.active_tab, Tab::Chronicle)
    }

    fn with_active_tab(active_tab: Tab) -> Self {
        Self {
            tree: TreeState::new(),
            positions: crate::ui::scene_layout::CollectionPositions::default(),
            active_tab,
            selected_artifact: None,
            focused_row: Some(0),
            chronicle_last_seen: None,
            visited_chronicle: false,
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
            chronicle_focused_pane: crate::ui::chronicle_dashboard::ChronicleScrollPane::RunLog,
        }
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
        if let Some(chrome) = [
            CollectionAction::Back,
            CollectionAction::SwitchSave,
            CollectionAction::PrevPage,
            CollectionAction::NextPage,
        ]
        .into_iter()
        .find(|&chrome| chrome.id() == f)
        {
            return Some(chrome);
        }
        (0..TABS.len())
            .map(CollectionAction::SelectTab)
            .find(|a| a.id() == f)
    }

    fn cycle_tab(
        &mut self,
        forward: bool,
        progress: &crate::core::progression::PlayerProgress,
        chronicle_last_seen: u32,
        bus: &mut crate::game::event_bus::EventBus,
    ) {
        let idx = TABS.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        let next = if forward {
            (idx + 1) % TABS.len()
        } else {
            (idx + TABS.len() - 1) % TABS.len()
        };
        enter_tab(self, TABS[next], progress, chronicle_last_seen, bus);
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
        chronicle_last_seen: u32,
    ) -> Vec<FlatItem<CollectionAction>> {
        let env_h = collection_sanitized_room_gltf_height_scale(env_h);
        let footer_prev_action = CollectionAction::PrevPage;
        let footer_next_action = CollectionAction::NextPage;

        let mut items = Vec::new();
        let cam = archive_glb::archive_camera_base(w, h, env_h);
        if let Some(back_rect) = archive_main_menu_btn_rect(w, h, &cam, env_h) {
            items.push(FlatItem::new(
                CollectionAction::Back.id(),
                back_rect,
                CollectionAction::Back,
            ));
        }
        if let Some(switch_rect) = archive_switch_save_btn_rect(w, h, &cam, env_h) {
            items.push(FlatItem::new(
                CollectionAction::SwitchSave.id(),
                switch_rect,
                CollectionAction::SwitchSave,
            ));
        }
        if collection_uses_footer_arrows(self.active_tab) {
            let nav = archive_page_nav(self.active_tab, progress, self.archive_page);
            if nav.show_prev
                && let Some(rect) = archive_page_left_btn_rect(w, h, &cam, env_h)
            {
                items.push(FlatItem::new(
                    footer_prev_action.id(),
                    rect,
                    footer_prev_action,
                ));
            }
            if nav.show_next
                && let Some(rect) = archive_page_right_btn_rect(w, h, &cam, env_h)
            {
                items.push(FlatItem::new(
                    footer_next_action.id(),
                    rect,
                    footer_next_action,
                ));
            }
        }

        let all = tab_artifacts(self.active_tab, progress, chronicle_last_seen);
        for (ti, rect) in archive_tab_hit_rects(w, h, env_h, &cam) {
            if !flat_rect_xywh_is_finite(rect) {
                continue;
            }
            items.push(FlatItem::new(
                CollectionAction::SelectTab(ti).id(),
                rect,
                CollectionAction::SelectTab(ti),
            ));
        }
        if matches!(self.active_tab, Tab::Chronicle) {
            push_chronicle_flat_items(
                &mut items,
                w,
                h,
                progress,
                self.chronicle_run_log_scroll.get(),
            );
        } else if !all.is_empty() {
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

        items
    }

    /// World-space anchor for [`ItemInspectScene`] orbit (matches HUD close-up).
    fn collection_inspect_target_world(
        &self,
        _w: f32,
        h: f32,
        bosses: &[Artifact],
        _layout: &crate::ui::layout::LayoutResult,
        env_h: f32,
    ) -> Option<[f32; 3]> {
        let env_h = collection_sanitized_room_gltf_height_scale(env_h);
        if bosses.is_empty() {
            return None;
        }
        archive_glb::with_archive_glb_cpu(|opt| {
            let cpu = opt?;
            let m = archive_glb::archive_marker_world_mat4(
                h,
                env_h,
                cpu,
                archive_glb::ARCHIVE_SPAWN_FOCUSED_ITEM,
            )?;
            Some(m.transform_point3(Vec3::ZERO).to_array())
        })
    }

    fn collection_inspect_orbit_for_focus(
        &self,
        w: f32,
        h: f32,
        bosses: &[Artifact],
        layout: &crate::ui::layout::LayoutResult,
        env_h: f32,
    ) -> Option<ItemInspectOrbitState> {
        if matches!(self.active_tab, Tab::Chronicle) {
            return None;
        }
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
        let chronicle_last_seen = self.chronicle_last_seen.unwrap_or(0);
        let bosses = tab_artifacts(self.active_tab, progress, chronicle_last_seen);
        self.collection_inspect_orbit_for_focus(
            w,
            h,
            &bosses,
            layout,
            crate::render::room_glb::SHOP_ENV_HEIGHT_SCALE,
        )
    }

    /// Move archive inspect focus while [`CollectionInspectPresenter`] is active.
    /// Returns true when the focused artifact changed.
    pub(crate) fn inspect_cycle_focus(
        &mut self,
        dir: FocusDir,
        w: f32,
        h: f32,
        progress: &crate::core::progression::PlayerProgress,
        env_h: f32,
        bus: &mut crate::game::event_bus::EventBus,
    ) -> bool {
        if matches!(self.active_tab, Tab::Chronicle) {
            return false;
        }
        let chronicle_last_seen = self.chronicle_last_seen.unwrap_or(0);
        let all_count = tab_artifacts(self.active_tab, progress, chronicle_last_seen).len();
        if all_count == 0 {
            return false;
        }
        let items = self.flat_items(w, h, progress, env_h, chronicle_last_seen);
        if archive_directional_step(
            self,
            bus,
            &items,
            dir,
            all_count,
            progress,
            chronicle_last_seen,
        ) {
            self.selected_artifact = self.focused_row;
            self.focused_chrome = None;
            return true;
        }
        false
    }

    /// Build the 3D frame for the active Archive tab: grid on a plane, camera
    /// eased to the focused cell, plus close-up and description on GLB signs (or a floating card
    /// when inspecting).
    fn build_archive_grid_frame(
        &self,
        mut frame: UiFrame,
        mut quads: Vec<GpuInstance>,
        text_labels: &mut Vec<TextLabel>,
        bosses: &[Artifact],
        ctx: &DrawCtx<'_>,
        inspect: Option<&ItemInspectOrbitState>,
    ) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let env_scale = collection_sanitized_room_gltf_height_scale(ctx.room_gltf_height_scale);
        let chronicle_dashboard = matches!(self.active_tab, Tab::Chronicle) && inspect.is_none();
        if !chronicle_dashboard {
            frame.archive_environment();
        }
        frame.archive_sign_description_decal_text = None;

        let cell = (w * 0.12).min(h * 0.18);
        let total_cells = bosses.len() as i32;
        let default_focus = total_cells.min(2);
        let focus_flat = self
            .focused_row
            .map(|i| i as i32)
            .unwrap_or(default_focus)
            .clamp(0, total_cells.saturating_sub(1).max(0));
        let base_cam = archive_glb::archive_camera_base(w, h, env_scale);
        let coll_rig = InspectRig::collection(h);
        // Compute the inspect-orbit camera for this frame (when inspect is on) and cache
        // it so the dolly-out lerp can keep sampling that endpoint after the overlay
        // pops and `ItemInspectOrbitState` is gone.
        let inspect_cam_now: Option<CameraParams> =
            inspect.map(|ins| inspect_orbit_camera(ins, &coll_rig));
        if let Some(ic) = inspect_cam_now {
            self.last_inspect_cam.set(Some(ic));
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

        if chronicle_dashboard {
            frame.archive_description_sign_use_left = None;
            frame.archive_page_left_visible = false;
            frame.archive_page_right_visible = false;
        } else {
            let nav = archive_page_nav(self.active_tab, ctx.progress, self.archive_page);
            frame.archive_page_left_visible =
                collection_uses_footer_arrows(self.active_tab) && nav.show_prev;
            frame.archive_page_right_visible =
                collection_uses_footer_arrows(self.active_tab) && nav.show_next;
            let ref_x = match ctx.input_mode {
                crate::ui::input::InputMode::Cursor => ctx.cursor_pos.0,
                crate::ui::input::InputMode::Keyboard | crate::ui::input::InputMode::Controller => {
                    let focused_slot_in_page = (focus_flat as usize) % archive_page_size().max(1);
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
            }
        }

        let mut plaques: Vec<Object3d> = if chronicle_dashboard {
            Vec::new()
        } else {
            Vec::with_capacity(archive_glb::ARCHIVE_SLOT_COUNT + 8)
        };

        if !chronicle_dashboard {
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
            push_archive_cubby_new_badges(
                bosses,
                page,
                focus_flat,
                &anchors,
                w,
                h,
                &final_cam,
                cell,
                &mut quads,
                text_labels,
            );
        }

        // ── Foreground: close-up + description plaque ───────────────
        // In front of the grid plane; camera easing keeps them stable on screen.
        let mut hud_plaques: Vec<Object3d> = Vec::new();

        if !chronicle_dashboard && let Some(boss) = bosses.get(focus_flat as usize) {
            let with_inspect_spin = |o: Object3d| -> Object3d {
                if let Some(ins) = inspect {
                    prepend_inspect_orbit_subject_rotation(o, ins, &coll_rig)
                } else {
                    o
                }
            };
            let closeup_anim = if inspect.is_some() {
                crate::render::draw_cmd::SHOP_INSPECT_SUBJECT_ANIM_ID
            } else {
                crate::render::draw_cmd::ARCHIVE_FEATURED_ANIM_ID
            };
            let closeup_size = cell * 0.95;
            let (closeup_ax, closeup_ay, closeup_az) = archive_glb::with_archive_glb_cpu(|opt| {
                let cpu = opt?;
                let m = archive_glb::archive_marker_world_mat4(
                    h,
                    env_scale,
                    cpu,
                    archive_glb::ARCHIVE_SPAWN_FOCUSED_ITEM,
                )?;
                let p = m.transform_point3(Vec3::ZERO);
                let a = surface_anchor_from_world_xyz(w, h, p);
                Some((a[0], a[1], a[2]))
            })
            .unwrap_or((w * 0.5, h * 0.5, 0.0));
            let closeup_anchor = crate::ui::placement::PlacementAnchor::new(
                [closeup_ax, closeup_ay, closeup_az],
                rot_fixed_axes_deg(90.0, 0.0, 0.0),
                &self.positions.pedestal,
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
                        },
                        hover_target: 1.0,
                        anim_id: closeup_anim,
                    }));
                }
                ArtifactKind::Talisman(tk) => {
                    // Grid cubbies share [`archive_talisman_rotation`]; Ry tilt
                    // keeps the holo sheen readable on the featured pedestal.
                    hud_plaques.push(with_inspect_spin(Object3d {
                        pos: closeup_anchor.pos,
                        extents: [
                            closeup_size * ARCHIVE_TALISMAN_EXTENTS[0],
                            closeup_size * ARCHIVE_TALISMAN_EXTENTS[1],
                            closeup_size * ARCHIVE_TALISMAN_EXTENTS[2],
                        ],
                        rotation: archive_talisman_rotation(),
                        color: closeup_bright,
                        kind: Object3dKind::Talisman { kind: *tk },
                        hover_target: 1.0,
                        anim_id: closeup_anim,
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
                            anim_id: closeup_anim,
                            placement_rot_deg: [0.0, 0.0, 0.0],
                        },
                    )));
                }
                ArtifactKind::Ordeal(kind) => {
                    // BossIcon mesh caps face local +Y (same as Relic). Pedestal
                    // placement uses Rx(90°) for vertical plaques — don't reuse that
                    // rotation or the icon lies edge-on. Billboard toward the active camera.
                    let closeup_rotation = collection_chrome_tablet_rotation(
                        w,
                        h,
                        &final_cam,
                        closeup_anchor.pos[0],
                        closeup_anchor.pos[1],
                        closeup_anchor.pos[2],
                    );
                    hud_plaques.push(with_inspect_spin(Object3d {
                        pos: closeup_anchor.pos,
                        extents: [closeup_size, closeup_size * 0.04, closeup_size],
                        rotation: closeup_rotation,
                        color: [1.0, 1.0, 1.0, 1.0],
                        kind: Object3dKind::BossIcon {
                            kind: *kind,
                            glow: 0.0,
                            pick_id: None,
                        },
                        hover_target: 1.0,
                        anim_id: closeup_anim,
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
                        anim_id: closeup_anim,
                    }));
                }
            }

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
            let sign_text = if inspect.is_some()
                && boss.unlocked
                && let ArtifactKind::Relic(rid) = &boss.kind
                && let Some(def) = all_relic_defs().iter().find(|d| d.id == *rid)
                && !def.flavor.is_empty()
            {
                def.flavor.iter().fold(String::new(), |acc, s| acc + s.text)
            } else {
                card_text
            };
            frame.archive_sign_description_decal_text = Some(sign_text);
        }

        // Assemble the frame. 2D chrome from the caller (`quads` / `text_labels`)
        // is merged here with the grid and focus plaques.
        frame.quads(quads);
        frame.object3d_batch(plaques);
        if chronicle_dashboard {
            let panel = crate::ui::chronicle_dashboard::chronicle_panel_rect(w, h);
            let dim = crate::ui::chronicle_dashboard::chronicle_dim_gradient(panel);
            let mut chart_quads: Vec<GpuInstance> = Vec::new();
            let mut chart_squircle_quads: Vec<GpuInstance> = Vec::new();
            let mut chart_images: Vec<crate::render::draw_cmd::ImageQuad> = Vec::new();
            crate::ui::chronicle_dashboard::push_chronicle_dashboard(
                w,
                h,
                panel,
                crate::ui::chronicle_dashboard::ChronicleView {
                    focused_run: self.focused_row,
                    focused_pane: self.chronicle_focused_pane,
                    run_log_scroll: self.chronicle_run_log_scroll.get(),
                    career_scroll: self.chronicle_dashboard_scroll.get(),
                },
                ctx.progress,
                ctx.archive_chronicle_last_seen_run_len,
                &mut chart_quads,
                &mut chart_squircle_quads,
                text_labels,
                &mut chart_images,
            );
            frame.gradient_quads(std::iter::once(dim));
            frame.quads(chart_quads);
            if !chart_squircle_quads.is_empty() {
                frame.squircle_quads(chart_squircle_quads);
            }
            if !chart_images.is_empty() {
                frame.image_quads(chart_images);
            }
        }
        if !hud_plaques.is_empty() {
            frame.object3d_batch(hud_plaques);
        }
        frame.texts(std::mem::take(text_labels));

        // Hit rects for 2D chrome — skipped while [`ItemInspectScene`] owns input.
        if inspect.is_none() {
            let items = self.flat_items(
                w,
                h,
                ctx.progress,
                env_scale,
                ctx.archive_chronicle_last_seen_run_len,
            );
            self.tree.register_flat_buttons(&items, &mut frame.buttons);
        }

        frame.window_title = {
            let tab_new = archive_seen::archive_new_counts(
                ctx.progress,
                ctx.archive_chronicle_last_seen_run_len,
            )
            .for_tab(tab_to_archive_tab(self.active_tab));
            if tab_new > 0 {
                format!(
                    "Mahjuro — Archive ({} · {} new)",
                    self.active_tab.label(),
                    tab_new
                )
            } else {
                format!("Mahjuro — Archive ({})", self.active_tab.label())
            }
        };
        if inspect.is_some()
            && let Some(tw) =
                self.collection_inspect_target_world(w, h, bosses, ctx.layout, env_scale)
        {
            frame.shop_inspect_shadow_target = Some(tw);
        }
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

        let frame = UiFrame::new();

        let mut quads: Vec<GpuInstance> = Vec::new();
        let mut text_labels: Vec<TextLabel> = Vec::new();

        let env_h_draw = collection_sanitized_room_gltf_height_scale(ctx.room_gltf_height_scale);
        let cam = archive_glb::archive_camera_base(w, h, env_h_draw);
        let back_h = ch.chrome_btn_h;
        let ring_focus = self.chrome_focus_for_draw(ctx.input_mode);
        let page_nav = archive_page_nav(self.active_tab, ctx.progress, self.archive_page);

        if let Some(back_rect) = archive_main_menu_btn_rect(w, h, &cam, env_h_draw)
            && ring_focus == Some(CollectionAction::Back)
        {
            push_focus_ring(back_rect, scale, w, h, &mut quads);
        }
        if let Some(switch_rect) = archive_switch_save_btn_rect(w, h, &cam, env_h_draw)
            && ring_focus == Some(CollectionAction::SwitchSave)
        {
            push_focus_ring(switch_rect, scale, w, h, &mut quads);
        }
        for (ti, rect) in archive_tab_hit_rects(w, h, env_h_draw, &cam) {
            if ring_focus == Some(CollectionAction::SelectTab(ti)) {
                push_focus_ring(rect, scale, w, h, &mut quads);
            }
            if ti < TABS.len() {
                let tab_new = archive_seen::archive_new_counts(
                    ctx.progress,
                    ctx.archive_chronicle_last_seen_run_len,
                )
                .for_tab(tab_to_archive_tab(TABS[ti]));
                if tab_new > 0 {
                    crate::ui::corner_badge::push_corner_badge(
                        &mut quads,
                        &mut text_labels,
                        rect,
                        h,
                        "NEW",
                    );
                }
            }
        }
        if collection_uses_footer_arrows(self.active_tab) {
            if page_nav.show_prev
                && let Some(rect) = archive_page_left_btn_rect(w, h, &cam, env_h_draw)
                && ring_focus == Some(CollectionAction::PrevPage)
            {
                push_focus_ring(rect, scale, w, h, &mut quads);
            }
            if page_nav.show_next
                && let Some(rect) = archive_page_right_btn_rect(w, h, &cam, env_h_draw)
                && ring_focus == Some(CollectionAction::NextPage)
            {
                push_focus_ring(rect, scale, w, h, &mut quads);
            }
        }

        let footer_anchor_y = h - back_h - h * 0.02;
        let chronicle_ledger = matches!(self.active_tab, Tab::Chronicle) && inspect.is_none();

        // Control hints — pinned above the footer chrome. The page / scroll
        // affordance line is omitted when the tab fits in one page so the
        // hint stays compact on small catalogs.
        let chronicle_last_seen = ctx.archive_chronicle_last_seen_run_len;
        let all_count_hint =
            tab_artifacts(self.active_tab, ctx.progress, chronicle_last_seen).len();
        let archive_page_count_now = archive_page_count(all_count_hint);
        let archive_multi_page = archive_page_count_now > 1;
        let hint_style = HintStyle::archive_footer(h);
        let hint_line_h = hint_style.line_h;
        let hint_band_x = margin_x * 0.5;
        let hint_band_w = w - margin_x;
        let controller = matches!(ctx.input_mode, InputMode::Controller);
        let inspect_open = inspect.is_some();

        let mut legend_row_rects: Vec<[f32; 4]> = Vec::new();
        let mut legend_rows: Vec<Vec<HintSegment>> = Vec::new();
        let mut hint_line_count = 1usize;

        if inspect_open {
            legend_rows.push(inspect_camera_hint_row(ctx.input_mode));
            legend_rows.push(inspect_dismiss_hint_row(ctx.input_mode));
            hint_line_count = 2;
        } else if matches!(self.active_tab, Tab::Chronicle) && all_count_hint == 0 {
            legend_rows.push(vec![HintSegment::text(
                "Finish a non-tutorial run to add folios here.",
            )]);
        } else if matches!(self.active_tab, Tab::Chronicle) && all_count_hint > 0 {
            let mut chronicle_row = HintRow::new();
            if !controller {
                chronicle_row = chronicle_row.text("Wheel or ");
            }
            legend_rows.push(
                chronicle_row
                    .bind("scroll", vec![HintKey::dpad_vertical()])
                    .sep()
                    .bind("pane", vec![HintKey::dpad_horizontal()])
                    .into_segments(),
            );
        } else {
            let inspect_keys = if controller {
                vec![
                    HintKey::Action(UiAction::Confirm),
                    HintKey::Action(UiAction::NorthFacePress),
                ]
            } else {
                vec![
                    HintKey::Keyboard("keyboard_enter"),
                    HintKey::Keyboard("keyboard_e"),
                ]
            };
            if archive_multi_page {
                legend_rows.push(
                    HintRow::new()
                        .bind("inspect", inspect_keys)
                        .sep()
                        .bind("page", vec![HintKey::dpad_horizontal()])
                        .into_segments(),
                );
            } else {
                legend_rows.push(HintRow::new().bind("inspect", inspect_keys).into_segments());
            }
        }

        let hint_h = hint_line_h * hint_line_count as f32 + 10.0;
        let hint_y = if chronicle_ledger {
            h - hint_h - (h * 0.018).max(10.0)
        } else {
            footer_anchor_y - hint_h - (h * 0.014).max(10.0)
        };

        if inspect_open {
            legend_row_rects.push([hint_band_x, hint_y, hint_band_w, hint_line_h]);
            legend_row_rects.push([hint_band_x, hint_y + hint_line_h, hint_band_w, hint_line_h]);
        } else {
            legend_row_rects.push([hint_band_x, hint_y, hint_band_w, hint_line_h]);
        }

        // Page indicator — `Page X / Y · ● ○ ○ …` centred in the gap between
        // the authored cabinet page buttons when the catalog spans multiple pages.
        if archive_multi_page && inspect.is_none() && !chronicle_ledger {
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
            let page_font = typography::size(typography::H36, h);
            if let Some(rect) = archive_page_indicator_rect(w, h, &cam, env_h_draw, page_font) {
                text_labels.push(TextLabel {
                    rect,
                    text: label,
                    color: [0.95, 0.86, 0.56, 0.95],
                    font_px: Some(page_font),
                    align: TextAlign::Center,
                    ..Default::default()
                });
            }
        }

        let all_artifacts = tab_artifacts(self.active_tab, ctx.progress, chronicle_last_seen);

        // Grid layout, focus close-up, and description plaque.
        let mut frame = self.build_archive_grid_frame(
            frame,
            quads,
            &mut text_labels,
            &all_artifacts,
            &ctx,
            inspect,
        );
        if !legend_rows.is_empty() {
            push_inline_hint_rows(
                &mut frame,
                &ctx,
                &legend_row_rects,
                &legend_rows,
                hint_style,
            );
        }
        frame
    }
}

pub(crate) fn sync_item_inspect_orbit_target(
    scene: &CollectionScene,
    w: f32,
    h: f32,
    layout: &crate::ui::layout::LayoutResult,
    progress: &crate::core::progression::PlayerProgress,
    room_gltf_height_scale: f32,
    orbit: &mut ItemInspectOrbitState,
) {
    let all_artifacts = tab_artifacts(scene.active_tab, progress, 0);
    if let Some(target_world) =
        scene.collection_inspect_target_world(w, h, &all_artifacts, layout, room_gltf_height_scale)
    {
        orbit.target_world = target_world;
    }
}

impl SceneBehavior for CollectionScene {
    fn face_button_bindings(
        &self,
        _ctx: crate::ui::input::FaceBindingCtx,
    ) -> crate::ui::input::FaceButtonBindings {
        crate::ui::input::FaceButtonBindings {
            north_press: Some(crate::ui::input::UiAction::NorthFacePress),
            ..Default::default()
        }
    }

    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if self.chronicle_last_seen.is_none() {
            self.chronicle_last_seen = Some(ctx.archive_chronicle_last_seen);
            if archive_seen::archive_seen_needs_migration_seed(ctx.progress) {
                *ctx.seed_archive_seen = true;
            }
        }
        let chronicle_last_seen = self
            .chronicle_last_seen
            .unwrap_or(ctx.archive_chronicle_last_seen);

        let items = self.flat_items(
            ctx.layout.window_w,
            ctx.layout.window_h,
            ctx.progress,
            self.drawn_room_gltf_height_scale.get(),
            chronicle_last_seen,
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
        let focus_changed = self.tree.take_focus_changed();
        if focus_changed {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }

        // Keyboard / controller / wheel navigation:
        //   - Triggers / Tab / Shift+Tab → cycle tabs
        //   - Cabinet: ←/↓ forward along the catalogue; →/↑ backward (wrap); wheel / PgUp/Dn flip pages
        //   - Confirm / North → orbit inspect on the focused artifact
        let all_count = tab_artifacts(self.active_tab, ctx.progress, chronicle_last_seen).len();
        if focus_changed && self.focused_chrome.is_none() {
            collection_sync_hover_artifact_focus(self, &items, all_count);
        }

        if ctx.scroll_lines.abs() > 0.001 {
            if matches!(self.active_tab, Tab::Chronicle) {
                let w = ctx.layout.window_w;
                let h = ctx.layout.window_h;
                let panel = chronicle_panel_rect(w, h);
                let pane = chronicle_resolve_scroll_pane(
                    self,
                    w,
                    h,
                    panel,
                    ctx.cursor_pos,
                    ctx.input_mode,
                );
                self.chronicle_focused_pane = pane;
                chronicle_apply_scroll_delta(
                    self,
                    w,
                    h,
                    panel,
                    ctx.progress,
                    pane,
                    ctx.scroll_lines * CHRONICLE_SCROLL_STEP_PX,
                );
            } else {
                let dir: i32 = if ctx.scroll_lines > 0.0 { 1 } else { -1 };
                let from = archive_focus_row_col_in_page(self.focused_row);
                archive_page_step(
                    self,
                    ctx.bus,
                    dir,
                    from,
                    all_count,
                    ctx.progress,
                    chronicle_last_seen,
                    None,
                );
            }
        }

        let apply_artifact_focus = |scene: &mut CollectionScene,
                                    bus: &mut crate::game::event_bus::EventBus,
                                    idx: usize| {
            bus.push(GameEvent::UiSound(SfxId::TilePlace));
            collection_focus_artifact(scene, idx, ctx.progress, chronicle_last_seen, bus);
        };

        for a in ctx.actions {
            match a {
                UiAction::Cancel | UiAction::Pause | UiAction::CommitDiscard => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    maybe_bump_chronicle_on_exit(
                        self,
                        ctx.progress,
                        ctx.bump_archive_chronicle_seen,
                    );
                    return Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()));
                }
                UiAction::TabNext => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    self.cycle_tab(true, ctx.progress, chronicle_last_seen, ctx.bus);
                }
                UiAction::TabPrev => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    self.cycle_tab(false, ctx.progress, chronicle_last_seen, ctx.bus);
                }
                UiAction::PageNext => {
                    if matches!(self.active_tab, Tab::Chronicle) {
                        let w = ctx.layout.window_w;
                        let h = ctx.layout.window_h;
                        let panel = chronicle_panel_rect(w, h);
                        let pane = chronicle_resolve_scroll_pane(
                            self,
                            w,
                            h,
                            panel,
                            ctx.cursor_pos,
                            ctx.input_mode,
                        );
                        self.chronicle_focused_pane = pane;
                        chronicle_apply_scroll_delta(
                            self,
                            w,
                            h,
                            panel,
                            ctx.progress,
                            pane,
                            h * 0.22,
                        );
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    } else {
                        let from = archive_focus_row_col_in_page(self.focused_row);
                        archive_page_step(
                            self,
                            ctx.bus,
                            1,
                            from,
                            all_count,
                            ctx.progress,
                            chronicle_last_seen,
                            None,
                        );
                    }
                }
                UiAction::PagePrev => {
                    if matches!(self.active_tab, Tab::Chronicle) {
                        let w = ctx.layout.window_w;
                        let h = ctx.layout.window_h;
                        let panel = chronicle_panel_rect(w, h);
                        let pane = chronicle_resolve_scroll_pane(
                            self,
                            w,
                            h,
                            panel,
                            ctx.cursor_pos,
                            ctx.input_mode,
                        );
                        self.chronicle_focused_pane = pane;
                        chronicle_apply_scroll_delta(
                            self,
                            w,
                            h,
                            panel,
                            ctx.progress,
                            pane,
                            -h * 0.22,
                        );
                        ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    } else {
                        let from = archive_focus_row_col_in_page(self.focused_row);
                        archive_page_step(
                            self,
                            ctx.bus,
                            -1,
                            from,
                            all_count,
                            ctx.progress,
                            chronicle_last_seen,
                            None,
                        );
                    }
                }
                UiAction::FocusNext => {
                    if self.focused_chrome.is_some() {
                        collection_chrome_directional(self, ctx.bus, &items, FocusDir::Right);
                        continue;
                    }
                    if matches!(self.active_tab, Tab::Chronicle) && all_count > 0 {
                        ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                        let pane = match self.chronicle_focused_pane {
                            crate::ui::chronicle_dashboard::ChronicleScrollPane::RunLog => {
                                crate::ui::chronicle_dashboard::ChronicleScrollPane::Career
                            }
                            crate::ui::chronicle_dashboard::ChronicleScrollPane::Career => {
                                crate::ui::chronicle_dashboard::ChronicleScrollPane::RunLog
                            }
                        };
                        chronicle_set_focused_pane(self, pane, all_count);
                        continue;
                    }
                    if all_count == 0 {
                        continue;
                    }
                    archive_directional_step(
                        self,
                        ctx.bus,
                        &items,
                        FocusDir::Right,
                        all_count,
                        ctx.progress,
                        chronicle_last_seen,
                    );
                }
                UiAction::FocusPrev => {
                    if self.focused_chrome.is_some() {
                        collection_chrome_directional(self, ctx.bus, &items, FocusDir::Left);
                        continue;
                    }
                    if matches!(self.active_tab, Tab::Chronicle) && all_count > 0 {
                        ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                        let pane = match self.chronicle_focused_pane {
                            crate::ui::chronicle_dashboard::ChronicleScrollPane::RunLog => {
                                crate::ui::chronicle_dashboard::ChronicleScrollPane::Career
                            }
                            crate::ui::chronicle_dashboard::ChronicleScrollPane::Career => {
                                crate::ui::chronicle_dashboard::ChronicleScrollPane::RunLog
                            }
                        };
                        chronicle_set_focused_pane(self, pane, all_count);
                        continue;
                    }
                    if all_count == 0 {
                        continue;
                    }
                    archive_directional_step(
                        self,
                        ctx.bus,
                        &items,
                        FocusDir::Left,
                        all_count,
                        ctx.progress,
                        chronicle_last_seen,
                    );
                }
                UiAction::FocusUp => {
                    if self.focused_chrome.is_some() {
                        collection_chrome_directional(self, ctx.bus, &items, FocusDir::Up);
                        continue;
                    }
                    if matches!(self.active_tab, Tab::Chronicle) && all_count > 0 {
                        if self.chronicle_focused_pane
                            == crate::ui::chronicle_dashboard::ChronicleScrollPane::Career
                        {
                            if self.chronicle_dashboard_scroll.get() <= 0.001 {
                                chronicle_set_focused_pane(
                                    self,
                                    crate::ui::chronicle_dashboard::ChronicleScrollPane::RunLog,
                                    all_count.max(1),
                                );
                                ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                            } else {
                                let w = ctx.layout.window_w;
                                let h = ctx.layout.window_h;
                                let panel = chronicle_panel_rect(w, h);
                                chronicle_apply_scroll_delta(
                                    self,
                                    w,
                                    h,
                                    panel,
                                    ctx.progress,
                                    crate::ui::chronicle_dashboard::ChronicleScrollPane::Career,
                                    -CHRONICLE_SCROLL_STEP_PX,
                                );
                                ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                            }
                        } else {
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
                        }
                        continue;
                    }
                    if all_count == 0
                        || !archive_directional_step(
                            self,
                            ctx.bus,
                            &items,
                            FocusDir::Up,
                            all_count,
                            ctx.progress,
                            chronicle_last_seen,
                        )
                    {
                        collection_enter_chrome(self, ctx.bus, &items, FocusDir::Up);
                    }
                }
                UiAction::FocusDown => {
                    if self.focused_chrome.is_some() {
                        collection_chrome_directional(self, ctx.bus, &items, FocusDir::Down);
                        continue;
                    }
                    if matches!(self.active_tab, Tab::Chronicle) && all_count > 0 {
                        if self.chronicle_focused_pane
                            == crate::ui::chronicle_dashboard::ChronicleScrollPane::Career
                        {
                            let w = ctx.layout.window_w;
                            let h = ctx.layout.window_h;
                            let panel = chronicle_panel_rect(w, h);
                            chronicle_apply_scroll_delta(
                                self,
                                w,
                                h,
                                panel,
                                ctx.progress,
                                crate::ui::chronicle_dashboard::ChronicleScrollPane::Career,
                                CHRONICLE_SCROLL_STEP_PX,
                            );
                            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                        } else {
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
                                chronicle_set_focused_pane(
                                    self,
                                    crate::ui::chronicle_dashboard::ChronicleScrollPane::Career,
                                    all_count.max(1),
                                );
                                ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                            }
                        }
                        continue;
                    }
                    if all_count == 0
                        || !archive_directional_step(
                            self,
                            ctx.bus,
                            &items,
                            FocusDir::Down,
                            all_count,
                            ctx.progress,
                            chronicle_last_seen,
                        )
                    {
                        collection_enter_chrome(self, ctx.bus, &items, FocusDir::Down);
                    }
                }
                UiAction::Confirm => {
                    if let Some(chrome) = self.focused_chrome {
                        match chrome {
                            CollectionAction::Back => {
                                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                                maybe_bump_chronicle_on_exit(
                                    self,
                                    ctx.progress,
                                    ctx.bump_archive_chronicle_seen,
                                );
                                return Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()));
                            }
                            CollectionAction::SwitchSave => {
                                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                                return Some(Scene::ProfileSelect(
                                    ProfileSelectScene::from_archive_switch_save(),
                                ));
                            }
                            CollectionAction::PrevPage => {
                                let from = archive_focus_row_col_in_page(self.focused_row);
                                archive_page_step(
                                    self,
                                    ctx.bus,
                                    -1,
                                    from,
                                    all_count,
                                    ctx.progress,
                                    chronicle_last_seen,
                                    Some(CollectionAction::PrevPage),
                                );
                            }
                            CollectionAction::NextPage => {
                                let from = archive_focus_row_col_in_page(self.focused_row);
                                archive_page_step(
                                    self,
                                    ctx.bus,
                                    1,
                                    from,
                                    all_count,
                                    ctx.progress,
                                    chronicle_last_seen,
                                    None,
                                );
                            }
                            CollectionAction::SelectTab(i) => {
                                if i < TABS.len() {
                                    ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                                    enter_tab(
                                        self,
                                        TABS[i],
                                        ctx.progress,
                                        chronicle_last_seen,
                                        ctx.bus,
                                    );
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    if !matches!(self.active_tab, Tab::Chronicle) {
                        let w = ctx.layout.window_w;
                        let h = ctx.layout.window_h;
                        let bosses =
                            tab_artifacts(self.active_tab, ctx.progress, chronicle_last_seen);
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
                    }
                }
                UiAction::NorthFacePress => {
                    let w = ctx.layout.window_w;
                    let h = ctx.layout.window_h;
                    let bosses = tab_artifacts(self.active_tab, ctx.progress, chronicle_last_seen);
                    if bosses.is_empty() {
                        continue;
                    }
                    if matches!(self.active_tab, Tab::Chronicle) {
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
                maybe_bump_chronicle_on_exit(self, ctx.progress, ctx.bump_archive_chronicle_seen);
                return Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()));
            }
            Some(CollectionAction::SwitchSave) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                return Some(Scene::ProfileSelect(
                    ProfileSelectScene::from_archive_switch_save(),
                ));
            }
            Some(CollectionAction::PrevPage) => {
                let from = archive_focus_row_col_in_page(self.focused_row);
                archive_page_step(
                    self,
                    ctx.bus,
                    -1,
                    from,
                    all_count,
                    ctx.progress,
                    chronicle_last_seen,
                    Some(CollectionAction::PrevPage),
                );
            }
            Some(CollectionAction::NextPage) => {
                let from = archive_focus_row_col_in_page(self.focused_row);
                archive_page_step(
                    self,
                    ctx.bus,
                    1,
                    from,
                    all_count,
                    ctx.progress,
                    chronicle_last_seen,
                    Some(CollectionAction::NextPage),
                );
            }
            Some(CollectionAction::SelectTab(i)) => {
                if i < TABS.len() {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    enter_tab(self, TABS[i], ctx.progress, chronicle_last_seen, ctx.bus);
                }
            }
            Some(CollectionAction::ChronicleFocusCareer) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                chronicle_set_focused_pane(
                    self,
                    crate::ui::chronicle_dashboard::ChronicleScrollPane::Career,
                    all_count.max(1),
                );
            }
            Some(CollectionAction::SelectArtifact(idx)) => {
                if all_count > 0 {
                    let idx = idx.min(all_count.saturating_sub(1));
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
                    self.selected_artifact = Some(idx);
                    self.focused_chrome = None;
                    push_relic_stinger_for(ctx.bus, self.active_tab, ctx.progress, idx);
                    collection_focus_artifact(
                        self,
                        idx,
                        ctx.progress,
                        chronicle_last_seen,
                        ctx.bus,
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
            let panel = chronicle_panel_rect(w, h);
            chronicle_update_pane_from_cursor(self, w, h, panel, ctx.cursor_pos, ctx.input_mode);
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
            self.chronicle_run_log_scroll
                .set(self.chronicle_run_log_scroll.get().clamp(0.0, run_max));
        }

        let n = tab_artifacts(self.active_tab, ctx.progress, chronicle_last_seen).len();
        let pc = archive_page_count(n);
        self.archive_page = self.archive_page.min(pc.saturating_sub(1));
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
    let arts = tab_artifacts(tab, progress, 0);
    if let Some(art) = arts.get(idx)
        && art.unlocked
        && let ArtifactKind::Relic(rid) = art.kind
    {
        bus.push(GameEvent::PlayRelicStinger(rid));
    }
}

fn tab_artifacts(
    tab: Tab,
    progress: &crate::core::progression::PlayerProgress,
    chronicle_last_seen: u32,
) -> Vec<Artifact> {
    let mut out = match tab {
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
                    is_new: false,
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
                is_new: false,
            })
            .collect(),
        Tab::Ordeals => all_ordeals()
            .iter()
            .chain(final_ordeals().iter())
            .filter(|def| progress.ordeal_times_encountered.contains_key(&def.kind))
            .map(|def| Artifact {
                name: def.name.to_string(),
                unlocked: true,
                kind: ArtifactKind::Ordeal(def.kind),
                accent: def.tier.halo_color(),
                is_new: false,
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
                is_new: false,
            })
            .collect(),
        Tab::Chronicle => {
            let indices = archive_career::chronicle_indices_recent_first(progress);
            let mut chronicle = Vec::with_capacity(indices.len() + 1);
            chronicle.push(Artifact {
                name: "Summary".into(),
                unlocked: true,
                kind: ArtifactKind::ChronicleSummary,
                accent: color::CHAMPAGNE,
                is_new: false,
            });
            for (list_i, &idx) in indices.iter().enumerate() {
                let Some(rec) = progress.run_history.get(idx) else {
                    continue;
                };
                let display =
                    archive_career::chronicle_display_run_number(list_i + 1, progress).unwrap_or(0);
                chronicle.push(Artifact {
                    name: archive_career::chronicle_run_log_title(progress, display, rec),
                    unlocked: true,
                    kind: ArtifactKind::ChronicleRun(idx),
                    accent: color::PARCHMENT,
                    is_new: false,
                });
            }
            chronicle
        }
    };
    for art in &mut out {
        art.is_new = artifact_is_new(art, progress, chronicle_last_seen);
    }
    out
}

fn tab_to_archive_tab(tab: Tab) -> ArchiveTab {
    match tab {
        Tab::Relics => ArchiveTab::Relics,
        Tab::Talismans => ArchiveTab::Talismans,
        Tab::Yaku => ArchiveTab::Yaku,
        Tab::Ordeals => ArchiveTab::Ordeals,
        Tab::Chronicle => ArchiveTab::Chronicle,
    }
}

fn artifact_is_new(
    art: &Artifact,
    progress: &crate::core::progression::PlayerProgress,
    chronicle_last_seen: u32,
) -> bool {
    match &art.kind {
        ArtifactKind::Relic(id) => !progress.archive_seen_relics.contains(id),
        ArtifactKind::Zodiac(zk) => !progress.archive_seen_yaku.contains(&zk.yaku()),
        ArtifactKind::Ordeal(bk) => !progress.archive_seen_ordeals.contains(bk),
        ArtifactKind::Talisman(tk) => !progress.archive_seen_talismans.contains(tk),
        ArtifactKind::ChronicleRun(idx) => {
            archive_seen::chronicle_run_is_new(*idx, chronicle_last_seen)
        }
        ArtifactKind::ChronicleSummary => false,
    }
}

fn artifact_seen_mark(art: &Artifact) -> Option<ArchiveSeenMark> {
    match art.kind {
        ArtifactKind::Relic(id) => Some(ArchiveSeenMark::Relic(id)),
        ArtifactKind::Zodiac(zk) => Some(ArchiveSeenMark::Yaku(zk.yaku())),
        ArtifactKind::Ordeal(bk) => Some(ArchiveSeenMark::Ordeal(bk)),
        ArtifactKind::Talisman(tk) => Some(ArchiveSeenMark::Talisman(tk)),
        ArtifactKind::ChronicleRun(_) | ArtifactKind::ChronicleSummary => None,
    }
}

fn mark_artifact_seen_if_new(
    tab: Tab,
    progress: &crate::core::progression::PlayerProgress,
    idx: usize,
    chronicle_last_seen: u32,
    bus: &mut crate::game::event_bus::EventBus,
) {
    let arts = tab_artifacts(tab, progress, chronicle_last_seen);
    let Some(art) = arts.get(idx) else {
        return;
    };
    if !art.is_new {
        return;
    }
    if let Some(mark) = artifact_seen_mark(art) {
        bus.push(GameEvent::ArchiveItemSeen(mark));
    }
}

fn enter_tab(
    scene: &mut CollectionScene,
    tab: Tab,
    progress: &crate::core::progression::PlayerProgress,
    chronicle_last_seen: u32,
    _bus: &mut crate::game::event_bus::EventBus,
) {
    scene.active_tab = tab;
    scene.selected_artifact = None;
    scene.focused_chrome = None;
    scene.archive_page = 0;
    scene.chronicle_dashboard_scroll.set(0.0);
    scene.chronicle_run_log_scroll.set(0.0);
    scene.chronicle_focused_pane = crate::ui::chronicle_dashboard::ChronicleScrollPane::RunLog;
    if matches!(tab, Tab::Chronicle) {
        scene.visited_chronicle = true;
    }
    let artifacts = tab_artifacts(tab, progress, chronicle_last_seen);
    if let Some(idx) = artifacts.iter().position(|a| a.is_new) {
        collection_sync_artifact_focus_to_idx(scene, idx);
    } else if artifacts.is_empty() {
        scene.focused_row = None;
    } else {
        scene.focused_row = Some(0);
        scene
            .tree
            .set_focus(CollectionAction::SelectArtifact(0).id());
    }
}

fn maybe_bump_chronicle_on_exit(
    scene: &CollectionScene,
    progress: &crate::core::progression::PlayerProgress,
    bump: &mut Option<u32>,
) {
    if scene.visited_chronicle {
        *bump = Some(progress.run_history.len() as u32);
    }
}

fn collection_focus_artifact(
    scene: &mut CollectionScene,
    idx: usize,
    progress: &crate::core::progression::PlayerProgress,
    chronicle_last_seen: u32,
    bus: &mut crate::game::event_bus::EventBus,
) {
    collection_sync_artifact_focus_to_idx(scene, idx);
    mark_artifact_seen_if_new(scene.active_tab, progress, idx, chronicle_last_seen, bus);
}

/// Map a yaku to its matching zodiac ribbon. Every yaku has exactly one
/// ribbon that levels it up, so this is total. Inverse of
/// `ZodiacKind::yaku`.
fn zodiac_for_yaku(yk: YakuKind) -> ZodiacKind {
    ZodiacKind::for_yaku(yk).expect("every scoring YakuKind has a matching ZodiacKind")
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
            None,
        ),
        ArtifactKind::Talisman(kind) => kind.description().to_string(),
        ArtifactKind::Zodiac(kind) => format!(
            "Levelled by the {} zodiac ribbon (+0.5 mult, +20 chips per level).",
            kind.name()
        ),
        ArtifactKind::Ordeal(kind) => kind.def().description.to_string(),
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

// ── Camera projection helpers ───────────────────────────────────────

/// Mirror of `crate::render::world_space::pixel_to_world` for the
/// scene's local use — keeps the scene independent of the renderer
/// module while ensuring positions agree with what the renderer emits.
fn pixel_to_world_xy(w: f32, h: f32, px: f32, py: f32, lift: f32) -> glam::Vec3 {
    glam::Vec3::new(px - w * 0.5, h * 0.5 - py, lift)
}

/// Orients a camera-facing plaque's local **+Y** face toward the camera. Archive
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

fn push_archive_cubby_new_badges(
    bosses: &[Artifact],
    page: usize,
    focus_flat: i32,
    anchors: &[Option<[f32; 3]>],
    w: f32,
    h: f32,
    cam: &CameraParams,
    cell: f32,
    quads: &mut Vec<GpuInstance>,
    texts: &mut Vec<TextLabel>,
) {
    let page_size = archive_page_size();
    let cell_gap = cell * 0.22;
    let cell_pitch = cell + cell_gap;
    let rect_w = cell_pitch * 0.95;
    let rect_h = cell_pitch * 0.95;
    let vp = camera_view_proj(w, h, cam);
    for (slot, anchor) in anchors.iter().enumerate().take(page_size) {
        let Some(anchor) = anchor else {
            continue;
        };
        let global_idx = page * page_size + slot;
        if global_idx >= bosses.len() {
            continue;
        }
        let boss = &bosses[global_idx];
        if !boss.is_new || global_idx == focus_flat as usize {
            continue;
        }
        let world = pixel_to_world_xy(w, h, anchor[0], anchor[1], anchor[2]);
        let (sx, sy) = world_to_screen(vp, w, h, world);
        if !screen_hit_anchor_is_finite(sx, sy, rect_w, rect_h) {
            continue;
        }
        let rect = [sx - rect_w * 0.5, sy - rect_h * 0.5, rect_w, rect_h];
        crate::ui::corner_badge::push_center_badge(quads, texts, rect, h, "NEW");
    }
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
        let k = if is_focus {
            2.0
        } else if boss.is_new {
            1.95
        } else {
            1.6
        };
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
                },
                hover_target: if is_focus { 1.0 } else { 0.0 },
                anim_id: boss_i as u64,
            });
        }
        ArtifactKind::Talisman(tk) => {
            plaques.push(Object3d {
                pos: [cx, nameplate_py, cz],
                extents: [
                    plate_w * ARCHIVE_TALISMAN_EXTENTS[0],
                    plate_w * ARCHIVE_TALISMAN_EXTENTS[1],
                    plate_w * ARCHIVE_TALISMAN_EXTENTS[2],
                ],
                rotation: archive_talisman_rotation(),
                color: bright,
                kind: Object3dKind::Talisman { kind: *tk },
                hover_target: if is_focus { 1.0 } else { 0.0 },
                anim_id: boss_i as u64,
            });
        }
        ArtifactKind::Zodiac(zk) => {
            let zodiac_anchor = crate::ui::placement::PlacementAnchor::new(
                [cx, nameplate_py, cz],
                rot_fixed_axes_deg(90.0, 0.0, 0.0),
                cubby_zodiac,
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
                placement_rot_deg: [0.0, 0.0, 0.0],
            }));
        }
        ArtifactKind::Ordeal(kind) => {
            let f = fade.max(if is_focus { 1.0 } else { 0.55 });
            let lum = f.min(1.0);
            plaques.push(Object3d {
                pos: [cx, nameplate_py, cz],
                extents: [plate_w, plate_w * 0.04, plate_w],
                rotation: euler_xyz_rad_from_deg(180.0, 0.0, 0.0),
                color: [lum, lum, lum, 1.0],
                kind: Object3dKind::BossIcon {
                    kind: *kind,
                    glow: if is_focus { 0.5 } else { 0.0 },
                    pick_id: Some(boss_i as u32),
                },
                hover_target: if is_focus { 1.0 } else { 0.0 },
                anim_id: boss_i as u64,
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

/// Cabinet page navigation affordances for the authored `btn_page_*` meshes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ArchivePageNav {
    show_prev: bool,
    show_next: bool,
}

#[inline]
fn archive_page_nav(
    tab: Tab,
    progress: &crate::core::progression::PlayerProgress,
    archive_page: usize,
) -> ArchivePageNav {
    if !collection_uses_footer_arrows(tab) {
        return ArchivePageNav {
            show_prev: false,
            show_next: false,
        };
    }
    let all_count = tab_artifacts(tab, progress, 0).len();
    let page_count = archive_page_count(all_count);
    if page_count <= 1 {
        return ArchivePageNav {
            show_prev: false,
            show_next: false,
        };
    }
    let cur = archive_page.min(page_count.saturating_sub(1));
    ArchivePageNav {
        show_prev: cur > 0,
        show_next: cur + 1 < page_count,
    }
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

/// Keep [`CollectionScene::focused_row`] in sync when the widget tree's hover
/// target moves onto a catalog cell (same flat rects as every other tab).
fn collection_sync_hover_artifact_focus(
    scene: &mut CollectionScene,
    items: &[FlatItem<CollectionAction>],
    all_count: usize,
) {
    if all_count == 0 {
        return;
    }
    let Some(f) = scene.tree.focused() else {
        return;
    };
    let Some(it) = items.iter().find(|i| i.id == f) else {
        return;
    };
    let CollectionAction::SelectArtifact(idx) = it.action else {
        return;
    };
    let idx = idx.min(all_count - 1);
    if scene.focused_row != Some(idx) {
        collection_sync_artifact_focus_to_idx(scene, idx);
    }
}

fn collection_sync_artifact_focus_to_idx(scene: &mut CollectionScene, idx: usize) {
    let prev = scene.focused_row;
    scene.focused_row = Some(idx);
    if matches!(scene.active_tab, Tab::Chronicle) {
        if prev != Some(idx) {
            scene.chronicle_dashboard_scroll.set(0.0);
        }
        scene.chronicle_focused_pane = crate::ui::chronicle_dashboard::ChronicleScrollPane::RunLog;
    } else {
        scene.archive_page = archive_page_for_idx(idx);
    }
    scene
        .tree
        .set_focus(CollectionAction::SelectArtifact(idx).id());
}

/// (row, col) within the active archive page (0-indexed; cols < `ARCHIVE_SLOT_COLS`).
/// Returns (0, 0) when no item is focused so callers can land on slot 0.
fn archive_focus_row_col_in_page(focused: Option<usize>) -> (usize, usize) {
    let cols = archive_glb::ARCHIVE_SLOT_COLS.max(1);
    let page_size = archive_page_size().max(1);
    let slot = focused.map(|i| i % page_size).unwrap_or(0);
    (slot / cols, slot % cols)
}

/// When a footer page button triggered a page step, pick the button that should
/// keep focus after the page index changes.
#[inline]
fn collection_footer_page_button_after_step(
    nav: ArchivePageNav,
    via: CollectionAction,
) -> Option<CollectionAction> {
    match via {
        CollectionAction::PrevPage if nav.show_prev => Some(CollectionAction::PrevPage),
        CollectionAction::NextPage if nav.show_next => Some(CollectionAction::NextPage),
        CollectionAction::PrevPage if nav.show_next => Some(CollectionAction::NextPage),
        CollectionAction::NextPage if nav.show_prev => Some(CollectionAction::PrevPage),
        _ => None,
    }
}

/// Flip the archive cabinet by `dir` pages (positive = next, negative = prev) and land focus on
/// the slot corresponding to `target_in_page = (row, col)` within the new page. The candidate
/// slot is clamped to the last present artifact, so partial pages still land on a real item.
///
/// Caller chooses `target_in_page` based on gesture intent:
///   - **Directional edge-cross** (← / → walked off the column edge): preserve the row, swap the
///     column to the opposite edge so Right-then-Left is a no-op on full pages.
///   - **Bulk page-flip** (PgUp/PgDn, mouse wheel, page buttons): preserve both row and column
///     so the gesture is fully reversible.
///
/// No-ops when the catalogue is empty or already at the requested edge.
/// When a footer page button triggered the step, focus stays on that button if
/// it is still visible; otherwise it moves to the remaining footer button.
fn archive_page_step(
    scene: &mut CollectionScene,
    bus: &mut crate::game::event_bus::EventBus,
    dir: i32,
    target_in_page: (usize, usize),
    all_count: usize,
    progress: &crate::core::progression::PlayerProgress,
    chronicle_last_seen: u32,
    via_page_button: Option<CollectionAction>,
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
    collection_focus_artifact(scene, new_focus, progress, chronicle_last_seen, bus);
    bus.push(GameEvent::UiSound(SfxId::UiConfirm));

    let chrome_via = scene
        .focused_chrome
        .filter(|a| collection_chrome_is_bottom(*a))
        .or(via_page_button);
    if let Some(via) = chrome_via {
        let nav = archive_page_nav(scene.active_tab, progress, scene.archive_page);
        if let Some(target) = collection_footer_page_button_after_step(nav, via) {
            scene.focused_chrome = Some(target);
            scene.tree.set_focus(target.id());
        } else {
            scene.focused_chrome = None;
        }
    }
}

/// Spatial neighbour move within the current archive page; if the requested
/// neighbour falls off the page horizontally we flip to the adjacent page.
/// Vertical edges (no Up / Down neighbour) hand focus to the tab bar above or
/// the footer (Prev / Next) below so controller users can reach every button.
/// Returns whether the gesture consumed input — `false` means the caller
/// should consider chrome entry when there is no spatial neighbour.
fn archive_directional_step(
    scene: &mut CollectionScene,
    bus: &mut crate::game::event_bus::EventBus,
    items: &[FlatItem<CollectionAction>],
    dir: FocusDir,
    all_count: usize,
    progress: &crate::core::progression::PlayerProgress,
    chronicle_last_seen: u32,
) -> bool {
    if all_count == 0 {
        return false;
    }
    if let Some(ni) = collection_spatial_artifact_step(items, scene.focused_row, dir)
        && Some(ni) != scene.focused_row
    {
        bus.push(GameEvent::UiSound(SfxId::TilePlace));
        collection_focus_artifact(scene, ni, progress, chronicle_last_seen, bus);
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
            if let Some(from) = collection_focused_artifact_rect(items, scene.focused_row) {
                let chrome = collection_chrome_rects(items);
                if collection_focus_chrome_spatial(scene, bus, from, FocusDir::Right, &chrome) {
                    return true;
                }
            }
            archive_page_step(
                scene,
                bus,
                1,
                (from_row, 0),
                all_count,
                progress,
                chronicle_last_seen,
                None,
            );
            true
        }
        FocusDir::Left => {
            if let Some(from) = collection_focused_artifact_rect(items, scene.focused_row) {
                let chrome = collection_chrome_rects(items);
                if collection_focus_chrome_spatial(scene, bus, from, FocusDir::Left, &chrome) {
                    return true;
                }
            }
            archive_page_step(
                scene,
                bus,
                -1,
                (from_row, last_col),
                all_count,
                progress,
                chronicle_last_seen,
                None,
            );
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
            | CollectionAction::SelectTab(_) => Some((it.action, it.rect)),
            _ => None,
        })
        .collect()
}

fn push_chronicle_flat_items(
    items: &mut Vec<FlatItem<CollectionAction>>,
    w: f32,
    h: f32,
    progress: &crate::core::progression::PlayerProgress,
    run_log_scroll: f32,
) {
    let panel = chronicle_panel_rect(w, h);
    let entry_count = archive_career::chronicle_list_entry_count(progress);
    for (i, rect) in crate::ui::chronicle_dashboard::chronicle_run_log_hit_rects(
        w,
        h,
        panel,
        run_log_scroll,
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
    let (_, career_rect) = crate::ui::chronicle_dashboard::chronicle_pane_rects(w, h, panel);
    if flat_rect_xywh_is_finite(career_rect) {
        items.push(FlatItem::new(
            CollectionAction::ChronicleFocusCareer.id(),
            career_rect,
            CollectionAction::ChronicleFocusCareer,
        ));
    }
}

#[inline]
fn chronicle_panel_rect(w: f32, h: f32) -> [f32; 4] {
    crate::ui::chronicle_dashboard::chronicle_panel_rect(w, h)
}

const CHRONICLE_SCROLL_STEP_PX: f32 = 42.0;

#[inline]
fn collection_uses_footer_arrows(tab: Tab) -> bool {
    !matches!(tab, Tab::Chronicle)
}

#[inline]
fn chronicle_update_pane_from_cursor(
    scene: &mut CollectionScene,
    w: f32,
    h: f32,
    panel: [f32; 4],
    cursor: (f32, f32),
    input_mode: InputMode,
) {
    if input_mode != InputMode::Cursor {
        return;
    }
    if let Some(pane) =
        crate::ui::chronicle_dashboard::chronicle_scroll_pane_at(w, h, panel, cursor)
    {
        scene.chronicle_focused_pane = pane;
    }
}

#[inline]
fn chronicle_resolve_scroll_pane(
    scene: &CollectionScene,
    w: f32,
    h: f32,
    panel: [f32; 4],
    cursor: (f32, f32),
    input_mode: InputMode,
) -> crate::ui::chronicle_dashboard::ChronicleScrollPane {
    if input_mode == InputMode::Cursor {
        crate::ui::chronicle_dashboard::chronicle_scroll_pane_at(w, h, panel, cursor)
            .unwrap_or(scene.chronicle_focused_pane)
    } else {
        scene.chronicle_focused_pane
    }
}

#[inline]
fn chronicle_set_focused_pane(
    scene: &mut CollectionScene,
    pane: crate::ui::chronicle_dashboard::ChronicleScrollPane,
    entry_count: usize,
) {
    use crate::ui::chronicle_dashboard::ChronicleScrollPane;

    scene.chronicle_focused_pane = pane;
    scene.focused_chrome = None;
    match pane {
        ChronicleScrollPane::RunLog => {
            let idx = scene
                .focused_row
                .unwrap_or(0)
                .min(entry_count.saturating_sub(1));
            scene
                .tree
                .set_focus(CollectionAction::SelectArtifact(idx).id());
        }
        ChronicleScrollPane::Career => {
            scene
                .tree
                .set_focus(CollectionAction::ChronicleFocusCareer.id());
        }
    }
}

#[inline]
fn chronicle_apply_scroll_delta(
    scene: &mut CollectionScene,
    w: f32,
    h: f32,
    panel: [f32; 4],
    progress: &crate::core::progression::PlayerProgress,
    pane: crate::ui::chronicle_dashboard::ChronicleScrollPane,
    delta: f32,
) {
    use crate::ui::chronicle_dashboard::ChronicleScrollPane;

    match pane {
        ChronicleScrollPane::RunLog => {
            let entry_count = archive_career::chronicle_list_entry_count(progress);
            let max_s = crate::ui::chronicle_dashboard::chronicle_run_log_scroll_max(
                w,
                h,
                panel,
                entry_count,
            );
            let next = (scene.chronicle_run_log_scroll.get() + delta).clamp(0.0, max_s);
            scene.chronicle_run_log_scroll.set(next);
        }
        ChronicleScrollPane::Career => {
            let max_s = crate::ui::chronicle_dashboard::chronicle_right_pane_scroll_max(
                w,
                h,
                panel,
                progress,
                scene.focused_row,
            );
            let next = (scene.chronicle_dashboard_scroll.get() + delta).clamp(0.0, max_s);
            scene.chronicle_dashboard_scroll.set(next);
        }
    }
}

#[inline]
fn chronicle_sync_run_log_scroll(
    scene: &mut CollectionScene,
    w: f32,
    h: f32,
    progress: &crate::core::progression::PlayerProgress,
) {
    let panel = chronicle_panel_rect(w, h);
    let entry_count = archive_career::chronicle_list_entry_count(progress);
    let panes = crate::ui::chronicle_dashboard::chronicle_pane_layout(w, h, panel);
    let scroll = crate::ui::chronicle_dashboard::chronicle_clamp_run_log_scroll(
        scene.chronicle_run_log_scroll.get(),
        scene.focused_row,
        entry_count,
        panes,
        true,
    );
    let max_s =
        crate::ui::chronicle_dashboard::chronicle_run_log_scroll_max(w, h, panel, entry_count);
    scene.chronicle_run_log_scroll.set(scroll.clamp(0.0, max_s));
}

#[inline]
fn collection_chrome_is_tab(action: CollectionAction) -> bool {
    matches!(action, CollectionAction::SelectTab(_))
}

#[inline]
fn collection_chrome_is_bottom(action: CollectionAction) -> bool {
    matches!(
        action,
        CollectionAction::PrevPage | CollectionAction::NextPage
    )
}

fn collection_focused_artifact_rect(
    items: &[FlatItem<CollectionAction>],
    focused: Option<usize>,
) -> Option<[f32; 4]> {
    let fi = focused?;
    items.iter().find_map(|it| {
        if let CollectionAction::SelectArtifact(idx) = it.action
            && idx == fi
        {
            Some(it.rect)
        } else {
            None
        }
    })
}

fn collection_chrome_rect_for(
    items: &[FlatItem<CollectionAction>],
    action: CollectionAction,
) -> Option<[f32; 4]> {
    collection_chrome_rects(items)
        .into_iter()
        .find(|(a, _)| *a == action)
        .map(|(_, rect)| rect)
}

fn collection_tab_chrome_rects(
    items: &[FlatItem<CollectionAction>],
) -> Vec<(CollectionAction, [f32; 4])> {
    collection_chrome_rects(items)
        .into_iter()
        .filter(|(action, _)| collection_chrome_is_tab(*action))
        .collect()
}

#[inline]
fn collection_chronicle_tab_index() -> usize {
    TABS.len().saturating_sub(1)
}

#[inline]
fn collection_ordeals_tab_index() -> usize {
    collection_chronicle_tab_index().saturating_sub(1)
}

/// Tab targets allowed for a vertical move from one tab button. Chronicle sits
/// on a lower shelf and is only reachable downward from Ordeals; other tabs
/// should drop into the cabinet instead.
fn collection_tab_chrome_rects_for_vertical_step(
    items: &[FlatItem<CollectionAction>],
    from: CollectionAction,
    dir: FocusDir,
) -> Vec<(CollectionAction, [f32; 4])> {
    let chronicle_idx = collection_chronicle_tab_index();
    let bosses_idx = collection_ordeals_tab_index();
    collection_tab_chrome_rects(items)
        .into_iter()
        .filter(|(action, _)| match (from, dir) {
            (CollectionAction::SelectTab(i), FocusDir::Down) if i != bosses_idx => {
                !matches!(action, CollectionAction::SelectTab(ti) if *ti == chronicle_idx)
            }
            _ => true,
        })
        .collect()
}

/// Move chrome focus to the spatial neighbour of `from_rect` within `candidates`.
fn collection_focus_chrome_spatial(
    scene: &mut CollectionScene,
    bus: &mut crate::game::event_bus::EventBus,
    from_rect: [f32; 4],
    dir: FocusDir,
    candidates: &[(CollectionAction, [f32; 4])],
) -> bool {
    let Some(target) = pick_neighbor(from_rect, dir, candidates) else {
        return false;
    };
    scene.focused_chrome = Some(target);
    scene.tree.set_focus(target.id());
    bus.push(GameEvent::UiSound(SfxId::TilePlace));
    true
}

/// Move focus from the focused cabinet cell into chrome along `dir`.
fn collection_enter_chrome(
    scene: &mut CollectionScene,
    bus: &mut crate::game::event_bus::EventBus,
    items: &[FlatItem<CollectionAction>],
    dir: FocusDir,
) -> bool {
    let Some(from) = collection_focused_artifact_rect(items, scene.focused_row) else {
        return false;
    };
    collection_focus_chrome_spatial(scene, bus, from, dir, &collection_chrome_rects(items))
}

/// Directional move while chrome is focused.
fn collection_chrome_directional(
    scene: &mut CollectionScene,
    bus: &mut crate::game::event_bus::EventBus,
    items: &[FlatItem<CollectionAction>],
    dir: FocusDir,
) {
    let Some(cur) = scene.focused_chrome else {
        return;
    };
    let Some(cur_rect) = collection_chrome_rect_for(items, cur) else {
        return;
    };
    let on_tab = collection_chrome_is_tab(cur);
    let on_bottom = collection_chrome_is_bottom(cur);

    if on_tab && dir == FocusDir::Down {
        let tabs = collection_tab_chrome_rects_for_vertical_step(items, cur, FocusDir::Down);
        if collection_focus_chrome_spatial(scene, bus, cur_rect, FocusDir::Down, &tabs) {
            return;
        }
        scene.focused_chrome = None;
        if matches!(scene.active_tab, Tab::Chronicle) {
            scene.chronicle_focused_pane =
                crate::ui::chronicle_dashboard::ChronicleScrollPane::RunLog;
        }
        if let Some(fi) = scene.focused_row {
            scene
                .tree
                .set_focus(CollectionAction::SelectArtifact(fi).id());
        }
        bus.push(GameEvent::UiSound(SfxId::TilePlace));
        return;
    }

    if on_bottom && dir == FocusDir::Up {
        scene.focused_chrome = None;
        if matches!(scene.active_tab, Tab::Chronicle) {
            scene.chronicle_focused_pane =
                crate::ui::chronicle_dashboard::ChronicleScrollPane::RunLog;
        }
        if let Some(fi) = scene.focused_row {
            scene
                .tree
                .set_focus(CollectionAction::SelectArtifact(fi).id());
        }
        bus.push(GameEvent::UiSound(SfxId::TilePlace));
        return;
    }

    let chrome = collection_chrome_rects(items);
    collection_focus_chrome_spatial(scene, bus, cur_rect, dir, &chrome);
}

fn archive_marker_screen_rect(
    w: f32,
    h: f32,
    cam: &CameraParams,
    node_name: &str,
    rw: f32,
    rh: f32,
    env_h: f32,
) -> Option<[f32; 4]> {
    archive_glb::with_archive_glb_cpu(|opt| {
        let cpu = opt?;
        if let Some(r) =
            room_glb::screen_rect_for_marker_mesh_bounds(&room_glb::MarkerScreenRectParams {
                win_w: w,
                win_h: h,
                cam,
                env_height_scale: env_h,
                cpu,
                node_name,
                min_rw: rw,
                min_rh: rh,
            })
        {
            return Some(r);
        }
        let tw = room_glb::marker_translation(cpu, node_name)?
            * room_glb::room_env_world_scale(h, env_h);
        let (cx, cy) = cam.project_world_to_screen(w, h, tw);
        Some([cx - rw * 0.5, cy - rh * 0.5, rw, rh])
    })
}

fn archive_main_menu_btn_rect(w: f32, h: f32, cam: &CameraParams, env_h: f32) -> Option<[f32; 4]> {
    archive_marker_screen_rect(
        w,
        h,
        cam,
        archive_glb::BTN_MAIN_MENU,
        w * 0.11,
        h * 0.052,
        env_h,
    )
}

fn archive_switch_save_btn_rect(
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
) -> Option<[f32; 4]> {
    archive_marker_screen_rect(
        w,
        h,
        cam,
        archive_glb::BTN_SWITCH_SAVE,
        w * 0.14,
        h * 0.052,
        env_h,
    )
}

fn archive_page_left_btn_rect(w: f32, h: f32, cam: &CameraParams, env_h: f32) -> Option<[f32; 4]> {
    archive_marker_screen_rect(
        w,
        h,
        cam,
        archive_glb::BTN_PAGE_LEFT,
        w * 0.08,
        h * 0.08,
        env_h,
    )
}

fn archive_page_right_btn_rect(w: f32, h: f32, cam: &CameraParams, env_h: f32) -> Option<[f32; 4]> {
    archive_marker_screen_rect(
        w,
        h,
        cam,
        archive_glb::BTN_PAGE_RIGHT,
        w * 0.08,
        h * 0.08,
        env_h,
    )
}

/// Screen rect for the page indicator, centred horizontally and placed just
/// above the cabinet page buttons (uses both marker positions even when one
/// button is hidden).
fn archive_page_indicator_rect(
    w: f32,
    h: f32,
    cam: &CameraParams,
    env_h: f32,
    font_px: f32,
) -> Option<[f32; 4]> {
    let left = archive_page_left_btn_rect(w, h, cam, env_h)?;
    let right = archive_page_right_btn_rect(w, h, cam, env_h)?;
    let btn_top = left[1].min(right[1]);
    let btn_h = left[3].max(right[3]);
    let band_h = crate::ui::colored_keywords::colored_row_line_step(font_px);
    let gap = btn_h * 0.08;
    // Full-width band keeps the pinned body tier from shrinking to fit the
    // narrow gap between the projected page-button markers.
    Some([0.0, btn_top - gap - band_h, w, band_h])
}

/// Tab hit/focus rects projected from authored `btn_*_tab` meshes ([`TABS`] order).
fn archive_tab_hit_rects(w: f32, h: f32, env_h: f32, cam: &CameraParams) -> Vec<(usize, [f32; 4])> {
    let rw = w * 0.09;
    let rh = h * 0.06;
    let mut out = Vec::new();
    for (ti, node) in archive_glb::ARCHIVE_TAB_BUTTON_NODES
        .iter()
        .enumerate()
        .take(TABS.len())
    {
        let Some(rect) = archive_marker_screen_rect(w, h, cam, node, rw, rh, env_h) else {
            continue;
        };
        if flat_rect_xywh_is_finite(rect) {
            out.push((ti, rect));
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::focus_nav::rect_center;

    #[test]
    fn archive_tab_hit_rects_match_glb_button_meshes() {
        let w = 1920.0;
        let h = 1080.0;
        let env_h = crate::render::room_glb::SHOP_ENV_HEIGHT_SCALE;
        let cam = archive_glb::archive_camera_base(w, h, env_h);
        let rw = w * 0.09;
        let rh = h * 0.06;
        let rects = archive_tab_hit_rects(w, h, env_h, &cam);
        assert_eq!(rects.len(), TABS.len());
        for (i, (ti, rect)) in rects.iter().enumerate() {
            assert_eq!(*ti, i);
            assert!(flat_rect_xywh_is_finite(*rect));
            let node = archive_glb::ARCHIVE_TAB_BUTTON_NODES[*ti];
            let mesh = archive_marker_screen_rect(w, h, &cam, node, rw, rh, env_h)
                .expect("tab button mesh should project");
            assert_eq!(*rect, mesh, "tab {ti} should use the glb btn mesh rect");
        }
        let chronicle_idx = TABS.len() - 1;
        let (_, main_row_y) = rect_center(rects[0].1);
        let (_, chronicle_y) = rect_center(rects[chronicle_idx].1);
        assert!(
            chronicle_y > main_row_y + h * 0.2,
            "chronicle tab should sit on the lower shelf"
        );
    }

    #[test]
    fn archive_chronicle_tab_spatially_reachable_from_bosses_and_grid() {
        use crate::core::progression::PlayerProgress;
        use crate::ui::focus_nav::pick_neighbor;

        let w = 1920.0;
        let h = 1080.0;
        let env_h = crate::render::room_glb::SHOP_ENV_HEIGHT_SCALE;
        let cam = archive_glb::archive_camera_base(w, h, env_h);
        let tab_rects = archive_tab_hit_rects(w, h, env_h, &cam);
        let chrome: Vec<(CollectionAction, [f32; 4])> = tab_rects
            .iter()
            .map(|(ti, rect)| (CollectionAction::SelectTab(*ti), *rect))
            .collect();
        let chronicle_idx = TABS.len() - 1;
        let bosses_idx = chronicle_idx - 1;
        let bosses = chrome
            .iter()
            .find(
                |(action, _)| matches!(action, CollectionAction::SelectTab(i) if *i == bosses_idx),
            )
            .expect("bosses tab rect")
            .1;
        assert_eq!(
            pick_neighbor(bosses, FocusDir::Down, &chrome),
            Some(CollectionAction::SelectTab(chronicle_idx)),
            "Down from Ordeals should reach the lower-shelf Chronicle btn"
        );
        let relics = chrome
            .iter()
            .find(|(action, _)| matches!(action, CollectionAction::SelectTab(0)))
            .expect("relics tab rect")
            .1;
        let relics_tabs: Vec<_> = chrome
            .iter()
            .filter(|(action, _)| {
                !matches!(action, CollectionAction::SelectTab(i) if *i == chronicle_idx)
            })
            .copied()
            .collect();
        assert!(
            pick_neighbor(relics, FocusDir::Down, &relics_tabs).is_none(),
            "Down from main-row tabs should not reach Chronicle"
        );

        let mut progress = PlayerProgress::default();
        progress.cheat_unlock_all_transformation_chains_meta();
        let scene = CollectionScene::new();
        let items = scene.flat_items(w, h, &progress, env_h, 0);
        let chrome = collection_chrome_rects(&items);
        let cols = archive_glb::ARCHIVE_SLOT_COLS.max(1);
        let page_size = archive_page_size().max(1);
        let bottom_right = collection_artifact_hit_rects(&items)
            .into_iter()
            .max_by(|(a_idx, a_rect), (b_idx, b_rect)| {
                let a_slot = a_idx % page_size;
                let b_slot = b_idx % page_size;
                let a_row = a_slot / cols;
                let b_row = b_slot / cols;
                let a_col = a_slot % cols;
                let b_col = b_slot % cols;
                a_row
                    .cmp(&b_row)
                    .then(a_col.cmp(&b_col))
                    .then((a_rect[0] + a_rect[2]).total_cmp(&(b_rect[0] + b_rect[2])))
            })
            .map(|(_, rect)| rect)
            .expect("cabinet should surface at least one artifact rect");
        assert_eq!(
            pick_neighbor(bottom_right, FocusDir::Down, &chrome),
            Some(CollectionAction::SelectTab(chronicle_idx)),
            "Down from the bottom-right cabinet slot should reach Chronicle"
        );

        let chronicle = chrome
            .iter()
            .find(|(action, _)| matches!(action, CollectionAction::SelectTab(i) if *i == chronicle_idx))
            .expect("chronicle tab rect")
            .1;
        assert!(
            pick_neighbor(chronicle, FocusDir::Left, &chrome).is_some(),
            "Left from Chronicle should reach some chrome neighbour"
        );
    }
}
