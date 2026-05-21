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
use crate::render::world_space::pixel_to_world;
use crate::scenes::celebration_overlay;
use crate::scenes::shop::{CelebPhase, PackCelebration, shop_celebration_camera};
use crate::ui::input::UiAction;
use crate::ui::layout::LayoutResult;
use crate::ui::placement::PlacementAnchor;
use crate::ui::scene_layout::ShopPositions;

use super::super::{BackgroundId, ButtonDef, DrawCtx, OverlayRequest, SceneTransition, UpdateCtx};

/// Pack mesh height vs window height (must match shop shelf “hero” scale).
pub(crate) const PACK_CELEB_BOX_H_FRAC: f32 = 0.56;
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
        Self::new(PackCelebration::screenshot_reveal_settled(
            tiles,
            pack_kind.name(),
            pack_kind,
        ))
    }

    pub fn render_hints() -> ShowcaseRenderHints {
        ShowcaseRenderHints {
            // Perspective shop camera: map screen `(px, py)` through the ray → `plane_z`
            // (`anchor.pos[2]`, usually 0). `pixel_to_world` is not the inverse of this
            // projection and pushes celebration meshes out of the frustum.
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
                let t = celeb.started_at.elapsed().as_secs_f32();
                let bob_x = (t * 0.7).sin() * h * 0.008;
                let bob_y = (t * 0.5).sin() * h * 0.006;
                let bob_rx = (t * 0.6).sin() * 2.5;
                let bob_ry = (t * 0.8).cos() * 3.0;
                let bob_rot = Mat4::from_rotation_y(bob_ry.to_radians())
                    * Mat4::from_rotation_x(bob_rx.to_radians());
                let anchor = pack_closeup_anchor(screen, &self.positions, box_h, bob_rot);
                let extents = pack_closeup_world_extents(
                    w,
                    h,
                    cam,
                    anchor.pos[0] + bob_x,
                    anchor.pos[1] + bob_y,
                    anchor.pos[2],
                    box_h,
                );
                frame.object3d_batch(vec![Object3d {
                    pos: [anchor.pos[0] + bob_x, anchor.pos[1] + bob_y, anchor.pos[2]],
                    extents,
                    rotation: anchor.object3d_rotation(),
                    color: celeb.pack_kind.foil_tint(),
                    kind: Object3dKind::Pack {
                        kind: celeb.pack_kind,
                        pick_id: None,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                }]);

                frame.text(celebration_overlay::label_confirm_to_open(h, w, t));
            }
            CelebPhase::Reveal => {
                let pack_extents = pack_closeup_world_extents(
                    w,
                    h,
                    cam,
                    pack_closeup.pos[0],
                    pack_closeup.pos[1],
                    pack_closeup.pos[2],
                    box_h,
                );

                // Keep the pack box visible while tiles fly out of it.
                // In Reveal, `started_at` is reset, so we use a smooth settle or just let it rest.
                // We'll let it rest at the base closeup position without the bobbing
                // so it looks like it settled down to open.
                frame.object3d_batch(vec![Object3d {
                    pos: pack_closeup.pos,
                    extents: pack_extents,
                    rotation: pack_closeup.object3d_rotation(),
                    color: celeb.pack_kind.foil_tint(),
                    kind: Object3dKind::Pack {
                        kind: celeb.pack_kind,
                        pick_id: None,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                }]);

                let reveal_anchor = PlacementAnchor::new(
                    [w * 0.5, h * 0.5, 0.0],
                    Mat4::IDENTITY,
                    &self.positions.celeb_pack_reveal,
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
                        overlay_rect_group: None,
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
                    ctx.run.pending_shop_focus_snap_after_celebration = true;
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

/// Screen-pixel pack height → world `Object3d::extents` for the perspective celebration camera.
pub(crate) fn pack_closeup_world_extents(
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    px: f32,
    py: f32,
    plane_z: f32,
    screen_h_px: f32,
) -> [f32; 3] {
    let half_h = screen_h_px * 0.5;
    let half_w = half_h * crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
    let sample = |x: f32, y: f32| {
        crate::render::world_space::world_on_camera_ray_plane_z(win_w, win_h, cam, x, y, plane_z)
    };
    let center = sample(px, py);
    let world_h = (sample(px, py - half_h) - center).length().max(1.0);
    let world_w = (sample(px + half_w, py) - center).length().max(1.0);
    [world_w, world_h * 0.10, world_h]
}

pub(crate) fn pack_closeup_anchor(
    screen: &LayoutResult,
    _positions: &ShopPositions,
    box_h: f32,
    base_rotation: Mat4,
) -> PlacementAnchor {
    let h = screen.window_h;
    let w = screen.window_w;
    let py_bias = h * PACK_CELEB_SCREEN_Y_DOWN_FRAC + box_h * PACK_CELEB_SCREEN_Y_PER_BOX_H;
    // Pack foil is authored on the −Y face; the shop celebration camera sits on
    // −Y looking toward +Y, so no extra flip is needed (π around X shows the back).
    // Keep lift at 0 in the anchor (same as zodiac ribbon): `pos[2]` is world Z via
    // `pixel_to_world`, not a ray-plane depth; large box-height lifts push the mesh
    // out of the celebration frustum.
    PlacementAnchor::new(
        [w * 0.5, h * 0.5 + py_bias, 0.0],
        base_rotation,
        &crate::ui::placement::Placement::default(),
        screen,
    )
}

pub(crate) fn pack_reveal_euler_rad(positions: &ShopPositions) -> [f32; 3] {
    [
        positions.celeb_pack_reveal.rx_deg.to_radians() + 32.0_f32.to_radians(),
        positions.celeb_pack_reveal.ry_deg.to_radians(),
        positions.celeb_pack_reveal.rz_deg.to_radians() + std::f32::consts::PI,
    ]
}

/// Push one pack mesh for celebration closeup (TPOS1 / TPOS2).
pub(crate) fn push_pack_object3d(
    frame: &mut UiFrame,
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    anchor: &PlacementAnchor,
    screen_h_px: f32,
    pack_kind: TilePackKind,
    color: [f32; 4],
    bob_xy: Option<(f32, f32)>,
) {
    let (bx, by) = bob_xy.unwrap_or((0.0, 0.0));
    let pos = [anchor.pos[0] + bx, anchor.pos[1] + by, anchor.pos[2]];
    let extents =
        pack_closeup_world_extents(win_w, win_h, cam, pos[0], pos[1], pos[2], screen_h_px);
    frame.object3d_batch(vec![Object3d {
        pos,
        extents,
        rotation: anchor.object3d_rotation(),
        color,
        kind: Object3dKind::Pack {
            kind: pack_kind,
            pick_id: None,
        },
        hover_target: 0.0,
        anim_id: 0,
    }]);
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
            let tw = pixel_to_world(w, h, cx, cy, lift);
            let lw = pixel_to_world(w, h, pos[0], pos[1], pos[2]);
            let dir = (tw - lw).normalize_or_zero();
            let dir = if dir.length_squared() < 1e-4 {
                Vec3::new(0.0, 0.5, -1.0).normalize()
            } else {
                dir
            };
            let _ = cam;
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
                [w * 0.5, h * 0.5, 0.0],
                Mat4::IDENTITY,
                &positions.celeb_pack_reveal,
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
            let tw = pixel_to_world(w, h, cx, cy, lift);
            let lw = pixel_to_world(w, h, pos[0], pos[1], pos[2]);
            let dir = (tw - lw).normalize_or_zero();
            let dir = if dir.length_squared() < 1e-4 {
                Vec3::new(0.0, 0.45, -1.0).normalize()
            } else {
                dir
            };
            let _ = cam;
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
                [w * 0.5, h * 0.5, 0.0],
                Mat4::IDENTITY,
                &positions.celeb_pack_reveal,
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
            color: color::rgb(color::PARCHMENT),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::world_space::pixel_to_world;
    use crate::ui::layout::UiLayout;

    #[test]
    fn pack_closeup_projects_into_viewport() {
        use crate::render::table_transform::translate_rot_scale;
        use glam::{Mat4, Vec3};

        let w = 1920.0_f32;
        let h = 1080.0_f32;
        let env_h = crate::render::room_glb::SHOP_ENV_HEIGHT_SCALE;
        let cam = shop_celebration_camera(w, h, env_h);
        let aspect = w / h;
        let eye = Vec3::from_array(cam.eye);
        let target = Vec3::from_array(cam.target);
        let up = Vec3::from_array(cam.up);
        let view = Mat4::look_at_rh(eye, target, up);
        let (near, far) = cam.clip_planes(h);
        let proj = Mat4::perspective_rh(cam.fovy_deg.to_radians(), aspect, near, far);
        let view_proj = proj * view;

        let mut layout_engine = UiLayout::new();
        let layout = layout_engine.solve(w, h);
        let box_h = h * PACK_CELEB_BOX_H_FRAC;
        let box_w = box_h * crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
        let box_d = box_h * 0.10;
        let anchor = pack_closeup_anchor(&layout, &ShopPositions::default(), box_h, Mat4::IDENTITY);
        let extents = pack_closeup_world_extents(
            w,
            h,
            &cam,
            anchor.pos[0],
            anchor.pos[1],
            anchor.pos[2],
            box_h,
        );
        let center = crate::render::world_space::world_on_camera_ray_plane_z(
            w,
            h,
            &cam,
            anchor.pos[0],
            anchor.pos[1],
            anchor.pos[2],
        );
        let model = translate_rot_scale(center, Mat4::IDENTITY, Vec3::from(extents));

        let mut ndc_min = Vec3::splat(f32::INFINITY);
        let mut ndc_max = Vec3::splat(f32::NEG_INFINITY);
        for &corner in &[
            Vec3::new(-0.5, -0.5, -0.5),
            Vec3::new(0.5, -0.5, -0.5),
            Vec3::new(-0.5, 0.5, -0.5),
            Vec3::new(0.5, 0.5, -0.5),
            Vec3::new(-0.5, -0.5, 0.5),
            Vec3::new(0.5, -0.5, 0.5),
            Vec3::new(-0.5, 0.5, 0.5),
            Vec3::new(0.5, 0.5, 0.5),
        ] {
            let world = model.transform_point3(corner);
            let clip = view_proj * world.extend(1.0);
            let ndc = clip.truncate() / clip.w;
            ndc_min = ndc_min.min(ndc);
            ndc_max = ndc_max.max(ndc);
        }
        assert!(
            ndc_min.x > -1.05 && ndc_max.x < 1.05 && ndc_min.y > -1.05 && ndc_max.y < 1.05,
            "pack NDC bounds {ndc_min:?}..{ndc_max:?} should intersect the viewport"
        );
        assert!(
            ndc_max.z > -1.0 && ndc_min.z < 1.0,
            "pack depth {ndc_min:?}..{ndc_max:?} should pass the clip volume"
        );
    }

    #[test]
    fn pack_closeup_pixel_anchor_stays_near_scene_origin() {
        let w = 1920.0_f32;
        let h = 1080.0_f32;
        let mut layout_engine = UiLayout::new();
        let layout = layout_engine.solve(w, h);
        let box_h = h * PACK_CELEB_BOX_H_FRAC;
        let anchor = pack_closeup_anchor(&layout, &ShopPositions::default(), box_h, Mat4::IDENTITY);
        let center = pixel_to_world(w, h, anchor.pos[0], anchor.pos[1], anchor.pos[2]);
        assert!(
            center.y > -h && center.y < h && center.z >= 0.0 && center.z < h,
            "pack center {center:?} should sit in front of the shop camera"
        );
    }
}
