//! Tile-pack purchase flow on the showcase overlay.

use glam::{Mat4, Vec3};

use crate::core::tile_pack::PACK_TILE_ID_BASE;
use crate::core::tile_pack::TilePackKind;
use crate::persistence::TilePreset;
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, Object3d, Object3dKind, ShowcaseRenderHints, ShowcaseTilePlacement,
    UiFrame,
};
use crate::render::showcase_tile_layout::{
    PackRevealRowLayoutParams, compute_pack_reveal_row_layout,
};
use crate::render::theme::color;
use crate::render::wgpu_renderer::{PointLight, SpotLight};
use crate::render::world_space::world_on_camera_ray_plane_z;
use crate::scenes::celebration_overlay;
use crate::scenes::shop::{
    CelebPhase, PackCelebration, shop_celebration_camera,
};
use crate::ui::input::UiAction;
use crate::ui::layout::LayoutResult;
use crate::ui::placement::PlacementAnchor;
use crate::ui::scene_layout::ShopPositions;

use super::super::{BackgroundId, ButtonDef, DrawCtx, OverlayRequest, SceneTransition, UpdateCtx};

/// Pack mesh height vs window height (must match shop shelf “hero” scale).
const PACK_CELEB_BOX_H_FRAC: f32 = 0.56;
/// Nudge pack **down** in layout pixels so a tall box clears the title band and stays centered.
const PACK_CELEB_SCREEN_Y_DOWN_FRAC: f32 = 0.055;
/// Extra downward shift scaled with box height (keeps framing when [`PACK_CELEB_BOX_H_FRAC`] changes).
const PACK_CELEB_SCREEN_Y_PER_BOX_H: f32 = 0.10;

pub struct TilePackPresenter {
    pub celebration: PackCelebration,
    pub positions: ShopPositions,
}

impl TilePackPresenter {
    pub fn new(celebration: PackCelebration) -> Self {
        Self {
            celebration,
            positions: ShopPositions::default(),
        }
    }

    pub fn new_headless_screenshot(
        _run: &crate::game::run::RunState,
        pack_kind: TilePackKind,
    ) -> Self {
        let tiles = pack_kind.generate_tiles(PACK_TILE_ID_BASE);
        Self::new(PackCelebration::screenshot_pack_closeup_headless(
            tiles,
            pack_kind.name(),
            pack_kind,
        ))
    }

    pub fn render_hints() -> ShowcaseRenderHints {
        ShowcaseRenderHints {
            object3d_use_camera_ray_plane_z: true,
            showcase_tiles_use_camera_ray_plane_z: true,
            tile_pack_celebration_tonemap: true,
            shop_tonemap_and_lit_mesh_context: false,
            collection_tonemap_context: false,
        }
    }

