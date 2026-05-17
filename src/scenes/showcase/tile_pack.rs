//! Tile-pack purchase flow on the showcase overlay.

use glam::{Mat4, Vec3};

use crate::core::tile_pack::PACK_TILE_ID_BASE;
use crate::core::tile_pack::TilePackKind;
use crate::game::engine::GameEngine;
use crate::persistence::TilePreset;
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, Object3d, Object3dKind, ShowcaseRenderHints, ShowcaseTilePlacement,
    UiFrame,
};
use crate::render::showcase_tile_layout::{compute_pack_reveal_row_layout, PackRevealRowLayoutParams};
use crate::render::table_transform::mat4_to_euler_xyz_rad;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{PointLight, SpotLight};
use crate::render::world_space::world_on_camera_ray_plane_z;
use crate::scenes::celebration_overlay;
use crate::scenes::shop::{
    CelebPhase, PackCelebration, ShopInventoryCounts, ShopLayout, shop_celebration_camera,
};
use crate::ui::input::UiAction;
use crate::ui::scene_layout::{ShopPositions, load_shop_positions};

use super::super::{BackgroundId, ButtonDef, DrawCtx, OverlayRequest, SceneTransition, UpdateCtx};

pub struct TilePackPresenter {
    pub celebration: PackCelebration,
    pub positions: ShopPositions,
    inventory: ShopInventoryCounts,
}

impl TilePackPresenter {
    pub fn new(celebration: PackCelebration, inventory: ShopInventoryCounts) -> Self {
        Self {
            celebration,
            positions: load_shop_positions(),
            inventory,
        }
    }

    #[allow(dead_code)] // parity with legacy headless helper; CLI uses `new_headless_with_shop_counts`
    pub fn new_headless_screenshot(
        run: &crate::game::run::RunState,
        pack_kind: TilePackKind,
    ) -> Self {
        let tiles = pack_kind.generate_tiles(PACK_TILE_ID_BASE);
        let shop_rm = GameEngine::read_shop(run);
        let inventory = ShopInventoryCounts {
            n_for_sale: 0,
            n_for_sale_talismans: 0,
            n_owned_relics: shop_rm.owned_relics.len(),
        };
        Self::new(
            PackCelebration::screenshot_pack_closeup_headless(tiles, pack_kind.name(), pack_kind),
            inventory,
        )
    }

    pub fn new_headless_with_shop_counts(
        _run: &crate::game::run::RunState,
        pack_kind: TilePackKind,
        counts: ShopInventoryCounts,
    ) -> Self {
        let tiles = pack_kind.generate_tiles(PACK_TILE_ID_BASE);
        Self::new(
            PackCelebration::screenshot_pack_closeup_headless(tiles, pack_kind.name(), pack_kind),
            counts,
        )
    }

    pub fn render_hints() -> ShowcaseRenderHints {
        ShowcaseRenderHints {
            object3d_use_camera_ray_plane_z: true,
            showcase_tiles_use_camera_ray_plane_z: true,
            suppress_table_shadows: true,
            tile_pack_celebration_tonemap: true,
            shop_tonemap_and_lit_mesh_context: false,
            collection_tonemap_context: false,
        }
    }

