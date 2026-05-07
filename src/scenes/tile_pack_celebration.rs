//! Tile-pack purchase celebration as a pushdown overlay (same stack pattern as
//! [`super::ZodiacCelebrationScene`]): black stage, shop camera, pack mesh /
//! flying tiles — shop underneath stays suspended until the player dismisses.

use glam::Mat4;

use crate::core::tile_pack::PACK_TILE_ID_BASE;
use crate::core::tile_pack::TilePackKind;
use crate::game::engine::GameEngine;
use crate::render::draw_cmd::{DrawCmd, Object3d, Object3dKind, ShowcaseTilePlacement, UiFrame};
use crate::render::table_transform::mat4_to_euler_xyz_rad;
use crate::render::wgpu_renderer::PointLight;
use crate::scenes::celebration_overlay;
use crate::scenes::shop::{
    CelebPhase, PackCelebration, ShopInventoryCounts, ShopLayout, shop_celebration_camera,
};
use crate::ui::input::UiAction;
use crate::ui::scene_layout::{ShopPositions, load_shop_positions};

use super::{
    BackgroundId, ButtonDef, DrawCtx, OverlayRequest, SceneBehavior, SceneTransition, UpdateCtx,
};

pub struct TilePackCelebrationScene {
    celebration: PackCelebration,
    pub(crate) positions: ShopPositions,
    inventory: ShopInventoryCounts,
}

impl TilePackCelebrationScene {
    pub fn new(celebration: PackCelebration, inventory: ShopInventoryCounts) -> Self {
        Self {
            celebration,
            positions: load_shop_positions(),
            inventory,
        }
    }

    /// Headless screenshot: settled reveal row (legacy counts-from-run helper).
    #[allow(dead_code)]
    pub fn new_headless_screenshot(
        run: &crate::game::run::RunState,
        pack_kind: TilePackKind,
    ) -> Self {
        let tiles = pack_kind.generate_tiles(PACK_TILE_ID_BASE);
        let shop_rm = GameEngine::read_shop(run);
        let inventory = ShopInventoryCounts {
            n_for_sale: 0,
            n_for_sale_zodiacs: 0,
            n_for_sale_talismans: 0,
            n_owned_relics: shop_rm.owned_relics.len(),
        };
        Self::new(
            PackCelebration::screenshot_reveal_settled(tiles, pack_kind.name(), pack_kind),
            inventory,
        )
    }

    /// Match counts used by the storeroom [`ShopLayout`] builder when the shop exists (CLI warmup).
    pub fn new_headless_with_shop_counts(
        _run: &crate::game::run::RunState,
        pack_kind: TilePackKind,
        counts: ShopInventoryCounts,
    ) -> Self {
        let tiles = pack_kind.generate_tiles(PACK_TILE_ID_BASE);
        Self::new(
            PackCelebration::screenshot_reveal_settled(tiles, pack_kind.name(), pack_kind),
            counts,
        )
    }

    fn push_celebration_draw(&self, frame: &mut UiFrame, layout: &ShopLayout, w: f32, h: f32) {
        let celeb = &self.celebration;
        let n = celeb.tiles.len();
        celebration_overlay::CelebrationOverlayScratch::new(w, h)
            .push_dimmer_then_depth_reset(frame);
        frame.text(celebration_overlay::label_pack_title(
            h,
            w,
            celeb.pack_name.to_string(),
        ));

        match celeb.phase {
            CelebPhase::Closeup => {
                let box_h = h * 0.28;
                let box_w = box_h * crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
                let box_d = box_h * 0.10;
                let t = celeb.started_at.elapsed().as_secs_f32();
                let bob_x = (t * 0.7).sin() * h * 0.008;
                let bob_y = (t * 0.5).sin() * h * 0.006;
                let bob_rx = (t * 0.6).sin() * 2.5;
                let bob_ry = (t * 0.8).cos() * 3.0;
                frame.object3d_batch(vec![Object3d {
                    pos: [
                        w * 0.5 + bob_x,
                        h * self.positions.celeb_pack_closeup.ny + bob_y,
                        layout.mm(self.positions.celeb_pack_closeup.lift_mm) + box_h * 0.5,
                    ],
                    extents: [box_w, box_d, box_h],
                    rotation: mat4_to_euler_xyz_rad(
                        Mat4::from_rotation_y(bob_ry.to_radians())
                            * Mat4::from_rotation_x(bob_rx.to_radians()),
                    ),
                    color: celeb.pack_kind.foil_tint(),
                    kind: Object3dKind::Pack {
                        kind: celeb.pack_kind,
                        pick_id: None,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: Some("shop.celebrations.pack_closeup"),
                }]);

                frame.text(celebration_overlay::label_confirm_to_open(h, w, t));
            }
            CelebPhase::Reveal => {
                // Strong perspective tilt enlarges the face on screen — keep height and row width tight.
                let gap_ratio = 0.22_f32;
                let row_units = n as f32 * (1.0 + gap_ratio) - gap_ratio;
                let tile_size = (h * 0.048).min((w * 0.48) / row_units.max(1.0));
                let gap = tile_size * gap_ratio;
                let total_w = n as f32 * tile_size + (n.saturating_sub(1)) as f32 * gap;
                let row_x0 = (w - total_w) * 0.5 + w * self.positions.celeb_pack_reveal.nx;
                let row_py = h * self.positions.celeb_pack_reveal.ny;
                let row_lift = layout.mm(self.positions.celeb_pack_reveal.lift_mm);
                let src_px = w * 0.5;
                let src_py = h * self.positions.celeb_pack_closeup.ny;
                let src_lift = row_lift + h * 0.15;
                let row_rx =
                    self.positions.celeb_pack_reveal.rx_deg.to_radians() + 32.0_f32.to_radians();
                let row_ry = self.positions.celeb_pack_reveal.ry_deg.to_radians();
                let row_rz =
                    self.positions.celeb_pack_reveal.rz_deg.to_radians() + std::f32::consts::PI;

                let mut placements = Vec::with_capacity(n);
                for i in 0..n {
                    let t = celeb.tile_progress(i);
                    let ease = 1.0 - (1.0_f32 - t).powi(3);

                    let dest_px = row_x0 + i as f32 * (tile_size + gap) + tile_size * 0.5;
                    let px = src_px + (dest_px - src_px) * ease;
                    let py = src_py + (row_py - src_py) * ease;
                    let lift = src_lift + (row_lift - src_lift) * ease;
                    let scale = 0.3 + 0.7 * ease;

                    placements.push(ShowcaseTilePlacement {
                        tile: celeb.tiles[i],
                        center_pos: [px, py, lift],
                        rotation: [row_rx, row_ry, row_rz],
                        scale,
                        size_px: tile_size,
                        brightness: 0.82,
                        selected: false,
                        hovered: false,
                        outline: false,
                        glow: false,
                        glow_color: None,
                        pick_id: None,
                    });
                }

                frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements));

                if celeb.fully_settled() {
                    let elapsed = celeb.elapsed();
                    frame.text(celebration_overlay::label_confirm_to_continue(
                        h, w, elapsed, 1.0,
                    ));
                }
            }
        }
    }
}

