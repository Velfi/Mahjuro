//! Pick-blind scene — Balatro-style: shows all three blinds for the
//! current ante (Small / Big / Boss) at once as 3D shrine objects, each
//! larger than the last, with the upcoming one lit by a literal warm
//! spotlight `PointLight`. Already-cleared shrines are dim slate; the
//! upcoming one glows champagne; future shrines sit in cool fill.
//!
//! The boss shrine takes its themed boss tier color from
//! `RunState::upcoming_boss`, so the player can read severity at a
//! glance before they reach the fight.
//!
//! Mirrors the shop scene's `draw_frame() -> UiFrame` pattern: a custom
//! `PickBlindLayout` builds the camera + per-shrine world positions, the
//! 3D meshes get pushed via `frame.shrine_batch(...)`, and a 2D HUD
//! (panel headers, per-shrine labels, skip button) sits on top.

use std::cell::Cell;

use crate::audio::SfxId;
use crate::core::rules::BlindKind;
use crate::game::engine::{GameCommand, GameEngine};
use crate::game::event_bus::GameEvent;
use crate::render::draw_cmd::{
    CameraParams, Object3d, Object3dKind, UiFrame, camera_facing_rotation,
};
use crate::render::table_transform::{mesh_y_thickness_along_local_y_to_z_up, rot_z_rad};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextLabel};
use crate::ui::focus_nav::push_focus_ring;
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::gameplay::GameplayScene;
use super::pause_menu::PauseMenu;
use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

/// `pick_id` for the play altar `DishExplicit`. Used to look up its
/// projected screen rect in `ctx.aux_dish_rects` for label placement.
const PICK_PLAY_DISH: u32 = 1;
/// `pick_id` for the skip altar `DishExplicit`.
const PICK_SKIP_DISH: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlindAction {
    PlayBlind,
    SkipBlind,
}

impl BlindAction {
    fn id(self) -> FocusId {
        FocusId(self as u32 + 1)
    }
}

pub struct PickBlindScene {
    tree: TreeState,
    pause_menu: PauseMenu,
    /// Cached projected screen rect of the play altar (from the previous
    /// frame). Written by `draw_frame()`, read by `update()` so the
    /// cursor hit-test matches the actual rendered position of the
    /// 3D dish (which is camera-projected, not at the raw pixel anchor).
    /// `None` until the first frame has been drawn.
    last_play_rect: Cell<Option<[f32; 4]>>,
    /// Cached projected screen rect of the skip altar.
    last_skip_rect: Cell<Option<[f32; 4]>>,
}

impl PickBlindScene {
    pub fn new() -> Self {
        let mut tree = TreeState::new();
        tree.set_focus(BlindAction::PlayBlind.id());
        Self {
            tree,
            pause_menu: PauseMenu::new(),
            last_play_rect: Cell::new(None),
            last_skip_rect: Cell::new(None),
        }
    }

    fn can_skip(blind: BlindKind) -> bool {
        !matches!(blind, BlindKind::Boss)
    }

    /// Hit-test rects shared between update() and draw(). Both Play and
    /// Skip are 3D altar objects on the floor — their *visible* screen
    /// position depends on the camera projection, not the raw pixel
    /// anchor we hand the renderer. We prefer the previous frame's
    /// projected screen rect (cached on `self`) and fall back to a
    /// generous bounding box around the unprojected anchor on the
    /// first frame, before any projection has run.
    fn flat_items(
        layout: &PickBlindLayout,
        _upcoming: BlindKind,
        can_skip: bool,
        cached_play: Option<[f32; 4]>,
        cached_skip: Option<[f32; 4]>,
    ) -> Vec<FlatItem<BlindAction>> {
        let play_rect = cached_play.map(inflate_hit_rect).unwrap_or_else(|| {
            let (px, py) = layout.play_dish_anchor_px;
            let pext = layout.play_dish_extents;
            let pw = (pext[0] * 2.20).max(220.0);
            let ph = (pext[2] * 4.20).max(160.0);
            [px - pw * 0.5, py - ph * 0.55, pw, ph]
        });

        let mut items = vec![FlatItem::new(
            BlindAction::PlayBlind.id(),
            play_rect,
            BlindAction::PlayBlind,
        )];
        if can_skip {
            let skip_rect = cached_skip.map(inflate_hit_rect).unwrap_or_else(|| {
                let (sx, sy) = layout.skip_dish_anchor_px;
                let sext = layout.skip_dish_extents;
                let sw = (sext[0] * 2.20).max(220.0);
                let sh = (sext[2] * 4.20).max(160.0);
                [sx - sw * 0.5, sy - sh * 0.55, sw, sh]
            });
            items.push(FlatItem::new(
                BlindAction::SkipBlind.id(),
                skip_rect,
                BlindAction::SkipBlind,
            ));
        }
        items
    }

    fn skip_focused(&self) -> bool {
        self.tree.focused() == Some(BlindAction::SkipBlind.id())
    }

    fn play_focused(&self) -> bool {
        self.tree.focused() == Some(BlindAction::PlayBlind.id())
    }
}

/// Shared placement for the floating plaque above the upcoming shrine.
/// Keeping the light and the physical plaque on the same anchor avoids
/// "fixing" the mesh position while leaving its highlight behind.
///
/// `plaque_h` is the (clamped) vertical extent of the plaque. On small
/// screens the plaque hits its minimum-size clamp while the shrine keeps
/// scaling down with the window, so a pure `shrine_ext[1] * factor` lift
/// puts the plaque's bottom edge inside the shrine roof. We enforce a
/// minimum clearance between the plaque bottom and the shrine top so the
/// plaque stays clearly floating above regardless of window size.
fn upcoming_plaque_anchor(
    upcoming: BlindKind,
    shrine_px: f32,
    shrine_py: f32,
    shrine_ext: [f32; 3],
    plaque_h: f32,
) -> (f32, f32, f32) {
    let (py_factor, world_y_factor) = if upcoming == BlindKind::Small {
        (-0.06, 1.62)
    } else {
        (0.10, 1.45)
    };
    let plaque_py = shrine_py + shrine_ext[2] * py_factor;
    let natural_world_y = shrine_ext[1] * world_y_factor;
    // Ensure the plaque bottom sits at least `min_clearance` above the
    // shrine top. On large screens `natural_world_y` already exceeds this
    // floor and the branch is a no-op.
    let min_clearance = plaque_h * 0.25;
    let min_world_y = shrine_ext[1] + plaque_h * 0.5 + min_clearance;
    let plaque_world_y = natural_world_y.max(min_world_y);
    (shrine_px, plaque_py, plaque_world_y)
}