    fn push_celebration_draw(
        &self,
        frame: &mut UiFrame,
        layout: &ShopLayout,
        w: f32,
        h: f32,
        cam: &CameraParams,
        tile_preset: TilePreset,
    ) {
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
                let box_h = h * 0.28;
                let box_w = box_h * crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
                let box_d = box_h * 0.10;

                // Keep the pack box visible while tiles fly out of it.
                // In Reveal, `started_at` is reset, so we use a smooth settle or just let it rest.
                // We'll let it rest at the base closeup position without the bobbing
                // so it looks like it settled down to open.
                frame.object3d_batch(vec![Object3d {
                    pos: [
                        w * 0.5,
                        h * self.positions.celeb_pack_closeup.ny,
                        layout.mm(self.positions.celeb_pack_closeup.lift_mm) + box_h * 0.5,
                    ],
                    extents: [box_w, box_d, box_h],
                    rotation: mat4_to_euler_xyz_rad(
                        Mat4::from_rotation_y(0.0) * Mat4::from_rotation_x(0.0),
                    ),
                    color: celeb.pack_kind.foil_tint(),
                    kind: Object3dKind::Pack {
                        kind: celeb.pack_kind,
                        pick_id: None,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: Some("shop.celebrations.pack_reveal_bg"),
                }]);

                let row_py = h * self.positions.celeb_pack_reveal.ny;
                let row_lift = layout.mm(self.positions.celeb_pack_reveal.lift_mm);
                let rotation = pack_reveal_euler_rad(&self.positions);
                let row = compute_pack_reveal_row_layout(&PackRevealRowLayoutParams {
                    win_w: w,
                    win_h: h,
                    cam,
                    preset: tile_preset,
                    n,
                    row_lift,
                    nx: self.positions.celeb_pack_reveal.nx,
                    ny: self.positions.celeb_pack_reveal.ny,
                    rotation_xyz_rad: rotation,
                });
                let src_px = w * 0.5;
                let src_py = h * self.positions.celeb_pack_closeup.ny;
                let src_lift = row_lift + h * 0.15;

                let mut placements = Vec::with_capacity(n);
                for i in 0..n {
                    let t = celeb.tile_progress(i);
                    let ease = 1.0 - (1.0_f32 - t).powi(3);

                    let dest_px = row.row_x0 + row.silhouette_w * 0.5 + i as f32 * row.step_px;
                    let px = src_px + (dest_px - src_px) * ease;
                    let py = src_py + (row_py - src_py) * ease;
                    let lift = src_lift + (row_lift - src_lift) * ease;
                    let scale = 0.3 + 0.7 * ease;

                    placements.push(ShowcaseTilePlacement {
                        tile: celeb.tiles[i],
                        center_pos: [px, py, lift],
                        rotation,
                        scale,
                        size_px: row.tile_size,
                        brightness: 1.0,
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

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if ctx.headless && !self.celebration.headless_hold_pack_closeup
            && matches!(self.celebration.phase, CelebPhase::Closeup) {
                self.celebration.phase = CelebPhase::Reveal;
                self.celebration.started_at = std::time::Instant::now();
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

    pub fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let env_h = ctx.room_gltf_height_scale;

        debug_assert!(
            !self.celebration.tiles.is_empty(),
            "pack celebration needs tiles"
        );

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        let layout = ShopLayout::build(ctx.layout, &self.positions, self.inventory);
        let cam = shop_celebration_camera(w, h, env_h);
        frame.camera_override = Some(cam);
        frame.scene_lighting.embedded_gltf_punctual = false;
        frame.scene_lighting.room_glb_brdf = false;
        frame
            .scene_lighting
            .set_smooth_points(pack_celebration_isolation_lights(
                self.celebration.phase,
                w,
                h,
                &layout,
                &self.positions,
            ));
        frame.scene_lighting.spot_lights = pack_celebration_subject_spotlight(
            &self.celebration,
            w,
            h,
            &cam,
            &layout,
            &self.positions,
            ctx.tile_preset,
        );
        self.push_celebration_draw(&mut frame, &layout, w, h, &cam, ctx.tile_preset);
        frame.window_title = "Mahjuro".to_string();
        frame.showcase_render_hints = Self::render_hints();

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

fn pack_reveal_euler_rad(positions: &ShopPositions) -> [f32; 3] {
    [
        positions.celeb_pack_reveal.rx_deg.to_radians() + 32.0_f32.to_radians(),
        positions.celeb_pack_reveal.ry_deg.to_radians(),
        positions.celeb_pack_reveal.rz_deg.to_radians() + std::f32::consts::PI,
    ]
}

fn pack_celebration_subject_spotlight(
    celeb: &PackCelebration,
    w: f32,
    h: f32,
    cam: &CameraParams,
    layout: &ShopLayout,
    positions: &ShopPositions,
    tile_preset: TilePreset,
) -> Vec<SpotLight> {
    let cos_outer = (34.0_f32).to_radians().cos();
    let cos_inner = (20.0_f32).to_radians().cos();
    let warm = [1.0_f32, 0.93, 0.78];
    match celeb.phase {
        CelebPhase::Closeup => {
            let box_h = h * 0.28;
            let cx = w * 0.5;
            let cy = h * positions.celeb_pack_closeup.ny;
            let lift = layout.mm(positions.celeb_pack_closeup.lift_mm) + box_h * 0.5;
            let light_lift = lift + h * 0.52;
            let pos = [cx, cy - h * 0.14, light_lift];
            let tw = world_on_camera_ray_plane_z(w, h, cam, cx, cy, lift);
            let lw = world_on_camera_ray_plane_z(w, h, cam, pos[0], pos[1], pos[2]);
            let dir = (tw - lw).normalize_or_zero();
            let dir = if dir.length_squared() < 1e-4 {
                Vec3::new(0.0, 0.5, -1.0).normalize()
            } else {
                dir
            };
            vec![SpotLight {
                pos,
                dir: dir.to_array(),
                radius: h * 2.2,
                cos_outer,
                cos_inner,
                color: warm,
                intensity: 7.0,
            }]
        }
        CelebPhase::Reveal => {
            let n = celeb.tiles.len();
            if n == 0 {
                return Vec::new();
            }
            let row_lift = layout.mm(positions.celeb_pack_reveal.lift_mm);
            let rotation = pack_reveal_euler_rad(positions);
            let row = compute_pack_reveal_row_layout(&PackRevealRowLayoutParams {
                win_w: w,
                win_h: h,
                cam,
                preset: tile_preset,
                n,
                row_lift,
                nx: positions.celeb_pack_reveal.nx,
                ny: positions.celeb_pack_reveal.ny,
                rotation_xyz_rad: rotation,
            });
            let total_w = n as f32 * row.silhouette_w + (n.saturating_sub(1)) as f32 * row.gap_px;
            let cx = row.row_x0 + total_w * 0.5;
            let row_py = h * positions.celeb_pack_reveal.ny;
            let cy = row_py;
            let lift = row_lift + row.tile_size * 0.15;
            let light_lift = lift + h * 0.55;
            let pos = [cx, cy - h * 0.12, light_lift];
            let tw = world_on_camera_ray_plane_z(w, h, cam, cx, cy, lift);
            let lw = world_on_camera_ray_plane_z(w, h, cam, pos[0], pos[1], pos[2]);
            let dir = (tw - lw).normalize_or_zero();
            let dir = if dir.length_squared() < 1e-4 {
                Vec3::new(0.0, 0.45, -1.0).normalize()
            } else {
                dir
            };
            vec![SpotLight {
                pos,
                dir: dir.to_array(),
                radius: h * 3.1,
                cos_outer,
                cos_inner,
                color: warm,
                intensity: 10.0,
            }]
        }
    }
}

fn pack_celebration_isolation_lights(
    phase: CelebPhase,
    w: f32,
    h: f32,
    layout: &ShopLayout,
    positions: &ShopPositions,
) -> Vec<PointLight> {
    let box_h = h * 0.28;
    let (cx, row_py, lift) = match phase {
        CelebPhase::Closeup => (
            w * 0.5,
            h * positions.celeb_pack_closeup.ny,
            layout.mm(positions.celeb_pack_closeup.lift_mm) + box_h * 0.5,
        ),
        CelebPhase::Reveal => (
            w * 0.5 + w * positions.celeb_pack_reveal.nx,
            h * positions.celeb_pack_reveal.ny,
            layout.mm(positions.celeb_pack_reveal.lift_mm),
        ),
    };
    let (i_mul, r_mul) = match phase {
        CelebPhase::Closeup => (2.4, 1.35),
        // Wider radius + higher intensity so an 8-tile row still reads on the sides.
        CelebPhase::Reveal => (1.65, 1.12),
    };
    vec![
        PointLight {
            pos: [cx + w * 0.14, row_py - h * 0.22, lift + h * 0.48],
            radius: h * 3.2 * r_mul,
            color: color::rgb(color::TALLOW),
            intensity: 1.55 * i_mul,
        },
        PointLight {
            pos: [cx - w * 0.16, row_py + h * 0.06, lift + h * 0.32],
            radius: h * 2.8 * r_mul,
            color: [0.70, 0.82, 1.0],
            intensity: 1.05 * i_mul,
        },
        PointLight {
            pos: [cx, row_py - h * 0.38, lift + h * 0.62],
            radius: h * 2.6 * r_mul,
            color: color::rgb(color::PARCHMENT),
            intensity: 1.25 * i_mul,
        },
    ]
}
