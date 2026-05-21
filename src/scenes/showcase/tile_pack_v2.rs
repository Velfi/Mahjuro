//! Tile-pack opening sequence #2 (TPOS2) on the showcase overlay.

use std::f32::consts::PI;
use std::time::{Duration, Instant};

use glam::{Mat4, Vec3};

use crate::persistence::TilePreset;
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, ShowcaseRenderHints, ShowcaseTilePlacement, UiFrame,
};
use crate::render::pack_palette;
use crate::render::showcase_tile_layout::{
    PackRevealRowLayoutParams, compute_pack_reveal_row_layout,
};
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, PointLight, SpotLight, TextAlign, TextLabel};
use crate::render::world_space::pixel_to_world;
use crate::scenes::celebration_overlay::{
    self, CelebrationOverlayScratch, CelebrationShowcaseIntroGate, ShootingStarCelebrationIntro,
};
use crate::scenes::shop::pack_celebration_v2::{PackCelebrationV2, Tpos2Phase};
use crate::scenes::shop::shop_celebration_camera;
use crate::ui::input::UiAction;
use crate::ui::layout::LayoutResult;
use crate::ui::placement::PlacementAnchor;
use crate::ui::scene_layout::ShopPositions;

use super::super::{BackgroundId, ButtonDef, DrawCtx, OverlayRequest, SceneTransition, UpdateCtx};
use super::tile_pack::{
    PACK_CELEB_BOX_H_FRAC, pack_closeup_anchor, pack_reveal_euler_rad, push_pack_object3d,
};

const UNSEAL_DOLLY_PULL: f32 = 0.04;

pub struct Tpos2Presenter {
    pub celebration: PackCelebrationV2,
    pub positions: ShopPositions,
    intro_gate: CelebrationShowcaseIntroGate,
    arrival_done_at: Option<Instant>,
}