/// Inflate a tight projected screen rect into a generous click target.
/// The renderer's projected dish rects are sized to the actual visible
/// dish silhouette, which is small for our altar dishes (~80×40px). We
/// pad them out so the cursor doesn't have to land pixel-perfectly on
/// the dish to register a click.
fn inflate_hit_rect(rect: [f32; 4]) -> [f32; 4] {
    let pad_x = (rect[2] * 0.80).max(60.0);
    let pad_y = (rect[3] * 1.50).max(60.0);
    [
        rect[0] - pad_x,
        rect[1] - pad_y * 0.6,
        rect[2] + pad_x * 2.0,
        rect[3] + pad_y * 1.4,
    ]
}

/// Index of `blind` in the canonical Small → Big → Boss order.
fn upcoming_index(blind: BlindKind) -> usize {
    match blind {
        BlindKind::Small => 0,
        BlindKind::Big => 1,
        BlindKind::Boss => 2,
    }
}

/// Estimate how many wrapped lines `text` will occupy under a simple
/// character-budget word wrap. Mirrors the ofuda decal's greedy wrap well
/// enough for scene-side sizing without coupling to renderer internals.
fn estimated_wrapped_lines(text: &str, max_chars: usize) -> usize {
    let max_chars = max_chars.max(1);
    let mut lines: usize = 0;
    let mut current_len: usize = 0;

    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if current_len == 0 {
            lines += word_len.div_ceil(max_chars).max(1);
            current_len = word_len % max_chars;
            if current_len == 0 && word_len > 0 {
                current_len = max_chars;
            }
        } else if current_len + 1 + word_len <= max_chars {
            current_len += 1 + word_len;
        } else {
            lines += word_len.div_ceil(max_chars).max(1);
            current_len = word_len % max_chars;
            if current_len == 0 && word_len > 0 {
                current_len = max_chars;
            }
        }
    }

    if lines == 0 { 1 } else { lines }
}

/// Pick-blind shrine ofuda sizing. Boss names and rule overrides vary a lot
/// in length, so the paper grows to fit the expected wrapped line count
/// instead of using one fixed aspect for every boss.
fn auto_size_shrine_ofuda(plaque_w: f32, plaque_h: f32, title: &str, rule: &str) -> (f32, f32) {
    let title_chars = title.chars().count();
    let rule_chars = rule.chars().count();
    let longest_rule_word = rule
        .split_whitespace()
        .map(|word| word.chars().count())
        .max()
        .unwrap_or(0);

    let width_scale = (0.52
        + (title_chars.saturating_sub(12) as f32) * 0.012
        + (longest_rule_word.saturating_sub(10) as f32) * 0.010
        + (rule_chars.saturating_sub(40) as f32) * 0.004)
        .clamp(0.58, 0.92);
    let ofuda_w = plaque_w * width_scale;

    // Approximate the decal's title/rule wrapping budgets from the chosen
    // paper width. Wider papers let both bands use larger line budgets.
    let title_chars_per_line = ((ofuda_w / plaque_h.max(1.0)) * 8.0).round() as usize;
    let rule_chars_per_line = ((ofuda_w / plaque_h.max(1.0)) * 13.0).round() as usize;
    let title_lines = estimated_wrapped_lines(title, title_chars_per_line.max(10));
    let rule_lines = estimated_wrapped_lines(rule, rule_chars_per_line.max(14));

    let line_units = title_lines as f32 * 1.15 + rule_lines as f32;
    let height_scale = (0.92 + line_units * 0.34).clamp(1.45, 2.70);
    let ofuda_h = plaque_h * height_scale;
    (ofuda_w, ofuda_h)
}

/// Visual state of one of the three shrines, derived from `upcoming_blind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShrineState {
    Cleared,
    Upcoming,
    Future,
}

fn shrine_state(idx: usize, upcoming_idx: usize) -> ShrineState {
    if idx < upcoming_idx {
        ShrineState::Cleared
    } else if idx == upcoming_idx {
        ShrineState::Upcoming
    } else {
        ShrineState::Future
    }
}

/// Spatial layout of the three pick-blind shrines, computed once per frame
/// from the window size. Mirrors the shop scene's `ShopLayout` pattern.
#[derive(Clone, Copy)]
struct PickBlindLayout {
    /// 3D camera framing all three shrines.
    camera: CameraParams,
    /// Per-shrine base anchor in pixel space (px, py). World_y for the
    /// base is 0 (sitting on the table).
    shrine_anchors_px: [(f32, f32); 3],
    /// Per-shrine full extents in world units (width × height × depth).
    /// Indexed Small=0, Big=1, Boss=2; each strictly larger than the
    /// previous.
    shrine_extents: [[f32; 3]; 3],
    /// Floor plane anchor (px, py). A wide flat slab the shrines stand
    /// on, drawn via a giant `DishExplicit` at world_y=0.
    floor_anchor_px: (f32, f32),
    /// Floor plane extents (width × thickness × depth) in world units.
    floor_extents: [f32; 3],
    /// 3D Play altar anchor (px, py) — sits in front of the shrines on
    /// the floor, left of center. World_y for the dish base is 0.
    play_dish_anchor_px: (f32, f32),
    /// 3D Play altar extents (dish width × rim height × depth).
    play_dish_extents: [f32; 3],
    /// 3D Skip dish anchor (px, py) — sits in front of the shrines on
    /// the floor, right of center.
    skip_dish_anchor_px: (f32, f32),
    /// 3D Skip dish extents (dish width × rim height × depth).
    skip_dish_extents: [f32; 3],
}