    fn push_celebration_draw(
        &self,
        frame: &mut UiFrame,
        screen: &LayoutResult,
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

        let box_h = h * PACK_CELEB_BOX_H_FRAC;
        let pack_closeup = pack_closeup_anchor(screen, &self.positions, box_h, Mat4::IDENTITY);
        let pack_py_base = pack_closeup.pos[1];

        match celeb.phase {
            CelebPhase::Closeup => {
                let box_w = box_h * crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
                let box_d = box_h * 0.10;
                let t = celeb.started_at.elapsed().as_secs_f32();
                let bob_x = (t * 0.7).sin() * h * 0.008;
                let bob_y = (t * 0.5).sin() * h * 0.006;
                let bob_rx = (t * 0.6).sin() * 2.5;
                let bob_ry = (t * 0.8).cos() * 3.0;
                let bob_rot = Mat4::from_rotation_y(bob_ry.to_radians())
                    * Mat4::from_rotation_x(bob_rx.to_radians());
                let anchor = pack_closeup_anchor(screen, &self.positions, box_h, bob_rot);
                frame.object3d_batch(vec![Object3d {
                    pos: [anchor.pos[0] + bob_x, anchor.pos[1] + bob_y, anchor.pos[2]],
                    extents: [box_w, box_d, box_h],
                    rotation: anchor.object3d_rotation(),
                    color: celeb.pack_kind.foil_tint(),
                    kind: Object3dKind::Pack {
                        kind: celeb.pack_kind,
                        pick_id: None,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: Some(anchor.arrange_name),
                }]);

                frame.text(celebration_overlay::label_confirm_to_open(h, w, t));
            }
            CelebPhase::Reveal => {
                let box_w = box_h * crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
                let box_d = box_h * 0.10;

                // Keep the pack box visible while tiles fly out of it.
                // In Reveal, `started_at` is reset, so we use a smooth settle or just let it rest.
                // We'll let it rest at the base closeup position without the bobbing
                // so it looks like it settled down to open.
                frame.object3d_batch(vec![Object3d {
                    pos: pack_closeup.pos,
                    extents: [box_w, box_d, box_h],
                    rotation: pack_closeup.object3d_rotation(),
                    color: celeb.pack_kind.foil_tint(),
                    kind: Object3dKind::Pack {
                        kind: celeb.pack_kind,
                        pick_id: None,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: Some(pack_closeup.arrange_name),
                }]);

                let reveal_anchor = PlacementAnchor::new(
                    [0.0, 0.0, 0.0],
                    Mat4::IDENTITY,
                    &self.positions.celeb_pack_reveal,
                    "shop.celebrations.pack_reveal",
                    screen,
                );
                let row_py = reveal_anchor.pos[1];
                let row_lift = reveal_anchor.pos[2];
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
                let src_px = pack_closeup.pos[0];
                let src_py = pack_py_base;
                // Spawn tiles from the upper half of the enlarged pack so the arc reads as “out of the box”.
                let src_lift = row_lift + h * 0.15 + box_h * 0.42;

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
                        arrange_group: Some("shop.celebrations.pack_reveal"),
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
        if ctx.headless
            && !self.celebration.headless_hold_pack_closeup
            && matches!(self.celebration.phase, CelebPhase::Closeup)
        {
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
        let cam = shop_celebration_camera(w, h, env_h);
        frame.camera_override = Some(cam);
        frame.scene_lighting.embedded_gltf_punctual = false;
        frame.scene_lighting.room_glb_brdf = false;
        frame
            .scene_lighting
            .set_smooth_points(pack_celebration_isolation_lights(
                self.celebration.phase,
                ctx.layout,
                &self.positions,
            ));
        frame.scene_lighting.spot_lights = pack_celebration_subject_spotlight(
            &self.celebration,
            ctx.layout,
            &cam,
            &self.positions,
            ctx.tile_preset,
        );
        self.push_celebration_draw(&mut frame, ctx.layout, w, h, &cam, ctx.tile_preset);
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

fn pack_closeup_anchor(
    screen: &LayoutResult,
    positions: &ShopPositions,
    box_h: f32,
    base_rotation: Mat4,
) -> PlacementAnchor {
    let h = screen.window_h;
    let py_bias = h * PACK_CELEB_SCREEN_Y_DOWN_FRAC + box_h * PACK_CELEB_SCREEN_Y_PER_BOX_H;
    PlacementAnchor::new(
        [screen.window_w * 0.5, py_bias, box_h * 0.5],
        base_rotation,
        &positions.celeb_pack_closeup,
        "shop.celebrations.pack_closeup",
        screen,
    )
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
    screen: &LayoutResult,
    cam: &CameraParams,
    positions: &ShopPositions,
    tile_preset: TilePreset,
) -> Vec<SpotLight> {
    let w = screen.window_w;
    let h = screen.window_h;
    let cos_outer = (34.0_f32).to_radians().cos();
    let cos_inner = (20.0_f32).to_radians().cos();
    let warm = [1.0_f32, 0.93, 0.78];
    match celeb.phase {
        CelebPhase::Closeup => {
            let box_h = h * PACK_CELEB_BOX_H_FRAC;
            let anchor = pack_closeup_anchor(screen, positions, box_h, Mat4::IDENTITY);
            let cx = anchor.pos[0];
            let cy = anchor.pos[1];
            let lift = anchor.pos[2];
            let light_lift = lift + h * 0.52 + box_h * 0.38;
            let pos = [cx, cy - h * 0.14 - box_h * 0.06, light_lift];
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
                radius: h * 2.2 + box_h * 0.85,
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
            let reveal_anchor = PlacementAnchor::new(
                [0.0, 0.0, 0.0],
                Mat4::IDENTITY,
                &positions.celeb_pack_reveal,
                "shop.celebrations.pack_reveal",
                screen,
            );
            let row_lift = reveal_anchor.pos[2];
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
            let row_py = reveal_anchor.pos[1];
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
    screen: &LayoutResult,
    positions: &ShopPositions,
) -> Vec<PointLight> {
    let w = screen.window_w;
    let h = screen.window_h;
    let box_h = h * PACK_CELEB_BOX_H_FRAC;
    let (cx, row_py, lift) = match phase {
        CelebPhase::Closeup => {
            let a = pack_closeup_anchor(screen, positions, box_h, Mat4::IDENTITY);
            (a.pos[0], a.pos[1], a.pos[2])
        }
        CelebPhase::Reveal => {
            let a = PlacementAnchor::new(
                [0.0, 0.0, 0.0],
                Mat4::IDENTITY,
                &positions.celeb_pack_reveal,
                "shop.celebrations.pack_reveal",
                screen,
            );
            (a.pos[0], a.pos[1], a.pos[2])
        }
    };
    let (i_mul, r_mul) = match phase {
        CelebPhase::Closeup => (2.4, 1.35),
        // Wider radius + higher intensity so an 8-tile row still reads on the sides.
        CelebPhase::Reveal => (1.65, 1.12),
    };
    let close_z_k = match phase {
        CelebPhase::Closeup => box_h * 0.22,
        CelebPhase::Reveal => 0.0,
    };
    vec![
        PointLight {
            pos: [
                cx + w * 0.14,
                row_py - h * 0.22 - box_h * 0.04,
                lift + h * 0.48 + close_z_k,
            ],
            radius: h * 3.2 * r_mul + box_h * 0.5 * r_mul,
            color: color::rgb(color::TALLOW),
            intensity: 1.55 * i_mul,
        },
        PointLight {
            pos: [
                cx - w * 0.16,
                row_py + h * 0.06 + box_h * 0.03,
                lift + h * 0.32 + close_z_k * 0.7,
            ],
            radius: h * 2.8 * r_mul + box_h * 0.45 * r_mul,
            color: [0.70, 0.82, 1.0],
            intensity: 1.05 * i_mul,
        },
        PointLight {
            pos: [
                cx,
                row_py - h * 0.38 - box_h * 0.08,
                lift + h * 0.62 + close_z_k * 1.1,
            ],
            radius: h * 2.6 * r_mul + box_h * 0.4 * r_mul,
            color: color::rgb(color::PARCHMENT),
            intensity: 1.25 * i_mul,
        },
    ]
}