impl Tpos2Presenter {
    pub fn new(celebration: PackCelebrationV2) -> Self {
        Self {
            celebration,
            positions: ShopPositions::default(),
            intro_gate: CelebrationShowcaseIntroGate::new(
                ShootingStarCelebrationIntro::new_pack_opening(),
            ),
            arrival_done_at: None,
        }
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

    fn intro_content_alpha(&self, ctx: &DrawCtx<'_>) -> f32 {
        self.intro_gate.intro.content_alpha_for(&ctx.effect_layers)
    }

    fn camera_with_dolly(&self, w: f32, h: f32, env_h: f32, pull: f32) -> CameraParams {
        let mut cam = shop_celebration_camera(w, h, env_h);
        if pull <= 0.0 {
            return cam;
        }
        let eye = Vec3::from_array(cam.eye);
        let target = Vec3::from_array(cam.target);
        let delta = (eye - target) * pull;
        cam.eye = (eye + delta).to_array();
        cam
    }

    fn push_celebration_draw(
        &self,
        frame: &mut UiFrame,
        screen: &LayoutResult,
        w: f32,
        h: f32,
        cam: &CameraParams,
        tile_preset: TilePreset,
        content_alpha: f32,
        ctx: &DrawCtx<'_>,
    ) {
        let celeb = &self.celebration;
        let palette = pack_palette::for_kind(celeb.pack_kind);
        let n = celeb.tiles.len();

        let after_dimmer =
            CelebrationOverlayScratch::new(w, h).push_dimmer_scaled(frame, content_alpha);
        after_dimmer
            .push_starfield_if(frame, ctx.effect_layers.starfield)
            .push_depth_reset_for_celebration_mesh(frame);

        if matches!(celeb.phase, Tpos2Phase::Arrival) {
            let wipe_t = self.intro_gate.intro.transition_progress();
            if wipe_t > 0.35 {
                let bg_a = content_alpha * smoothstep(0.35, 0.7, wipe_t) * 0.55;
                let mut bg = palette.bg;
                bg[3] *= bg_a;
                frame.quad(GpuInstance {
                    rect: [0.0, 0.0, w, h],
                    color: bg,
                    user: 0,
                });
            }
        }

        let title_alpha = match celeb.phase {
            Tpos2Phase::Arrival => {
                content_alpha * smoothstep(0.5, 0.85, self.intro_gate.intro.transition_progress())
            }
            _ => content_alpha,
        };
        if title_alpha > 0.01 {
            let title_font =
                crate::render::theme::typography::size(crate::render::theme::typography::H24, h);
            frame.text(TextLabel {
                text: celeb.pack_name.to_string(),
                rect: [0.0, h * 0.18, w, title_font * 1.5],
                font_px: Some(title_font),
                color: [
                    color::CHAMPAGNE[0],
                    color::CHAMPAGNE[1],
                    color::CHAMPAGNE[2],
                    title_alpha,
                ],
                align: TextAlign::Center,
                ..Default::default()
            });
        }

        let box_h = h * PACK_CELEB_BOX_H_FRAC;

        match celeb.phase {
            Tpos2Phase::Arrival => {}
            Tpos2Phase::Anticipation => {
                let t = celeb.elapsed();
                let bob_x = (t * 0.7).sin() * h * 0.008;
                let bob_y = (t * 0.5).sin() * h * 0.006;
                let bob_rx = (t * 0.6).sin() * 2.5;
                let bob_ry = (t * 0.8).cos() * 3.0;
                let bob_rot = Mat4::from_rotation_y(bob_ry.to_radians())
                    * Mat4::from_rotation_x(bob_rx.to_radians());
                let anchor = pack_closeup_anchor(screen, &self.positions, box_h, bob_rot);

                // Localized seal glow behind the pack (not full-screen).
                let seal_a = content_alpha * (0.12 + 0.06 * (t * 2.0).sin()).clamp(0.0, 0.22);
                let pad = box_h * 0.52;
                let mut seal_c = palette.seal;
                seal_c[3] = seal_a;
                frame.quad(GpuInstance {
                    rect: [
                        anchor.pos[0] + bob_x - pad,
                        anchor.pos[1] + bob_y - pad * 1.05,
                        pad * 2.0,
                        pad * 2.1,
                    ],
                    color: seal_c,
                    user: 0,
                });

                let shimmer = 0.06 * (0.5 + 0.5 * (t * 3.2).sin());
                let mut foil = celeb.pack_kind.foil_tint();
                foil[0] = (foil[0] + shimmer).min(1.25);
                foil[1] = (foil[1] + shimmer).min(1.25);
                foil[2] = (foil[2] + shimmer).min(1.25);
                push_pack_object3d(
                    frame,
                    w,
                    h,
                    cam,
                    &anchor,
                    box_h,
                    celeb.pack_kind,
                    foil,
                    Some((bob_x, bob_y)),
                );

                frame.text(celebration_overlay::label_confirm_to_unseal(
                    h,
                    w,
                    t,
                    content_alpha,
                ));
            }
            Tpos2Phase::Unseal => {
                let u = celeb.unseal_t();
                let snap = ease_out_cubic((u / 0.2).min(1.0));
                let screen_h = box_h * (1.0 + 0.06 * snap);
                let tilt_x = 8.0_f32.to_radians() * smoothstep(0.15, 0.35, u);
                let bob_rot = Mat4::from_rotation_x(tilt_x);
                let anchor = pack_closeup_anchor(screen, &self.positions, box_h, bob_rot);

                let flash = smoothstep(0.25, 0.4, u) * (1.0 - smoothstep(0.45, 0.55, u));
                let mut foil = celeb.pack_kind.foil_tint();
                if flash > 0.0 {
                    foil = [
                        foil[0] + (1.4 - foil[0]) * flash,
                        foil[1] + (1.4 - foil[1]) * flash,
                        foil[2] + (1.4 - foil[2]) * flash,
                        foil[3],
                    ];
                }
                let pack_alpha = content_alpha * (1.0 - smoothstep(0.4, 0.55, u));
                let ghost_scale = 1.0 - 0.15 * smoothstep(0.4, 0.55, u);
                if pack_alpha > 0.02 {
                    push_pack_object3d(
                        frame,
                        w,
                        h,
                        cam,
                        &anchor,
                        screen_h * ghost_scale,
                        celeb.pack_kind,
                        foil,
                        None,
                    );
                }

                let burst_a = content_alpha
                    * (smoothstep(0.05, 0.2, u) * (1.0 - smoothstep(0.35, 0.5, u)) * 0.25);
                if burst_a > 0.001 {
                    let r = h.min(w) * 0.45 * smoothstep(0.0, 0.35, u);
                    let mut burst = palette.seal;
                    burst[3] = burst_a;
                    frame.quad(GpuInstance {
                        rect: [anchor.pos[0] - r, anchor.pos[1] - r, r * 2.0, r * 2.0],
                        color: burst,
                        user: 0,
                    });
                }
            }
            Tpos2Phase::Deal => {
                let pack_closeup =
                    pack_closeup_anchor(screen, &self.positions, box_h, Mat4::IDENTITY);
                let pack_py_base = pack_closeup.pos[1];

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
                let src_lift = row_lift + h * 0.15 + box_h * 0.42;

                let fan_half = PackCelebrationV2::FAN_HALF_DEG.to_radians();
                let deal_elapsed = celeb.elapsed();
                let last_land_t = if n > 0 {
                    (n - 1) as f32 * PackCelebrationV2::DEAL_STAGGER
                        + PackCelebrationV2::DEAL_TILE_FLY_SECS
                } else {
                    0.0
                };
                let last_glow = deal_elapsed >= last_land_t
                    && deal_elapsed < last_land_t + PackCelebrationV2::LAST_TILE_GLOW_SECS;

                let mut placements = Vec::with_capacity(n);
                for i in 0..n {
                    let p = celeb.tile_progress(i);
                    let dest_px = row.row_x0 + row.silhouette_w * 0.5 + i as f32 * row.step_px;
                    let (px, py, lift, scale, spin_z) = deal_tile_transform(
                        p, w, h, n, i, fan_half, src_px, src_py, src_lift, dest_px, row_py,
                        row_lift,
                    );

                    let in_flight = p > 0.05 && p < 0.95;
                    let landed_last = i == n.saturating_sub(1) && p >= 1.0 && last_glow;
                    let glow = in_flight || landed_last;
                    let mut tile_rot = rotation;
                    tile_rot[2] += spin_z;

                    placements.push(ShowcaseTilePlacement {
                        tile: celeb.tiles[i],
                        center_pos: [px, py, lift],
                        rotation: tile_rot,
                        scale,
                        size_px: row.tile_size,
                        brightness: 1.0,
                        selected: false,
                        hovered: false,
                        outline: false,
                        glow,
                        glow_color: if glow { Some(palette.seal) } else { None },
                        pick_id: None,
                        overlay_rect_group: None,
                    });
                }

                frame.cmds.push(DrawCmd::ShowcaseTileBatch(placements));

                if celeb.fully_settled() {
                    frame.text(celebration_overlay::label_confirm_to_continue(
                        h,
                        w,
                        celeb.elapsed(),
                        content_alpha,
                    ));
                }
            }
        }
    }

    pub fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        self.intro_gate.tick(&mut ctx);

        if ctx.headless {
            self.intro_gate.intro.jump_to_done();
            if self.celebration.headless_hold_pack_closeup
                && self.celebration.phase == Tpos2Phase::Arrival
            {
                self.celebration.phase = Tpos2Phase::Anticipation;
                self.celebration.started_at = Instant::now();
            } else if !self.celebration.headless_hold_pack_closeup
                && self.celebration.phase == Tpos2Phase::Arrival
                && self.intro_gate.intro.is_done_for(&ctx.effect_layers)
            {
                self.celebration.phase = Tpos2Phase::Anticipation;
                self.celebration.started_at = Instant::now();
            }
        }

        if self.celebration.phase == Tpos2Phase::Arrival
            && self.intro_gate.intro.is_done_for(&ctx.effect_layers)
        {
            if self.arrival_done_at.is_none() {
                self.arrival_done_at = Some(Instant::now());
            } else if self.arrival_done_at.is_some()
                && Instant::now().saturating_duration_since(self.arrival_done_at.unwrap())
                    >= Duration::from_secs_f32(CelebrationShowcaseIntroGate::GRACE_AFTER_DONE_SECS)
            {
                self.celebration.phase = Tpos2Phase::Anticipation;
                self.celebration.started_at = Instant::now();
                self.arrival_done_at = None;
            }
        }

        let has_input = ctx.actions.iter().any(|a| {
            matches!(
                a,
                UiAction::Confirm | UiAction::Cancel | UiAction::CommitDiscard
            )
        }) || !ctx.button_clicks.is_empty();

        match self.celebration.phase {
            Tpos2Phase::Arrival => {}
            Tpos2Phase::Anticipation => {
                if has_input {
                    self.celebration.phase = Tpos2Phase::Unseal;
                    self.celebration.started_at = Instant::now();
                    ctx.bus.push(crate::game::event_bus::GameEvent::PackOpened);
                }
            }
            Tpos2Phase::Unseal => {
                if self.celebration.elapsed() >= PackCelebrationV2::UNSEAL_SECS {
                    self.celebration.phase = Tpos2Phase::Deal;
                    self.celebration.started_at = Instant::now();
                }
            }
            Tpos2Phase::Deal => {
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
                let done = self.celebration.fully_settled() || self.celebration.dismissed;
                if done && has_input {
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

        debug_assert!(!self.celebration.tiles.is_empty());

        let content_alpha = self.intro_content_alpha(&ctx);
        let dolly = match self.celebration.phase {
            Tpos2Phase::Unseal => UNSEAL_DOLLY_PULL * ease_out_cubic(self.celebration.unseal_t()),
            Tpos2Phase::Deal => UNSEAL_DOLLY_PULL,
            _ => 0.0,
        };
        let cam = self.camera_with_dolly(w, h, env_h, dolly);

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.camera_override = Some(cam);
        frame.scene_lighting.embedded_gltf_punctual = false;
        frame.scene_lighting.room_glb_brdf = false;
        frame.scene_lighting.set_smooth_points(tpos2_point_lights(
            &self.celebration,
            ctx.layout,
            &self.positions,
        ));
        frame.scene_lighting.spot_lights = tpos2_spot_lights(
            &self.celebration,
            ctx.layout,
            &cam,
            &self.positions,
            ctx.tile_preset,
        );

        self.push_celebration_draw(
            &mut frame,
            ctx.layout,
            w,
            h,
            &cam,
            ctx.tile_preset,
            content_alpha,
            &ctx,
        );

        self.intro_gate
            .intro
            .push_shooting_star_cascade_if_active(&mut frame, &ctx.effect_layers);

        frame.window_title = "Mahjuro".to_string();
        frame.showcase_render_hints = Self::render_hints();

        let allow_click = match self.celebration.phase {
            Tpos2Phase::Arrival => false,
            Tpos2Phase::Anticipation => self.intro_gate.intro.is_done_for(&ctx.effect_layers),
            Tpos2Phase::Unseal => false,
            Tpos2Phase::Deal => self.celebration.fully_settled() || self.celebration.dismissed,
        };
        if allow_click {
            frame.buttons = vec![ButtonDef::scene((0.0, 0.0, w, h), u32::MAX)];
        }
        frame
    }
}

fn deal_tile_transform(
    p: f32,
    w: f32,
    h: f32,
    n: usize,
    i: usize,
    fan_half: f32,
    src_px: f32,
    src_py: f32,
    src_lift: f32,
    dest_px: f32,
    dest_py: f32,
    dest_lift: f32,
) -> (f32, f32, f32, f32, f32) {
    let fan_t = if n <= 1 {
        0.0
    } else {
        i as f32 / (n - 1) as f32
    };
    let fan_angle = fan_half * (fan_t * 2.0 - 1.0);
    let lateral = w * 0.14 * fan_angle.sin();

    let m_px = src_px + (dest_px - src_px) * 0.5 + lateral;
    let m_py = src_py + (dest_py - src_py) * 0.5;
    let m_lift = src_lift + (dest_lift - src_lift) * 0.5 + h * 0.22;

    let ctrl_px = m_px + lateral * 0.35;
    let ctrl_py = m_py - h * 0.08;
    let ctrl_lift = m_lift + h * 0.10;

    let split = PackCelebrationV2::ARC_SPLIT;

    if p <= split {
        let u = (p / split).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - u).powi(3);
        let (px, py, lift) = quad_bezier(
            [src_px, src_py, src_lift],
            [ctrl_px, ctrl_py, ctrl_lift],
            [m_px, m_py, m_lift],
            ease,
        );
        let scale = 0.25 + 0.60 * ease;
        let spin = (1.0 - ease) * PI * 0.25;
        (px, py, lift, scale, spin)
    } else {
        let u = ((p - split) / (1.0 - split)).clamp(0.0, 1.0);
        let ease = smoothstep(0.0, 1.0, u);
        let overshoot = 1.0 + 0.04 * (ease * PI).sin();
        let px = m_px + (dest_px - m_px) * ease * overshoot;
        let py = m_py + (dest_py - m_py) * ease;
        let lift = m_lift + (dest_lift - m_lift) * ease;
        let scale = 0.85 + 0.15 * ease;
        let spin = (1.0 - ease) * PI * 0.08;
        (px, py, lift, scale, spin)
    }
}

fn quad_bezier(a: [f32; 3], b: [f32; 3], c: [f32; 3], t: f32) -> (f32, f32, f32) {
    let u = 1.0 - t;
    let px = u * u * a[0] + 2.0 * u * t * b[0] + t * t * c[0];
    let py = u * u * a[1] + 2.0 * u * t * b[1] + t * t * c[1];
    let lift = u * u * a[2] + 2.0 * u * t * b[2] + t * t * c[2];
    (px, py, lift)
}

fn tpos2_spot_lights(
    celeb: &PackCelebrationV2,
    screen: &LayoutResult,
    cam: &CameraParams,
    positions: &ShopPositions,
    tile_preset: TilePreset,
) -> Vec<SpotLight> {
    let w = screen.window_w;
    let h = screen.window_h;
    let (cos_outer, cos_inner, intensity) = match celeb.phase {
        Tpos2Phase::Anticipation | Tpos2Phase::Unseal => (
            (34.0_f32).to_radians().cos(),
            (20.0_f32).to_radians().cos(),
            7.0,
        ),
        Tpos2Phase::Deal => (
            (34.0_f32).to_radians().cos(),
            (20.0_f32).to_radians().cos(),
            10.0,
        ),
        Tpos2Phase::Arrival => return Vec::new(),
    };
    let warm = [1.0_f32, 0.93, 0.78];
    let box_h = h * PACK_CELEB_BOX_H_FRAC;

    let (cx, cy, lift) = match celeb.phase {
        Tpos2Phase::Anticipation | Tpos2Phase::Unseal => {
            let a = pack_closeup_anchor(screen, positions, box_h, Mat4::IDENTITY);
            (a.pos[0], a.pos[1], a.pos[2])
        }
        Tpos2Phase::Deal => {
            let pack_a = pack_closeup_anchor(screen, positions, box_h, Mat4::IDENTITY);
            let reveal_anchor = PlacementAnchor::new(
                [w * 0.5, h * 0.5, 0.0],
                Mat4::IDENTITY,
                &positions.celeb_pack_reveal,
                screen,
            );
            let n = celeb.tiles.len();
            if n == 0 {
                return Vec::new();
            }
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
            let row_cx = row.row_x0 + total_w * 0.5;
            let deal_t = (celeb.elapsed() / celeb.total_duration().max(0.01)).clamp(0.0, 1.0);
            let cx = pack_a.pos[0] + (row_cx - pack_a.pos[0]) * deal_t;
            (cx, reveal_anchor.pos[1], row_lift)
        }
        Tpos2Phase::Arrival => return Vec::new(),
    };

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
        radius: h * 2.4 + box_h,
        cos_outer,
        cos_inner,
        color: warm,
        intensity,
    }]
}