impl PickBlindLayout {
    fn build(layout: &crate::ui::layout::LayoutResult, upcoming_idx: usize) -> Self {
        let w = layout.window_w;
        let h = layout.window_h;
        // ── Camera: front-elevated, looking at the shrine row ────────────
        // Mirrors the shop's "person standing in front of the cabinet"
        // setup. The eye is dropped a bit so the boss shrine reads as
        // towering, and the target sits low+forward so the floor plane
        // catches the spotlight nicely.
        let camera = CameraParams {
            eye: [0.0, -h * 1.25, h * 0.50],
            target: [0.0, h * 0.05, h * 0.18],
            up: [0.0, 0.0, 1.0],
            fovy_deg: 55.0,
        };

        // ── Shrine layout: three bases in a left→right row, increasing
        // size. Centers are placed symmetrically around the screen center;
        // each shrine's *base* sits at the same pixel-y so the row reads
        // as a procession on a single platform. Shrine row pulled UP a
        // bit so the bottom info panel + buttons have clear screen real
        // estate without overlapping the shrines.
        let row_y = h * 0.50;
        let shrine_anchors_px = [
            (w * 0.20, row_y), // Small (left)
            (w * 0.50, row_y), // Big (center)
            (w * 0.80, row_y), // Boss (right)
        ];

        // Base unit derived from window height so the shrines scale with
        // resolution. The smallest shrine is roughly half the size of the
        // boss; ratios are tuned so the three are visibly distinct
        // without the small one becoming a token.
        let base_h = h * 0.36;
        let base_w = base_h * 0.78;
        let base_d = base_h * 0.55;
        let scale = |k: f32| [base_w * k, base_h * k, base_d * k];
        let shrine_extents = [
            scale(0.58), // Small
            scale(0.78), // Big
            scale(1.00), // Boss — the visual anchor of the row
        ];

        // ── Floor plane: a wide flat slab the shrines stand on. Drawn
        // via a giant `DishExplicit` (low rim height) so it reuses the
        // existing dish mesh + render path. The plane spans from well
        // before the small shrine to well after the boss shrine, with
        // depth that catches the spotlight pool around each base.
        let floor_anchor_px = (w * 0.50, row_y);
        // Floor slab thickness — a thin stage platform (~6mm) so the
        // shrines clearly stand on something without that something
        // dominating the scene.
        let floor_extents = [w * 0.95, layout.mm(6.0), h * 0.30];

        // ── 3D Play / Skip altars BELOW the upcoming shrine ─────────
        // The two altars sit side by side below the upcoming shrine's
        // base, centered on its horizontal position. They move with the
        // upcoming shrine — when the player advances to Big Blind
        // they'll appear below the middle shrine, etc.
        let (up_px, _up_py) = shrine_anchors_px[upcoming_idx];
        let _up_ext = shrine_extents[upcoming_idx];
        let altar_row_y = row_y + h * 0.16;
        // Brass altar dishes — small offering trays (~9mm rim height).
        let play_dish_extents = [h * 0.12, layout.mm(9.0), h * 0.06];
        let skip_dish_extents = [h * 0.10, layout.mm(8.0), h * 0.055];
        // Side-by-side gap between the two dishes, centered under shrine.
        // Generous spacing so they don't collide on small screens.
        let inner_gap = (h * 0.12).max(80.0);
        let play_x = (up_px - inner_gap * 0.5 - play_dish_extents[0] * 0.5)
            .max(play_dish_extents[0] * 0.5 + 16.0)
            .min(w - play_dish_extents[0] * 0.5 - 16.0);
        let skip_x = (up_px + inner_gap * 0.5 + skip_dish_extents[0] * 0.5)
            .max(skip_dish_extents[0] * 0.5 + 16.0)
            .min(w - skip_dish_extents[0] * 0.5 - 16.0);
        let play_dish_anchor_px = (play_x, altar_row_y);
        let skip_dish_anchor_px = (skip_x, altar_row_y);

        Self {
            camera,
            shrine_anchors_px,
            shrine_extents,
            floor_anchor_px,
            floor_extents,
            play_dish_anchor_px,
            play_dish_extents,
            skip_dish_anchor_px,
            skip_dish_extents,
        }
    }

    fn shrine_pixel_anchor(&self, idx: usize) -> (f32, f32) {
        self.shrine_anchors_px[idx]
    }

    fn shrine_extents(&self, idx: usize) -> [f32; 3] {
        self.shrine_extents[idx]
    }

    /// World-space anchor for a `PointLight` that should illuminate
    /// shrine `idx` from above and slightly in front. Mirrors the shop's
    /// hover-spotlight placement: light pos is in pixel-y / world-y mixed
    /// coordinates, where pos[2] is height above the table. We want the
    /// light just above the shrine roof (world_y ≈ ext[1] + small lift)
    /// so the bowl + plinth catch the warm pool, not just the roof.
    fn spotlight_pos(&self, idx: usize) -> [f32; 3] {
        let (px, py) = self.shrine_anchors_px[idx];
        let ext = self.shrine_extents[idx];
        // Pull the light slightly forward in pixel-y (toward the camera)
        // and place it just above the shrine top in world_y.
        [px, py + ext[2] * 0.20, ext[1] * 1.10]
    }
}

impl SceneBehavior for PickBlindScene {
    fn pause_options_overlay(&self) -> Option<&super::options::OptionsScene> {
        self.pause_menu.options_overlay()
    }

