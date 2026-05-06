use super::*;

use glam::Mat4;

use crate::render::table_transform::mat4_to_euler_xyz_rad;
use crate::scenes::celebration_overlay;

impl ShopScene {
    /// Tile-pack opening celebration — dimmer, title, pack mesh / reveal tiles.
    /// Caller clears scene buttons and registers [`SHOP_3D_HIT_ID`] fullscreen when active.
    pub(super) fn push_tile_pack_celebration(
        &self,
        frame: &mut UiFrame,
        layout: &ShopLayout,
        w: f32,
        h: f32,
    ) {
        let Some(ref celeb) = self.pack_celebration else {
            return;
        };
        let n = celeb.tiles.len();
        celebration_overlay::CelebrationOverlayScratch::new(w, h).push_dimmer_then_depth_reset(frame);
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
                let tile_size = h * 0.13;
                let gap = tile_size * 0.25;
                let total_w = n as f32 * tile_size + (n.saturating_sub(1)) as f32 * gap;
                let row_x0 = (w - total_w) * 0.5 + w * self.positions.celeb_pack_reveal.nx;
                let row_py = h * self.positions.celeb_pack_reveal.ny;
                let row_lift = layout.mm(self.positions.celeb_pack_reveal.lift_mm);
                let src_px = w * 0.5;
                let src_py = h * self.positions.celeb_pack_closeup.ny;
                let src_lift = row_lift + h * 0.15;
                let row_rx =
                    self.positions.celeb_pack_reveal.rx_deg.to_radians() + 60.0_f32.to_radians();
                let row_ry = self.positions.celeb_pack_reveal.ry_deg.to_radians();
                let row_rz =
                    self.positions.celeb_pack_reveal.rz_deg.to_radians() + std::f32::consts::PI;

                let mut placements = Vec::with_capacity(n);
                for i in 0..n {
                    let t = celeb.tile_progress(i);
                    let ease = 1.0 - (1.0 - t).powi(3);

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
                        brightness: 1.0,
                        selected: false,
                        hovered: false,
                        outline: false,
                        glow: false,
                        glow_color: None,
                        pick_id: None,
                    });
                }

                frame
                    .cmds
                    .push(crate::render::draw_cmd::DrawCmd::ShowcaseTileBatch(
                        placements,
                    ));

                if celeb.fully_settled() {
                    let elapsed = celeb.elapsed();
                    frame.text(celebration_overlay::label_confirm_to_continue(
                        h, w, elapsed, 1.0,
                    ));
                }
            }
        }
    }

    pub(super) fn push_shop_particle_quads(&self, frame: &mut UiFrame) {
        for (rect, color) in self.particles.instances() {
            frame.quad(GpuInstance { rect, color });
        }
    }

    pub(super) fn push_shop_score_popup_labels(&self, frame: &mut UiFrame, w: f32, h: f32) {
        let popup_scale = w.min(h) / 1080.0;
        let now_celeb = Instant::now();
        for lbl in self
            .score_popups
            .overlay_text_labels(now_celeb, w, h, popup_scale)
        {
            frame.text(lbl);
        }
    }

}