fn tpos2_point_lights(
    celeb: &PackCelebrationV2,
    screen: &LayoutResult,
    positions: &ShopPositions,
) -> Vec<PointLight> {
    let w = screen.window_w;
    let h = screen.window_h;
    let box_h = h * PACK_CELEB_BOX_H_FRAC;
    let (cx, row_py, lift) = match celeb.phase {
        Tpos2Phase::Anticipation | Tpos2Phase::Unseal => {
            let a = pack_closeup_anchor(screen, positions, box_h, Mat4::IDENTITY);
            (a.pos[0], a.pos[1], a.pos[2])
        }
        Tpos2Phase::Deal | Tpos2Phase::Arrival => {
            let a = PlacementAnchor::new(
                [w * 0.5, h * 0.5, 0.0],
                Mat4::IDENTITY,
                &positions.celeb_pack_reveal,
                screen,
            );
            (a.pos[0], a.pos[1], a.pos[2])
        }
    };
    let (i_mul, r_mul, close_z) = match celeb.phase {
        Tpos2Phase::Anticipation | Tpos2Phase::Unseal => (2.6, 1.4, box_h * 0.22),
        Tpos2Phase::Deal => (1.9, 1.15, 0.0),
        Tpos2Phase::Arrival => (1.2, 1.0, 0.0),
    };
    let foil = pack_palette::for_kind(celeb.pack_kind).foil;
    vec![
        PointLight {
            pos: [
                cx + w * 0.14,
                row_py - h * 0.22 - box_h * 0.04,
                lift + h * 0.48 + close_z,
            ],
            radius: h * 3.2 * r_mul + box_h * 0.5 * r_mul,
            color: color::rgb(color::PARCHMENT),
            intensity: 1.55 * i_mul,
        },
        PointLight {
            pos: [
                cx,
                row_py + h * 0.12 + box_h * 0.05,
                lift + h * 0.28 + close_z * 0.6,
            ],
            radius: h * 2.5 * r_mul,
            color: [
                foil[0] * 0.5 + 0.25,
                foil[1] * 0.5 + 0.35,
                foil[2] * 0.5 + 0.55,
            ],
            intensity: 1.2 * i_mul,
        },
        PointLight {
            pos: [
                cx - w * 0.16,
                row_py + h * 0.06 + box_h * 0.03,
                lift + h * 0.32 + close_z * 0.7,
            ],
            radius: h * 2.8 * r_mul + box_h * 0.45 * r_mul,
            color: [0.70, 0.82, 1.0],
            intensity: 1.15 * i_mul,
        },
    ]
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn ease_out_cubic(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3)
}