    fn has_blocking_overlay(&self) -> bool {
        self.pause_menu.paused
    }

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        // Pick up a pending zodiac celebration from a ZodiacBlessing tag.
        if let Some((kind, yaku, new_level)) = GameEngine::take_pending_zodiac_celebration(ctx.run)
        {
            ctx.bus
                .push(crate::game::event_bus::GameEvent::ZodiacReveal);
            *ctx.overlay_request = Some(super::OverlayRequest::Push(Box::new(
                Scene::ZodiacCelebration(super::ZodiacCelebrationScene::new(
                    kind,
                    yaku.name(),
                    new_level,
                )),
            )));
            return None;
        }

        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            return t;
        }

        let pick = GameEngine::read_pick_blind(ctx.run);
        let upcoming = pick.upcoming_blind;
        let can_skip = Self::can_skip(upcoming);

        let layout = PickBlindLayout::build(ctx.layout, upcoming_index(upcoming));
        let items = Self::flat_items(
            &layout,
            upcoming,
            can_skip,
            self.last_play_rect.get(),
            self.last_skip_rect.get(),
        );
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                ui_scale: ctx.ui_scale,
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }

        for a in ctx.actions {
            if matches!(a, UiAction::Cancel) && can_skip {
                let mut engine = GameEngine::new(ctx.run, ctx.bus);
                let _ = engine.dispatch(GameCommand::SkipUpcomingBlindWithTag);
                return Some(Scene::PickBlind(PickBlindScene::new()));
            }
        }

        match action {
            Some(BlindAction::SkipBlind) if can_skip => {
                let mut engine = GameEngine::new(ctx.run, ctx.bus);
                let _ = engine.dispatch(GameCommand::SkipUpcomingBlindWithTag);
                Some(Scene::PickBlind(PickBlindScene::new()))
            }
            Some(BlindAction::PlayBlind) | Some(BlindAction::SkipBlind) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::RoundStart));
                // Record boss encounters the moment the player commits to
                // fighting one. "Encountered" = selected via PlayBlind, so
                // skips don't count and unseen bosses stay hidden in the
                // Collection.
                if upcoming == BlindKind::Boss
                    && let Some(bk) = pick.boss_kind
                {
                    ctx.bus.push(GameEvent::BossEncountered(bk));
                }
                Some(Scene::Gameplay(GameplayScene::with_pending_blind(upcoming)))
            }
            None => None,
        }
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let ui_scale = ctx.ui_scale;

        let pick = GameEngine::read_pick_blind(ctx.run);
        let upcoming = pick.upcoming_blind;
        let can_skip = Self::can_skip(upcoming);
        let upcoming_idx = upcoming_index(upcoming);
        let layout = PickBlindLayout::build(ctx.layout, upcoming_idx);
        let blinds = [BlindKind::Small, BlindKind::Big, BlindKind::Boss];

        let mut frame = UiFrame::new();
        // Pure black temple background via the synthetic 1×1 black
        // background texture, so the fill draws in pass A *before* the
        // smoke composite. A fullscreen quad would be reordered into the
        // late HUD overlay pass and paint over the smoke. (Earlier we
        // layered dark indigo + vignettes, but the gamma-encoded linear
        // floor of even [0.002] reads as visible indigo on screen.)
        frame.background(BackgroundId::Black);
        frame.ember_drift();
        frame.camera_override = Some(layout.camera);

        // ── 3D shrines ────────────────────────────────────────────────
        let mut shrine_objects: Vec<Object3d> = Vec::with_capacity(3);
        for (i, &blind) in blinds.iter().enumerate() {
            let (px, py) = layout.shrine_pixel_anchor(i);
            let extents = layout.shrine_extents(i);
            let state = shrine_state(i, upcoming_idx);

            // Per-state base color. The shrines are weathered stone, so
            // colors are desaturated greys/browns rather than the warm
            // champagne and cool dusk used previously. Cleared/future
            // shrines are dark stone that catches very little light;
            // the upcoming shrine is a lighter limestone that the warm
            // spotlight tints gold without blowing out. The boss shrine
            // still takes its tier color but desaturated halfway to a
            // neutral stone grey so it reads as "colored stone" rather
            // than "painted plastic".
            let stone_dark: [f32; 4] = [0.16, 0.15, 0.14, 1.0];
            let stone_mid: [f32; 4] = [0.22, 0.21, 0.19, 1.0];
            let stone_light: [f32; 4] = [0.50, 0.46, 0.40, 1.0];
            let blend_with_stone = |c: [f32; 4], t: f32| -> [f32; 4] {
                [
                    stone_light[0] + (c[0] - stone_light[0]) * t,
                    stone_light[1] + (c[1] - stone_light[1]) * t,
                    stone_light[2] + (c[2] - stone_light[2]) * t,
                    c[3],
                ]
            };
            let base_color = match (state, blind) {
                (ShrineState::Cleared, _) => stone_dark,
                (ShrineState::Future, _) => stone_mid,
                (ShrineState::Upcoming, BlindKind::Boss) => pick
                    .boss_kind
                    .map(|k| blend_with_stone(k.tier().halo_color(), 0.45))
                    .unwrap_or(stone_light),
                (ShrineState::Upcoming, _) => stone_light,
            };
            // Lower glow so the upcoming shrine isn't blown out by its
            // own brightening alone — most of the warmth should come
            // from the spotlight tinting the stone, not from the
            // shrine self-illuminating.
            let glow = if state == ShrineState::Upcoming {
                0.30
            } else {
                0.0
            };

            shrine_objects.push(Object3d {
                pos: [px, py, 0.0],
                extents,
                rotation: glam::Mat4::IDENTITY,
                color: base_color,
                kind: Object3dKind::Shrine { glow },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
        }
        frame.object3d_batch(shrine_objects);

        // ── Floor plane under the shrines ────────────────────────────
        // A wide flat slab the shrines stand on, drawn via a giant
        // `DishExplicit` (low rim height) so it reuses the existing
        // dish mesh + render path. The plane catches the spotlight
        // pool and grounds the shrines.
        let (fx, fy) = layout.floor_anchor_px;
        frame.object3d(Object3d {
            pos: [fx, fy, layout.floor_extents[1] * 0.5],
            extents: layout.floor_extents,
            rotation: mesh_y_thickness_along_local_y_to_z_up(),
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::DiscSquare,
                material: crate::render::primitive::MaterialSpec::plain(),
                pick_id: None,
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: None,
        });

        // ── Play + Skip altars on the floor in front of the shrines ─
        // Two small dishes side by side. Play is on the LEFT and holds
        // a single big golden coin (the "ritual offering" — make it to
        // begin). Skip is on the RIGHT and holds a small pile of coins
        // (the tribute reward for walking away). Both reuse the shop's
        // existing dish + coin meshes.
        let skip_tag = pick.skip_tag;
        let (play_px, play_py) = layout.play_dish_anchor_px;
        let play_dext = layout.play_dish_extents;
        frame.object3d(Object3d {
            pos: [play_px, play_py, play_dext[1] * 0.5],
            extents: play_dext,
            rotation: mesh_y_thickness_along_local_y_to_z_up(),
            color: [1.0, 1.0, 1.0, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::DiscSquare,
                material: crate::render::primitive::MaterialSpec::plain(),
                pick_id: Some(PICK_PLAY_DISH),
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: None,
        });
        // Single large golden coin in the center of the play dish.
        let play_dish_top_y = play_dext[1] + 2.0;
        frame.object3d_batch(vec![Object3d {
            pos: [play_px, play_py, play_dish_top_y],
            extents: [14.0 * 2.0, 5.5, 14.0 * 2.0],
            rotation: rot_z_rad(0.4),
            color: [1.00, 0.84, 0.30, 1.0],
            kind: Object3dKind::Primitive {
                shape: crate::render::primitive::MeshId::Cylinder,
                material: crate::render::primitive::MaterialSpec::metal(),
                pick_id: None,
                shadow_caster: true,
                silhouette: false,
            },
            hover_target: 0.0,
            anim_id: 0,
            arrange_name: None,
        }]);

        if can_skip {
            let (skip_px, skip_py) = layout.skip_dish_anchor_px;
            let skip_dext = layout.skip_dish_extents;
            frame.object3d(Object3d {
                pos: [skip_px, skip_py, skip_dext[1] * 0.5],
                extents: skip_dext,
                rotation: mesh_y_thickness_along_local_y_to_z_up(),
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::Primitive {
                    shape: crate::render::primitive::MeshId::DiscSquare,
                    material: crate::render::primitive::MaterialSpec::plain(),
                    pick_id: Some(PICK_SKIP_DISH),
                    shadow_caster: true,
                    silhouette: false,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
            // Single token on the skip dish, tinted by tag rarity.
            let dish_top_y = skip_dext[1] + 2.0;
            let tag_color = match skip_tag.map(|t| t.rarity()) {
                Some(crate::core::tag::TagRarity::Rare) => [1.00, 0.84, 0.30, 1.0], // gold
                Some(crate::core::tag::TagRarity::Uncommon) => [0.55, 0.85, 0.55, 1.0], // jade
                _ => [0.82, 0.82, 0.88, 1.0],                                       // silver
            };
            frame.object3d_batch(vec![Object3d {
                pos: [skip_px, skip_py, dish_top_y],
                extents: [12.0 * 2.0, 4.5, 12.0 * 2.0],
                rotation: rot_z_rad(0.4),
                color: tag_color,
                kind: Object3dKind::Primitive {
                    shape: crate::render::primitive::MeshId::Cylinder,
                    material: crate::render::primitive::MaterialSpec::metal(),
                    pick_id: None,
                    shadow_caster: true,
                    silhouette: false,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            }]);
        }

        // ── Lighting: temple hall at night ────────────────────────────
        // The pick-blind hall is intentionally darker than the shop:
        // the upcoming shrine should be the ONLY thing the player's eye
        // lands on. We deliberately avoid ANY centered ambient lights
        // because they create hot specular streaks on the middle shrine's
        // central pillar. Instead we use per-shrine local fills that are
        // anchored to each shrine's own position, so the lighting on each
        // shrine is independent and the upcoming spotlight doesn't have
        // to compete with anything centered on the row.
        let mut point_lights: Vec<PointLight> = Vec::new();

        // Per-cleared/future shrine: a single dim cool fill from in
        // front, just enough to keep the silhouette readable. Intensity
        // and warmth scale with *distance* from the upcoming shrine —
        // the shrine "next in line" reads brighter and slightly warmer
        // (anticipation), the one furthest away is dimmest + coolest.
        for i in 0..3 {
            if i == upcoming_idx {
                continue;
            }
            let (px, py) = layout.shrine_pixel_anchor(i);
            let ext = layout.shrine_extents(i);
            let dist = (i as i32 - upcoming_idx as i32).unsigned_abs();
            // distance 1 → more visible, distance 2 → very dim.
            let (color, intensity) = match dist {
                1 => ([0.62, 0.74, 1.00], 0.70),
                _ => ([0.45, 0.58, 0.95], 0.38),
            };
            point_lights.push(PointLight {
                pos: [px, py + ext[2] * 0.30, ext[1] * 0.50],
                radius: ext[1] * 1.30,
                color,
                intensity,
            });
        }

        // ── Upcoming shrine: spotlight + close warm key ───────────────
        // Spotlight: tight, intense, warm gold beam from above the
        // shrine roof. Smaller radius + higher intensity so falloff is
        // dramatic — this IS the focal point of the scene. When the
        // player's focus is on Play (cursor over the shrine OR keyboard
        // focused), the spotlight pulses up ~15% brighter as visual
        // feedback that "click/Enter will activate this".
        let play_focused = self.play_focused();
        let focus_boost = if play_focused { 1.15 } else { 1.0 };
        let (up_px, up_py) = layout.shrine_pixel_anchor(upcoming_idx);
        let up_ext = layout.shrine_extents(upcoming_idx);
        let spot = layout.spotlight_pos(upcoming_idx);
        // The floating wood plaque sits above the upcoming shrine and needs
        // its own narrow key so the engraved lettering reads independently of
        // the shrine spotlight. Keep the pool tight enough that it brightens
        // the plaque/ofuda cluster without washing the stone roof below.
        let plaque_w = (w * 0.70).clamp(560.0, 1000.0);
        let plaque_h = (h * 0.26).clamp(190.0, 300.0);
        let (raw_plaque_px, plaque_py, plaque_world_y) =
            upcoming_plaque_anchor(upcoming, up_px, up_py, up_ext, plaque_h);
        let plaque_px = raw_plaque_px.clamp(plaque_w * 0.5 + 16.0, w - plaque_w * 0.5 - 16.0);
        point_lights.push(PointLight {
            pos: spot,
            radius: up_ext[1] * 2.20,
            color: [1.00, 0.92, 0.72],
            intensity: 2.20 * focus_boost,
        });
        // Close warm fill from in front of the shrine, lighting the
        // bowl + plinth + pillar so the whole structure glows. Sits at
        // mid-height so its falloff covers the full body. Dimmed to
        // match the new lower spotlight — the stone shrines need much
        // less light to read as "lit" than the previous polished mesh.
        point_lights.push(PointLight {
            pos: [up_px, up_py + up_ext[2] * 0.40, up_ext[1] * 0.55],
            radius: up_ext[1] * 1.50,
            color: [1.00, 0.84, 0.52],
            intensity: 1.30 * focus_boost,
        });
        // Floor bounce — a small warm pool right at the base of the
        // upcoming shrine, simulating candle light pooling on the
        // temple floor around it. Sells the "shrine on the altar" read.
        point_lights.push(PointLight {
            pos: [up_px, up_py + 20.0, 8.0],
            radius: up_ext[0] * 1.80,
            color: [1.00, 0.76, 0.40],
            intensity: 0.90,
        });
        // Plaque accent: a smaller, slightly brighter warm-white light
        // hovering just in front of the sign band so the engraved text
        // reads independently of the shrine roof spotlight.
        point_lights.push(PointLight {
            pos: [
                plaque_px,
                plaque_py - plaque_h * 0.06,
                plaque_world_y + plaque_h * 0.45,
            ],
            radius: plaque_w * 0.46,
            color: [1.00, 0.93, 0.80],
            intensity: 1.45 * focus_boost,
        });
        if upcoming == BlindKind::Boss
            && let (Some(kind), Some(description)) =
                (pick.boss_kind, pick.boss_description.as_deref())
        {
            let def = kind.def();
            let (ofuda_w, ofuda_h) =
                auto_size_shrine_ofuda(plaque_w, plaque_h, def.name, description);
            let ofuda_gap = (plaque_w * 0.06).clamp(56.0, 84.0);
            // Mirror the light anchor to the ofuda's drawn position below
            // (the wider gap keeps the plaque's rotated side from clipping
            // the paper decal).
            let ofuda_px =
                (plaque_px - plaque_w * 0.5 - ofuda_w * 0.5 - ofuda_gap).max(ofuda_w * 0.5 + 8.0);
            let ofuda_py = plaque_py + up_ext[2] * 0.15 + 24.0;
            let ofuda_world_y = plaque_world_y * 0.86 + 6.0;
            point_lights.push(PointLight {
                pos: [
                    ofuda_px - ofuda_w * 0.06,
                    ofuda_py + ofuda_h * 0.03,
                    ofuda_world_y + ofuda_w * 0.32,
                ],
                radius: ofuda_h * 1.10,
                color: [1.00, 0.95, 0.82],
                intensity: 1.35 * focus_boost,
            });
        }

        // ── Play altar spotlight ─────────────────────────────────────
        // Always-on warm key on the play altar so the golden coin reads
        // as a "ready offering". Pulses brighter when Play is focused
        // (cursor over OR keyboard focused) — the visual hover state.
        let skip_focused_now = self.skip_focused();
        let play_boost = if play_focused { 2.20 } else { 1.0 };
        let (pdx, pdy) = layout.play_dish_anchor_px;
        let pdext = layout.play_dish_extents;
        point_lights.push(PointLight {
            pos: [pdx, pdy - pdext[2] * 0.20, pdext[1] * 5.0],
            radius: pdext[0] * 2.40,
            color: [1.00, 0.90, 0.55],
            intensity: 1.60 * play_boost,
        });

        // ── Skip altar spotlight ─────────────────────────────────────
        if can_skip {
            let skip_boost = if skip_focused_now { 2.20 } else { 1.0 };
            let (sdx, sdy) = layout.skip_dish_anchor_px;
            let sdext = layout.skip_dish_extents;
            point_lights.push(PointLight {
                pos: [sdx, sdy - sdext[2] * 0.20, sdext[1] * 5.0],
                radius: sdext[0] * 2.40,
                color: [1.00, 0.86, 0.50],
                intensity: 1.30 * skip_boost,
            });
        }

        frame.point_lights = point_lights;

        // ── Minimal 2D HUD ────────────────────────────────────────────
        // The 3D shrines + spotlight do the heavy lifting; the only text
        // that still pulls weight is each shrine's name, its target chip
        // count, and (for the boss) the rule the player needs to read
        // before committing. Everything else — score header, relic strip,
        // instruction strip, round-wind line, state stamps — has been
        // dropped to keep the eye on the shrines.
        let mut quads: Vec<GpuInstance> = Vec::new();
        let mut texts: Vec<TextLabel> = Vec::new();
        let mut buttons: Vec<ButtonDef> = Vec::new();

        // ── Per-shrine name labels (anchored to projected rects) ─────
        // Each blind label sits directly above its shrine using the
        // renderer's previous-frame projected screen rect. On the first
        // frame `projected_shrine_rects` is empty, so we fall back to
        // a screen-pixel estimate from the layout's pixel anchor and
        // extents — close enough for one frame.
        //
        // The boss shrine gets two extra lines: its rule description
        // (e.g. "Hand size -1") and its tier label (e.g. "[Soft]").
        // Other shrines just show their name.
        let title_h = typography::size(typography::HEADING, h, ui_scale) * 1.4;
        let desc_h = typography::size(typography::CAPTION, h, ui_scale) * 1.4;
        let base_target = pick.base_target;
        let upcoming_run_number = pick.run_number;
        for (i, &blind) in blinds.iter().enumerate() {
            // The upcoming shrine's label is replaced by the 3D plaque
            // below; skip it here to avoid redundancy.
            if i == upcoming_idx {
                continue;
            }
            let state = shrine_state(i, upcoming_idx);
            let title_color = match state {
                ShrineState::Upcoming => color::CHAMPAGNE,
                ShrineState::Cleared => color::SLATE,
                ShrineState::Future => color::MIST,
            };

            let label_w = (w * 0.22).clamp(180.0, 320.0);

            // Use projected shrine rect when available; fall back to
            // a small estimate around the pixel anchor on first frame.
            let projected = ctx.proj.shrine_rects.get(i).copied();
            let anchor_rect = projected.unwrap_or_else(|| {
                let (px, py) = layout.shrine_pixel_anchor(i);
                let ext = layout.shrine_extents(i);
                [px - ext[0] * 0.5, py - ext[1] * 0.5, ext[0], ext[1]]
            });

            let cx = anchor_rect[0] + anchor_rect[2] * 0.5;
            let label_x = (cx - label_w * 0.5).max(8.0).min(w - label_w - 8.0);

            // Boss shrine: stack name + description + tier above.
            // Other shrines: just the name.
            let title_text: String = if blind == BlindKind::Boss {
                pick.boss_name
                    .clone()
                    .unwrap_or_else(|| "Boss Blind".to_string())
            } else {
                blind.name().to_string()
            };

            let total_stack_h = if blind == BlindKind::Boss && pick.boss_kind.is_some() {
                title_h + desc_h * 2.0 + 4.0
            } else {
                title_h
            };
            // Anchor the stack so its BOTTOM sits just above the shrine
            // top (anchor_rect.y minus a small gap). Clamp to screen.
            let stack_bottom_y = (anchor_rect[1] - 8.0).max(total_stack_h + 8.0);
            let title_y = (stack_bottom_y - total_stack_h).max(8.0);

            texts.push(TextLabel {
                rect: [label_x, title_y, label_w, title_h],
                text: title_text,
                color: title_color,
                ..Default::default()
            });

            if blind == BlindKind::Boss
                && let (Some(description), Some(tier_label)) =
                    (pick.boss_description.as_deref(), pick.boss_tier_label)
            {
                texts.push(TextLabel {
                    rect: [label_x, title_y + title_h + 2.0, label_w, desc_h],
                    text: description.to_string(),
                    color: color::AMBER,
                    ..Default::default()
                });
                texts.push(TextLabel {
                    rect: [label_x, title_y + title_h + desc_h + 4.0, label_w, desc_h],
                    text: format!("[{}]", tier_label),
                    color: color::AMBER,
                    ..Default::default()
                });
            }
        }

        // ── Plaque above the upcoming shrine ─────────────────────────
        // Replaces the old bottom info panel: a 3D wood plaque floats
        // above the lit shrine with the blind name + ante on the top
        // line and target / reward / gold on the bottom line.
        {
            let (shrine_px, shrine_py) = layout.shrine_anchors_px[upcoming_idx];
            let shrine_ext = layout.shrine_extents[upcoming_idx];
            // Camera sits at h*1.25 depth so the plaque needs to be
            // much larger than a typical close-up plaque to read well.
            let plaque_w = (w * 0.70).clamp(560.0, 1000.0);
            let plaque_h = (h * 0.26).clamp(190.0, 300.0);
            let (raw_plaque_px, plaque_py, plaque_world_y) =
                upcoming_plaque_anchor(upcoming, shrine_px, shrine_py, shrine_ext, plaque_h);
            let plaque_px = raw_plaque_px.clamp(plaque_w * 0.5 + 16.0, w - plaque_w * 0.5 - 16.0);

            let blind_name = if upcoming == BlindKind::Boss {
                pick.boss_name
                    .clone()
                    .unwrap_or_else(|| "Boss Blind".to_string())
            } else {
                upcoming.name().to_string()
            };
            let target_value = base_target.saturating_mul(upcoming_run_number);

            let cam_rot = glam::Mat4::from_rotation_x((-60.0_f32).to_radians())
                * camera_facing_rotation(layout.camera.eye, layout.camera.target);
            frame.object3d(Object3d {
                pos: [plaque_px, plaque_py, plaque_world_y],
                extents: [plaque_w, plaque_h, 10.0],
                rotation: cam_rot,
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::Primitive {
                    shape: crate::render::primitive::MeshId::BeveledSlab,
                    material: crate::render::primitive::MaterialSpec::lacquered_wood_flat()
                        .with_decal(crate::render::primitive::plaque_decal({
                            // Stake badge: Spring is the baseline — no badge
                            // so the plaque stays clean for new players.
                            // Higher stakes print a trailing tag.
                            let stake_tag = match ctx.run.mode.stake {
                                crate::core::stake::Stake::Spring => String::new(),
                                other => format!("   ·   {}", other.label()),
                            };
                            format!(
                                "ANTE {}/{} · {}{}\nTarget {}   ·   Reward ${}",
                                pick.ante,
                                crate::game::run::FINAL_ANTE,
                                blind_name,
                                stake_tag,
                                target_value,
                                upcoming.clear_reward(),
                            )
                        })),
                    pick_id: None,
                    shadow_caster: false,
                    silhouette: false,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });

            // Boss blinds get an ofuda beside the plaque showing the rule
            // description + tier — the plaque's two lines are already full.
            if upcoming == BlindKind::Boss
                && let (Some(kind), Some(description)) =
                    (pick.boss_kind, pick.boss_description.as_deref())
            {
                let def = kind.def();
                let (ofuda_w, ofuda_h) =
                    auto_size_shrine_ofuda(plaque_w, plaque_h, def.name, description);
                let ofuda_gap = (plaque_w * 0.06).clamp(56.0, 84.0);
                // Position to the left of the plaque, but keep it tucked
                // close enough to share the plaque accent light. The gap
                // has to clear the plaque side in perspective. The lit-mesh
                // pipeline renders both front and back faces, so if these
                // props sit nearly on the same depth band the plaque edge can
                // win a few samples and visibly punch through the paper.
                let ofuda_px = (plaque_px - plaque_w * 0.5 - ofuda_w * 0.5 - ofuda_gap)
                    .max(ofuda_w * 0.5 + 8.0);
                // Pull the paper a touch toward the camera as a depth bias
                // so it no longer shares the plaque face's sample range.
                let ofuda_py = plaque_py + shrine_ext[2] * 0.15 + 24.0;
                // Float the ofuda slightly higher than the plaque's nominal
                // lift so its front face clearly sits in front of anything
                // behind it (shrine plinths) and doesn't co-planar-fight.
                let ofuda_world_y = plaque_world_y * 0.86 + 6.0;
                frame.object3d(Object3d {
                    pos: [ofuda_px, ofuda_py, ofuda_world_y],
                    extents: [ofuda_w, ofuda_h, 3.0],
                    rotation: cam_rot,
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::Primitive {
                        shape: crate::render::primitive::MeshId::Ofuda,
                        material: crate::render::primitive::MaterialSpec::plain().with_decal(
                            crate::render::primitive::DecalSpec {
                                text: format!("{}\n{}", def.name, description),
                                layout: crate::render::primitive::DecalLayout::TitleRule {
                                    target_short_edge: crate::render::decal::OFUDA_DECAL_LONG_EDGE,
                                },
                            },
                        ),
                        pick_id: None,
                        shadow_caster: false,
                        silhouette: false,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
            }
        }

        // ── Caption labels above the 3D altars ───────────────────────
        // We anchor the labels to the *projected* screen rect of each
        // altar (computed by the renderer the previous frame) so the
        // text always sits above the visible dish, regardless of camera
        // angle or window size. The renderer pushes a (pick_id, rect)
        // entry into `aux_dish_rects` for each `DishExplicit`; we look
        // up our PICK_PLAY_DISH / PICK_SKIP_DISH entries here.
        //
        // On the very first frame `aux_dish_rects` is empty, so we fall
        // back to a screen-pixel estimate derived from the layout's raw
        // pixel anchor — close enough that the labels still look
        // reasonable for the one frame before projection kicks in.
        let play_focused_label = self.play_focused();
        let skip_focused_label = self.skip_focused();

        let projected_play = ctx
            .proj
            .aux_dish_rects
            .iter()
            .find_map(|(pid, r)| (*pid == Some(PICK_PLAY_DISH)).then_some(*r));
        let projected_skip = ctx
            .proj
            .aux_dish_rects
            .iter()
            .find_map(|(pid, r)| (*pid == Some(PICK_SKIP_DISH)).then_some(*r));

        // Cache for next frame's update() and flat_items() — must come
        // BEFORE the labels are pushed so the cached value reflects this
        // frame's projection.
        if let Some(r) = projected_play {
            self.last_play_rect.set(Some(r));
        }
        if let Some(r) = projected_skip {
            self.last_skip_rect.set(Some(r));
        }

        let altar_label_w = (w * 0.28).clamp(240.0, 420.0);
        let altar_label_h = typography::size(typography::HEADING, h, ui_scale) * 2.4;
        let altar_caption_h = typography::size(typography::CAPTION, h, ui_scale) * 2.4;

        // Helper: stack a two-line label (title + caption) above a
        // projected screen rect, clamped to the window bounds.
        let push_altar_caption = |rect: [f32; 4],
                                  title: &str,
                                  caption: &str,
                                  focused: bool,
                                  no_glossary: bool,
                                  texts: &mut Vec<TextLabel>| {
            let cx = rect[0] + rect[2] * 0.5;
            let lx = (cx - altar_label_w * 0.5)
                .max(8.0)
                .min(w - altar_label_w - 8.0);
            // Stack the two lines below the projected dish bottom so they
            // aren't occluded by the shrine base geometry.
            let stack_h = altar_label_h + altar_caption_h + 2.0;
            let ly = (rect[1] + rect[3] + 30.0).min(h - stack_h - 8.0);
            texts.push(TextLabel {
                rect: [lx, ly, altar_label_w, altar_label_h],
                text: title.to_string(),
                color: if focused {
                    color::CHAMPAGNE
                } else {
                    color::PARCHMENT
                },
                no_glossary,
                ..Default::default()
            });
            texts.push(TextLabel {
                rect: [lx, ly + altar_label_h + 2.0, altar_label_w, altar_caption_h],
                text: caption.to_string(),
                color: if focused { color::GOLD } else { color::MIST },
                no_glossary,
                ..Default::default()
            });
        };

        // Play altar caption — projected rect from this frame, or
        // (first-frame fallback) a small estimate around the pixel anchor.
        let play_anchor_rect = projected_play.unwrap_or_else(|| {
            let (pdx, pdy) = layout.play_dish_anchor_px;
            let est_w = layout.play_dish_extents[0] * 0.8;
            let est_h = layout.play_dish_extents[2] * 0.8;
            [pdx - est_w * 0.5, pdy - est_h * 0.5, est_w, est_h]
        });
        push_altar_caption(
            play_anchor_rect,
            "Play",
            "Begin · Enter",
            play_focused_label,
            true,
            &mut texts,
        );

        let skip_anchor_rect = projected_skip.unwrap_or_else(|| {
            let (sdx, sdy) = layout.skip_dish_anchor_px;
            let est_w = layout.skip_dish_extents[0] * 0.8;
            let est_h = layout.skip_dish_extents[2] * 0.8;
            [sdx - est_w * 0.5, sdy - est_h * 0.5, est_w, est_h]
        });
        if can_skip {
            if let Some(tag) = skip_tag {
                // Three-line stack: "Skip" (heading), tag name, tag description.
                let cx = skip_anchor_rect[0] + skip_anchor_rect[2] * 0.5;
                let lx = (cx - altar_label_w * 0.5)
                    .max(8.0)
                    .min(w - altar_label_w - 8.0);
                let stack_h = altar_label_h + altar_caption_h * 2.0 + 4.0;
                let ly = (skip_anchor_rect[1] + skip_anchor_rect[3] + 30.0).min(h - stack_h - 8.0);
                let title_color = if skip_focused_label {
                    color::CHAMPAGNE
                } else {
                    color::PARCHMENT
                };
                let sub_color = if skip_focused_label {
                    color::GOLD
                } else {
                    color::MIST
                };
                texts.push(TextLabel {
                    rect: [lx, ly, altar_label_w, altar_label_h],
                    text: "Skip".to_string(),
                    color: title_color,
                    ..Default::default()
                });
                texts.push(TextLabel {
                    rect: [lx, ly + altar_label_h + 2.0, altar_label_w, altar_caption_h],
                    text: tag.name().to_string(),
                    color: sub_color,
                    ..Default::default()
                });
                texts.push(TextLabel {
                    rect: [
                        lx,
                        ly + altar_label_h + altar_caption_h + 4.0,
                        altar_label_w,
                        altar_caption_h,
                    ],
                    text: tag.description().to_string(),
                    color: sub_color,
                    ..Default::default()
                });
            } else {
                push_altar_caption(
                    skip_anchor_rect,
                    "Skip",
                    "Tribute · Esc",
                    skip_focused_label,
                    false,
                    &mut texts,
                );
            }
        }

        let scale = metrics::scene_scale(w, h, ui_scale);

        // ── Gold outline around the focused altar ────────────────────
        // A chunky gold border (3× the normal focus ring thickness)
        // around whichever altar is currently selected so the player
        // can immediately read which action they're about to confirm.
        let big_ring_scale = scale * 3.0;
        if play_focused_label {
            push_focus_ring(play_anchor_rect, big_ring_scale, w, h, &mut quads);
        }
        if skip_focused_label && can_skip {
            push_focus_ring(skip_anchor_rect, big_ring_scale, w, h, &mut quads);
        }

        // Register focus-tree click targets for PlayBlind + SkipBlind.
        let items = Self::flat_items(
            &layout,
            upcoming,
            can_skip,
            self.last_play_rect.get(),
            self.last_skip_rect.get(),
        );
        self.tree.register_flat_buttons(&items, &mut buttons);

        // Pause menu overlay. Drop scene buttons while paused so the
        // pause menu's own buttons are the only clickable surfaces.
        if self.pause_menu.paused {
            buttons.clear();
        }
        self.pause_menu.draw(
            crate::ui::layout::ViewportCtx {
                window_w: w,
                window_h: h,
                ui_scale,
            },
            scale,
            &mut quads,
            &mut texts,
            &mut buttons,
        );
        if self.pause_menu.paused {
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }

        // Volumetric smoke pass — pushed unconditionally after every 3D
        // scene object (shrines, dishes, coin piles) so the smoke draws
        // *over* them, mirroring the shop scene. Previously the marker
        // was only emitted inside the `transition_at` block below, which
        // meant the renderer's pass split saw `split_idx = None` on every
        // idle pick-blind frame and skipped the volume pass entirely —
        // the fluid sim was still simmering full of density carried over
        // from gameplay, but it was never drawn. The transition burst
        // still works because it pumps wind impulses into the same
        // marker; it just no longer has to push the marker itself.
        frame.fluid_smoke();

        // Push 2D layers + metadata onto the frame.
        frame.quads(quads);
        frame.texts(texts);

        frame.buttons = buttons;
        frame.window_title = "Mahjuro".to_string();

        frame
    }
}