fn pack_celebration_isolation_lights(
    w: f32,
    h: f32,
    layout: &ShopLayout,
    positions: &ShopPositions,
) -> Vec<PointLight> {
    let cx = w * 0.5 + w * positions.celeb_pack_reveal.nx;
    let row_py = h * positions.celeb_pack_reveal.ny;
    let lift = layout.mm(positions.celeb_pack_reveal.lift_mm);
    vec![
        PointLight {
            pos: [cx + w * 0.14, row_py - h * 0.22, lift + h * 0.48],
            radius: h * 3.2,
            color: [1.0, 0.93, 0.78],
            intensity: 1.55,
        },
        PointLight {
            pos: [cx - w * 0.16, row_py + h * 0.06, lift + h * 0.32],
            radius: h * 2.8,
            color: [0.70, 0.82, 1.0],
            intensity: 1.05,
        },
        PointLight {
            pos: [cx, row_py - h * 0.38, lift + h * 0.62],
            radius: h * 2.6,
            color: [1.0, 0.97, 0.90],
            intensity: 1.25,
        },
    ]
}

impl SceneBehavior for TilePackCelebrationScene {
    fn has_blocking_overlay(&self) -> bool {
        true
    }

    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if ctx.headless {
            if matches!(self.celebration.phase, CelebPhase::Closeup) {
                self.celebration.phase = CelebPhase::Reveal;
                self.celebration.started_at = std::time::Instant::now();
            }
        }

        let has_input = ctx.actions.iter().any(|a| {
            matches!(
                a,
                UiAction::Confirm | UiAction::Cancel | UiAction::CommitDiscard
            )
        }) || !ctx.button_clicks.is_empty();

        match self.celebration.phase {
            CelebPhase::Closeup => {
                if has_input {
                    self.celebration.phase = CelebPhase::Reveal;
                    self.celebration.started_at = std::time::Instant::now();
                    ctx.bus.push(crate::game::event_bus::GameEvent::PackOpened);
                }
            }
            CelebPhase::Reveal => {
                let n = self.celebration.tiles.len();
                while self.celebration.revealed_count < n
                    && self
                        .celebration
                        .tile_progress(self.celebration.revealed_count)
                        > 0.0
                {
                    ctx.bus
                        .push(crate::game::event_bus::GameEvent::PackTileRevealed);
                    self.celebration.revealed_count += 1;
                }
                let dominated = self.celebration.fully_settled() || self.celebration.dismissed;
                if dominated && has_input {
                    ctx.run.pending_shop_focus_snap_after_pack_celebration = true;
                    *ctx.overlay_request = Some(OverlayRequest::Pop);
                }
            }
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let env_h = ctx.shop_env_height_scale;

        debug_assert!(
            !self.celebration.tiles.is_empty(),
            "pack celebration needs tiles"
        );

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        let layout = ShopLayout::build(ctx.layout, &self.positions, self.inventory.clone());
        let cam = shop_celebration_camera(w, h, env_h);
        frame.camera_override = Some(cam);
        frame.shop_env_gltf_punctual = false;
        frame.point_lights = pack_celebration_isolation_lights(w, h, &layout, &self.positions);
        self.push_celebration_draw(&mut frame, &layout, w, h);
        frame.window_title = "Mahjuro".to_string();

        let allow_click = match self.celebration.phase {
            CelebPhase::Closeup => true,
            CelebPhase::Reveal => self.celebration.fully_settled() || self.celebration.dismissed,
        };
        if allow_click {
            frame.buttons = vec![ButtonDef::scene((0.0, 0.0, w, h), u32::MAX)];
        }
        frame
    }
}
