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

use crate::core::rules::BlindKind;
use crate::render::draw_cmd::{
    CameraParams, CoinPlacement, DishExplicit, ShrinePlacement, UiFrame,
};
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::widget::{self, PanelVariant};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::gameplay::GameplayScene;
use super::pause_menu::PauseMenu;
use super::{
    BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneDrawOutput, SceneTransition,
    UpdateCtx,
};

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
    /// Bottom info-panel rect in screen pixels: shows the upcoming
    /// blind's name, target, reward, and (boss-only) rule description.
    info_panel_rect: [f32; 4],
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
            eye: [0.0, h * 0.50, h * 1.25],
            target: [0.0, h * 0.18, -h * 0.05],
            up: [0.0, 1.0, 0.0],
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

        // ── Bottom info panel (text only — no buttons; Play and Skip
        // are 3D objects on the floor in front of the shrines).
        let panel_pad = 16.0_f32;
        let panel_w = (w * 0.86).min(1100.0);
        let panel_h = (h * 0.18).clamp(120.0, 200.0);
        let panel_x = (w - panel_w) * 0.5;
        let panel_y = h - panel_h - panel_pad;
        let info_panel_rect = [panel_x, panel_y, panel_w, panel_h];

        // ── 3D Play / Skip altars FLANKING the upcoming shrine ──────
        // The two altars sit immediately to the left and right of the
        // currently-upcoming shrine on the floor plane, so the eye can
        // read "this shrine, with these two choices beside it" as one
        // compositional unit. They move with the upcoming shrine —
        // when the player advances to Big Blind they'll appear flanking
        // the middle shrine, etc. They're pulled slightly forward in
        // pixel-y to sit on the visible floor pool in front of the
        // shrine row, giving a clear triangular grouping.
        let (up_px, _up_py) = shrine_anchors_px[upcoming_idx];
        let up_ext = shrine_extents[upcoming_idx];
        let altar_row_y = row_y + h * 0.08;
        // Horizontal offset: half the upcoming shrine's footprint plus
        // half the altar dish width plus a small gap, so the altars
        // sit visibly outside the shrine's silhouette.
        // Brass altar dishes — small offering trays (~9mm rim height).
        // Width and depth stay layout-relative so they scale with the
        // shrine row at any resolution.
        let play_dish_extents = [h * 0.12, layout.mm(9.0), h * 0.06];
        let skip_dish_extents = [h * 0.10, layout.mm(8.0), h * 0.055];
        let gap = h * 0.04;
        let play_offset = up_ext[0] * 0.55 + play_dish_extents[0] * 0.6 + gap;
        let skip_offset = up_ext[0] * 0.55 + skip_dish_extents[0] * 0.6 + gap;
        // Clamp altar X positions so neither runs off-screen for the
        // boss shrine (which sits at w*0.80).
        let play_x = (up_px - play_offset)
            .max(play_dish_extents[0] * 0.5 + 16.0)
            .min(w - play_dish_extents[0] * 0.5 - 16.0);
        let skip_x = (up_px + skip_offset)
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
            info_panel_rect,
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
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            return t;
        }

        let upcoming = ctx.run.upcoming_blind;
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
            },
        );

        for a in ctx.actions {
            if matches!(a, UiAction::Cancel) && can_skip {
                let reward = upcoming.skip_reward();
                ctx.run.gold = ctx.run.gold.saturating_add(reward);
                ctx.run.skip_to_next_blind();
                return Some(Scene::PickBlind(PickBlindScene::new()));
            }
        }

        match action {
            Some(BlindAction::SkipBlind) if can_skip => {
                let reward = upcoming.skip_reward();
                ctx.run.gold = ctx.run.gold.saturating_add(reward);
                ctx.run.skip_to_next_blind();
                Some(Scene::PickBlind(PickBlindScene::new()))
            }
            Some(BlindAction::PlayBlind) | Some(BlindAction::SkipBlind) => {
                ctx.run.apply_blind(upcoming);
                Some(Scene::Gameplay(GameplayScene::new()))
            }
            None => None,
        }
    }

    fn draw(&self, _ctx: DrawCtx<'_>) -> SceneDrawOutput {
        // Legacy fallback — the canonical path is `draw_frame()` below.
        SceneDrawOutput::default()
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;

        let upcoming = ctx.run.upcoming_blind;
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
        frame.camera_override = Some(layout.camera);

        // ── 3D shrines ────────────────────────────────────────────────
        let mut shrine_placements: Vec<ShrinePlacement> = Vec::with_capacity(3);
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
                (ShrineState::Upcoming, BlindKind::Boss) => ctx
                    .run
                    .upcoming_boss
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

            shrine_placements.push(ShrinePlacement {
                world_pos: [px, py, 0.0],
                extents,
                color: base_color,
                glow,
            });
        }
        frame.shrine_batch(shrine_placements);

        // ── Floor plane under the shrines ────────────────────────────
        // A wide flat slab the shrines stand on, drawn via a giant
        // `DishExplicit` (low rim height) so it reuses the existing
        // dish mesh + render path. The plane catches the spotlight
        // pool and grounds the shrines.
        let (fx, fy) = layout.floor_anchor_px;
        frame.dish_explicit(DishExplicit {
            center_pos: [fx, fy, 0.0],
            extents: layout.floor_extents,
            pick_id: None,
        });

        // ── Play + Skip altars on the floor in front of the shrines ─
        // Two small dishes side by side. Play is on the LEFT and holds
        // a single big golden coin (the "ritual offering" — make it to
        // begin). Skip is on the RIGHT and holds a small pile of coins
        // (the tribute reward for walking away). Both reuse the shop's
        // existing dish + coin meshes.
        let skip_reward = upcoming.skip_reward();
        let (play_px, play_py) = layout.play_dish_anchor_px;
        let play_dext = layout.play_dish_extents;
        frame.dish_explicit(DishExplicit {
            center_pos: [play_px, play_py, 0.0],
            extents: play_dext,
            pick_id: Some(PICK_PLAY_DISH),
        });
        // Single large golden coin in the center of the play dish.
        let play_dish_top_y = play_dext[1] + 2.0;
        frame.coin_batch(vec![CoinPlacement {
            world_pos: [play_px, play_py, play_dish_top_y],
            rotation_y: 0.4,
            radius: 14.0,
            thickness: 5.5,
            color: [1.00, 0.84, 0.30, 1.0],
        }]);

        if can_skip {
            let (skip_px, skip_py) = layout.skip_dish_anchor_px;
            let skip_dext = layout.skip_dish_extents;
            frame.dish_explicit(DishExplicit {
                center_pos: [skip_px, skip_py, 0.0],
                extents: skip_dext,
                pick_id: Some(PICK_SKIP_DISH),
            });
            // Small spread of coins matching the reward (capped at 5).
            let mut coins: Vec<CoinPlacement> = Vec::new();
            let n_coins = (skip_reward as usize).clamp(1, 5);
            let coin_radius = 7.0_f32;
            let coin_thickness = 3.0_f32;
            let dish_top_y = skip_dext[1] + 2.0;
            for i in 0..n_coins {
                let t = if n_coins <= 1 {
                    0.0
                } else {
                    (i as f32 / (n_coins as f32 - 1.0)) - 0.5
                };
                let off_x = t * skip_dext[0] * 0.45;
                let off_z = ((i % 2) as f32 - 0.5) * 6.0;
                coins.push(CoinPlacement {
                    world_pos: [skip_px + off_x, skip_py + off_z, dish_top_y],
                    rotation_y: (i as f32) * 0.7,
                    radius: coin_radius,
                    thickness: coin_thickness,
                    color: [1.00, 0.78, 0.30, 1.0],
                });
            }
            frame.coin_batch(coins);
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
        let title_h = typography::size(typography::HEADING, h) * 1.4;
        let desc_h = typography::size(typography::CAPTION, h) * 1.4;
        let base_target = ctx.run.base_target;
        for (i, &blind) in blinds.iter().enumerate() {
            let state = shrine_state(i, upcoming_idx);
            let title_color = match state {
                ShrineState::Upcoming => color::CHAMPAGNE,
                ShrineState::Cleared => color::SLATE,
                ShrineState::Future => color::MIST,
            };

            let label_w = (w * 0.22).clamp(180.0, 320.0);

            // Use projected shrine rect when available; fall back to
            // a small estimate around the pixel anchor on first frame.
            let projected = ctx.projected_shrine_rects.get(i).copied();
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
                ctx.run
                    .upcoming_boss
                    .map(|k| k.def().name.to_string())
                    .unwrap_or_else(|| "Boss Blind".to_string())
            } else {
                blind.name().to_string()
            };

            let total_stack_h = if blind == BlindKind::Boss && ctx.run.upcoming_boss.is_some() {
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

            if blind == BlindKind::Boss {
                if let Some(kind) = ctx.run.upcoming_boss {
                    let def = kind.def();
                    // Reactive bosses (Mirror, Tax Collector) override
                    // the static description with the variant chosen
                    // at reveal time, so the player sees the actual
                    // rule before they ever fight it.
                    let description: &str = ctx
                        .run
                        .upcoming_boss_effect
                        .as_ref()
                        .and_then(|e| e.description_override.as_deref())
                        .unwrap_or(def.description);
                    texts.push(TextLabel {
                        rect: [label_x, title_y + title_h + 2.0, label_w, desc_h],
                        text: description.to_string(),
                        color: color::AMBER,
                        ..Default::default()
                    });
                    texts.push(TextLabel {
                        rect: [label_x, title_y + title_h + desc_h + 4.0, label_w, desc_h],
                        text: format!("[{}]", def.tier.label()),
                        color: color::AMBER,
                        ..Default::default()
                    });
                }
            }
        }

        // ── Bottom info panel ─────────────────────────────────────────
        // Shows the upcoming blind's full details: name, target chip
        // count, gold reward, and (boss-only) the rule description +
        // tier label. The buttons live in the bottom row of this panel.
        let panel_rect = layout.info_panel_rect;
        widget::push_panel(&mut quads, panel_rect, PanelVariant::Hero);

        let panel_pad = 16.0_f32;
        let panel_inner_x = panel_rect[0] + panel_pad;
        let panel_inner_y = panel_rect[1] + panel_pad;
        let panel_inner_w = panel_rect[2] - panel_pad * 2.0;
        let title_h = typography::size(typography::HEADING, h) * 1.6;
        let line_h = typography::size(typography::CAPTION, h) * 1.6;

        // Title: upcoming blind name + ante header
        let title_text = if upcoming == BlindKind::Boss {
            ctx.run
                .upcoming_boss
                .map(|k| k.def().name.to_string())
                .unwrap_or_else(|| "Boss Blind".to_string())
        } else {
            upcoming.name().to_string()
        };
        texts.push(TextLabel {
            rect: [panel_inner_x, panel_inner_y, panel_inner_w, title_h],
            text: format!(
                "ANTE {}/{} · {}",
                ctx.run.ante,
                crate::game::run::FINAL_ANTE,
                title_text
            ),
            color: color::CHAMPAGNE,
            ..Default::default()
        });

        // Target + reward summary
        let target_value = (base_target as f32 * upcoming.target_multiplier()) as u32;
        let summary_y = panel_inner_y + title_h + 6.0;
        texts.push(TextLabel {
            rect: [panel_inner_x, summary_y, panel_inner_w, line_h],
            text: format!(
                "Target {}   ·   Reward ${}   ·   Gold {}",
                target_value,
                upcoming.clear_reward(),
                ctx.run.gold,
            ),
            color: color::PARCHMENT,
            ..Default::default()
        });

        // Boss-only rule description + tier on the next line.
        if upcoming == BlindKind::Boss {
            if let Some(kind) = ctx.run.upcoming_boss {
                let def = kind.def();
                let description: &str = ctx
                    .run
                    .upcoming_boss_effect
                    .as_ref()
                    .and_then(|e| e.description_override.as_deref())
                    .unwrap_or(def.description);
                let desc_y = summary_y + line_h + 4.0;
                texts.push(TextLabel {
                    rect: [panel_inner_x, desc_y, panel_inner_w, line_h],
                    text: format!("{}   [{}]", description, def.tier.label()),
                    color: color::AMBER,
                    ..Default::default()
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
            .aux_dish_rects
            .iter()
            .find_map(|(pid, r)| (*pid == Some(PICK_PLAY_DISH)).then_some(*r));
        let projected_skip = ctx
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

        let altar_label_w = (w * 0.16).clamp(160.0, 240.0);
        let altar_label_h = typography::size(typography::HEADING, h) * 1.4;
        let altar_caption_h = typography::size(typography::CAPTION, h) * 1.4;

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
            // Stack the two lines just above the projected dish top.
            let stack_h = altar_label_h + altar_caption_h + 2.0;
            let ly = (rect[1] - stack_h - 6.0).max(8.0);
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

        if can_skip {
            let skip_anchor_rect = projected_skip.unwrap_or_else(|| {
                let (sdx, sdy) = layout.skip_dish_anchor_px;
                let est_w = layout.skip_dish_extents[0] * 0.8;
                let est_h = layout.skip_dish_extents[2] * 0.8;
                [sdx - est_w * 0.5, sdy - est_h * 0.5, est_w, est_h]
            });
            push_altar_caption(
                skip_anchor_rect,
                &format!("Skip  +{}g", skip_reward),
                "Tribute · Esc",
                skip_focused_label,
                false,
                &mut texts,
            );
        }

        let scale = (w.min(h)) / 600.0;

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
        self.pause_menu
            .draw(w, h, scale, &mut quads, &mut texts, &mut buttons);
        if self.pause_menu.paused {
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }

        // Volumetric smoke pass — pushed unconditionally after every 3D
        // scene object (shrines, dishes, coin piles) so the smoke draws
        // *over* them, mirroring the shop scene. Previously the marker
        // was only emitted inside the `transition_at` block below, which
        // meant the renderer's pass split saw `split_idx = None` on every
        // idle pick-blind frame and skipped the volume pass entirely —
        // the fluid sim was still simmering full of density from
        // gameplay candle plumes, but it was never drawn. The transition
        // burst still works because it pumps wind impulses into the same
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
