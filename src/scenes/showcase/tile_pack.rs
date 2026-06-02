//! Tile-pack opening celebration on the showcase overlay.

use std::f32::consts::PI;
use std::time::{Duration, Instant};

use glam::{Mat4, Vec3};

use crate::core::tile_pack::{PACK_TILE_ID_BASE, TilePackKind};
use crate::persistence::TilePreset;
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, Object3d, Object3dKind, ShowcaseRenderHints, ShowcaseTilePlacement,
    UiFrame,
};
use crate::render::pack_palette;
use crate::render::showcase_tile_layout::{
    PackRevealRowLayoutParams, compute_pack_reveal_row_layout,
};
use crate::render::theme::color;
use crate::render::wgpu_renderer::{
    GpuInstance, GradientQuadInstance, PointLight, TextAlign, TextLabel,
};
use crate::scenes::celebration_overlay::{
    CelebrationContentDrift, CelebrationOverlayScratch, CelebrationShowcaseIntroGate,
    ShootingStarCelebrationIntro,
};
use crate::scenes::shop::pack_celebration::{PackCelebPhase, PackCelebration};
use crate::scenes::shop::shop_celebration_camera;
use crate::ui::input::UiAction;
use crate::ui::layout::LayoutResult;
use crate::ui::placement::PlacementAnchor;
use crate::ui::scene_layout::ShopPositions;

use super::super::{BackgroundId, ButtonDef, DrawCtx, OverlayRequest, SceneTransition, UpdateCtx};

const UNSEAL_DOLLY_PULL: f32 = 0.04;
/// Pack-tint fullscreen flash during the shooting-star wipe (fraction of wipe progress).
const ARRIVAL_BG_FLASH_START: f32 = 0.30;
const ARRIVAL_BG_FLASH_END: f32 = 0.46;

/// Pack mesh height vs window height (celebration hero framing).
pub(crate) const PACK_CELEB_BOX_H_FRAC: f32 = 0.62;
/// Center Y for the hero pack — clears the title band at ~0.18h without sitting too low.
const PACK_CELEB_ANCHOR_Y_FRAC: f32 = 0.57;

pub struct TilePackPresenter {
    pub celebration: PackCelebration,
    pub positions: ShopPositions,
    intro_gate: CelebrationShowcaseIntroGate,
    arrival_done_at: Option<Instant>,
}

impl TilePackPresenter {
    pub fn new(celebration: PackCelebration) -> Self {
        Self {
            celebration,
            positions: ShopPositions::default(),
            intro_gate: CelebrationShowcaseIntroGate::new(
                ShootingStarCelebrationIntro::new_pack_opening(),
            ),
            arrival_done_at: None,
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
            layout_use_ray_plane_z: true,
            tile_pack_celebration_tonemap: true,
            shop_tonemap_and_lit_mesh_context: false,
            collection_tonemap_context: false,
            modal_relic_staging: false,
        }
    }

    fn intro_content_alpha(&self, ctx: &DrawCtx<'_>) -> f32 {
        self.intro_gate.intro.content_alpha_for(&ctx.effect_layers)
    }

    fn emit_skip_to_settled(&mut self, ctx: &mut UpdateCtx<'_>) {
        let needs_opened = matches!(
            self.celebration.phase,
            PackCelebPhase::Arrival | PackCelebPhase::Anticipation
        );
        let from = self.celebration.revealed_count;
        self.celebration.skip_to_settled();
        if needs_opened {
            ctx.bus
                .push(crate::game::event_bus::GameEvent::PackOpened);
        }
        for _ in from..self.celebration.tiles.len() {
            ctx.bus
                .push(crate::game::event_bus::GameEvent::PackTileRevealed);
        }
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
        dimmer_alpha: f32,
        content_alpha: f32,
        ctx: &DrawCtx<'_>,
    ) {
        let celeb = &self.celebration;
        let drift = self
            .intro_gate
            .intro
            .content_drift_for(w, h, &ctx.effect_layers);
        let palette = pack_palette::for_kind(celeb.pack_kind);
        let n = celeb.tiles.len();

        let after_dimmer =
            CelebrationOverlayScratch::new(w, h).push_dimmer_scaled(frame, dimmer_alpha);
        after_dimmer
            .push_starfield_if(frame, ctx.effect_layers.starfield)
            .push_depth_reset_for_celebration_mesh(frame);

        if matches!(
            celeb.phase,
            PackCelebPhase::Anticipation | PackCelebPhase::Unseal | PackCelebPhase::Deal
        ) {
            let mut vignette = Vec::new();
            push_edge_vignette(&mut vignette, w, h, content_alpha * 0.85);
            frame.gradient_quads(vignette);
        }

        if matches!(celeb.phase, PackCelebPhase::Arrival) {
            let wipe_t = self.intro_gate.intro.transition_progress();
            if wipe_t > ARRIVAL_BG_FLASH_START {
                let bg_a = content_alpha
                    * smoothstep(ARRIVAL_BG_FLASH_START, ARRIVAL_BG_FLASH_END, wipe_t)
                    * 0.55;
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
            PackCelebPhase::Arrival => {
                content_alpha * smoothstep(0.5, 0.85, self.intro_gate.intro.transition_progress())
            }
            _ => content_alpha,
        };
        let push_pack_title = |frame: &mut UiFrame| {
            if title_alpha <= 0.01 {
                return;
            }
            let title_font =
                crate::render::theme::typography::size(crate::render::theme::typography::H24, h);
            frame.text(TextLabel {
                text: celeb.pack_name.to_string(),
                rect: [0.0, h * 0.18 + drift.xy[1], w, title_font * 1.5],
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
        };

        let box_h = h * PACK_CELEB_BOX_H_FRAC;

        match celeb.phase {
            PackCelebPhase::Arrival => {
                let anchor = pack_closeup_anchor(screen, &self.positions, box_h * drift.scale, Mat4::IDENTITY);
                let hero = drift.apply_to_pos(anchor.pos);
                let cx = hero[0];
                let cy = hero[1];

                let mut gradients = Vec::new();
                let mut squircles = Vec::new();
                let wipe_t = self.intro_gate.intro.transition_progress();
                let seal_pulse = 0.35 + 0.25 * smoothstep(0.35, 0.75, wipe_t);
                push_pack_aura(
                    &mut gradients,
                    &mut squircles,
                    cx,
                    cy,
                    box_h * drift.scale,
                    palette,
                    content_alpha,
                    seal_pulse,
                );
                flush_backdrop_quads(frame, gradients, squircles);

                let mut foil = celeb.pack_kind.foil_tint();
                foil[3] *= content_alpha;
                push_pack_object3d_at(
                    frame,
                    w,
                    h,
                    cam,
                    &anchor,
                    hero,
                    box_h * drift.scale,
                    celeb.pack_kind,
                    foil,
                );
                push_pack_title(frame);
            }
            PackCelebPhase::Anticipation => {
                let t = celeb.elapsed();
                let bob_x = (t * 0.7).sin() * h * 0.008;
                let bob_y = (t * 0.5).sin() * h * 0.006;
                let bob_rx = (t * 0.6).sin() * 2.5;
                let bob_ry = (t * 0.8).cos() * 3.0;
                let bob_rot = Mat4::from_rotation_y(bob_ry.to_radians())
                    * Mat4::from_rotation_x(bob_rx.to_radians());
                let anchor = pack_closeup_anchor(screen, &self.positions, box_h, bob_rot);
                let hero = drift.apply_to_pos([
                    anchor.pos[0] + bob_x,
                    anchor.pos[1] + bob_y,
                    anchor.pos[2],
                ]);
                let cx = hero[0];
                let cy = hero[1];

                let mut gradients = Vec::new();
                let mut squircles = Vec::new();
                push_pack_brand_wash(&mut gradients, w, h, palette.bg, content_alpha * 0.14);
                let seal_pulse = 0.5 + 0.5 * (t * 2.0).sin();
                push_pack_aura(
                    &mut gradients,
                    &mut squircles,
                    cx,
                    cy,
                    box_h,
                    palette,
                    content_alpha,
                    seal_pulse,
                );
                flush_backdrop_quads(frame, gradients, squircles);

                let foil = celeb.pack_kind.foil_tint();
                push_pack_object3d_at(
                    frame,
                    w,
                    h,
                    cam,
                    &anchor,
                    hero,
                    box_h,
                    celeb.pack_kind,
                    foil,
                );
                push_pack_title(frame);

            }
            PackCelebPhase::Unseal => {
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
                push_pack_title(frame);

                let mut gradients = Vec::new();
                push_unseal_burst(
                    &mut gradients,
                    anchor.pos[0],
                    anchor.pos[1],
                    h.min(w),
                    palette,
                    content_alpha,
                    u,
                );
                flush_backdrop_quads(frame, gradients, Vec::new());
            }
            PackCelebPhase::Deal => {
                push_pack_title(frame);

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

                let fan_half = PackCelebration::FAN_HALF_DEG.to_radians();
                let deal_elapsed = celeb.elapsed();
                let last_land_t = if n > 0 {
                    (n - 1) as f32 * PackCelebration::DEAL_STAGGER
                        + PackCelebration::DEAL_TILE_FLY_SECS
                } else {
                    0.0
                };
                let last_glow = deal_elapsed >= last_land_t
                    && deal_elapsed < last_land_t + PackCelebration::LAST_TILE_GLOW_SECS;

                let total_w =
                    n as f32 * row.silhouette_w + (n.saturating_sub(1)) as f32 * row.gap_px;
                let row_cx = row.row_x0 + total_w * 0.5;
                let deal_t = (deal_elapsed / celeb.total_duration().max(0.01)).clamp(0.0, 1.0);
                let wash_cx = src_px + (row_cx - src_px) * deal_t;
                let mut gradients = Vec::new();
                push_deal_row_wash(
                    &mut gradients,
                    wash_cx,
                    row_py,
                    row.tile_size,
                    total_w,
                    palette.foil,
                    content_alpha,
                );
                flush_backdrop_quads(frame, gradients, Vec::new());

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

            }
        }
    }

    pub fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        self.intro_gate.tick(&mut ctx);

        if ctx.headless {
            self.intro_gate.skip_intro();
            if self.celebration.phase == PackCelebPhase::Arrival {
                self.celebration.phase = PackCelebPhase::Anticipation;
                self.celebration.started_at = Instant::now();
            }
        }

        if self.celebration.phase == PackCelebPhase::Arrival
            && self.intro_gate.intro.is_done_for(&ctx.effect_layers)
        {
            if self.arrival_done_at.is_none() {
                self.arrival_done_at = Some(Instant::now());
            } else if Instant::now().saturating_duration_since(self.arrival_done_at.unwrap())
                >= Duration::from_secs_f32(CelebrationShowcaseIntroGate::GRACE_AFTER_DONE_SECS)
            {
                self.celebration.phase = PackCelebPhase::Anticipation;
                self.celebration.started_at = Instant::now();
                self.arrival_done_at = None;
            }
        }

        let confirm_or_click = ctx.actions.iter().any(|a| {
            matches!(a, UiAction::Confirm | UiAction::CommitDiscard)
        }) || !ctx.button_clicks.is_empty();
        let cancel = ctx.actions.iter().any(|a| matches!(a, UiAction::Cancel));

        if confirm_or_click || cancel {
            self.intro_gate.skip_intro();
            match self.celebration.phase {
                PackCelebPhase::Arrival => {
                    self.celebration.phase = PackCelebPhase::Anticipation;
                    self.celebration.started_at = Instant::now();
                    self.arrival_done_at = None;
                }
                PackCelebPhase::Anticipation if confirm_or_click => {
                    self.celebration.phase = PackCelebPhase::Unseal;
                    self.celebration.started_at = Instant::now();
                    ctx.bus
                        .push(crate::game::event_bus::GameEvent::PackOpened);
                }
                PackCelebPhase::Anticipation => {
                    self.emit_skip_to_settled(&mut ctx);
                }
                PackCelebPhase::Unseal => {
                    self.emit_skip_to_settled(&mut ctx);
                }
                PackCelebPhase::Deal if self.celebration.fully_settled() => {
                    ctx.run.pending_shop_focus_snap_after_celebration = true;
                    *ctx.overlay_request = Some(OverlayRequest::Pop);
                }
                PackCelebPhase::Deal => {
                    self.emit_skip_to_settled(&mut ctx);
                }
            }
            return None;
        }

        match self.celebration.phase {
            PackCelebPhase::Arrival | PackCelebPhase::Anticipation => {}
            PackCelebPhase::Unseal => {
                if self.celebration.elapsed() >= PackCelebration::UNSEAL_SECS {
                    self.celebration.phase = PackCelebPhase::Deal;
                    self.celebration.started_at = Instant::now();
                }
            }
            PackCelebPhase::Deal => {
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
        let dimmer_alpha = pack_celebration_dimmer_alpha(content_alpha, &self.celebration);
        let dolly = match self.celebration.phase {
            PackCelebPhase::Unseal => {
                UNSEAL_DOLLY_PULL * ease_out_cubic(self.celebration.unseal_t())
            }
            PackCelebPhase::Deal => UNSEAL_DOLLY_PULL,
            _ => 0.0,
        };
        let cam = self.camera_with_dolly(w, h, env_h, dolly);

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.camera_override = Some(cam);
        frame.scene_lighting.embedded_gltf_punctual = false;
        frame.scene_lighting.room_glb_brdf = false;
        let drift = self
            .intro_gate
            .intro
            .content_drift_for(w, h, &ctx.effect_layers);
        frame
            .scene_lighting
            .set_smooth_points(pack_celebration_point_lights(
                &self.celebration,
                ctx.layout,
                &self.positions,
                drift,
            ));

        self.push_celebration_draw(
            &mut frame,
            ctx.layout,
            w,
            h,
            &cam,
            ctx.tile_preset,
            dimmer_alpha,
            content_alpha,
            &ctx,
        );

        self.intro_gate
            .intro
            .push_shooting_star_cascade_if_active(&mut frame, &ctx.effect_layers);

        frame.window_title = "Mahjuro".to_string();
        frame.showcase_render_hints = Self::render_hints();

        frame.buttons = vec![ButtonDef::scene((0.0, 0.0, w, h), u32::MAX)];
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

    let split = PackCelebration::ARC_SPLIT;

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

fn pack_hero_center(
    screen: &LayoutResult,
    positions: &ShopPositions,
    box_h: f32,
    drift: CelebrationContentDrift,
    bob_xy: (f32, f32),
) -> (f32, f32, f32) {
    let anchor = pack_closeup_anchor(screen, positions, box_h, Mat4::IDENTITY);
    let p = drift.apply_to_pos([
        anchor.pos[0] + bob_xy.0,
        anchor.pos[1] + bob_xy.1,
        anchor.pos[2],
    ]);
    (p[0], p[1], p[2])
}

fn pack_celebration_point_lights(
    celeb: &PackCelebration,
    screen: &LayoutResult,
    positions: &ShopPositions,
    drift: CelebrationContentDrift,
) -> Vec<PointLight> {
    let w = screen.window_w;
    let h = screen.window_h;
    let box_h = h * PACK_CELEB_BOX_H_FRAC;
    let (cx, row_py, lift) = match celeb.phase {
        PackCelebPhase::Arrival | PackCelebPhase::Anticipation | PackCelebPhase::Unseal => {
            pack_hero_center(screen, positions, box_h, drift, (0.0, 0.0))
        }
        PackCelebPhase::Deal => {
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
        PackCelebPhase::Arrival => (1.6, 1.15, box_h * 0.12),
        PackCelebPhase::Anticipation | PackCelebPhase::Unseal => (2.6, 1.4, box_h * 0.22),
        PackCelebPhase::Deal => (1.9, 1.15, 0.0),
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

/// Brief backdrop deepen during the unseal punch.
fn pack_celebration_dimmer_alpha(content_alpha: f32, celeb: &PackCelebration) -> f32 {
    if celeb.phase != PackCelebPhase::Unseal {
        return content_alpha;
    }
    let u = celeb.unseal_t();
    let bump = smoothstep(0.0, 0.12, u) * (1.0 - smoothstep(0.2, 0.35, u));
    (content_alpha * (1.0 + 0.15 * bump)).min(0.85)
}

fn flush_backdrop_quads(
    frame: &mut UiFrame,
    gradients: Vec<GradientQuadInstance>,
    squircles: Vec<GpuInstance>,
) {
    if !gradients.is_empty() {
        frame.gradient_quads(gradients);
    }
    if !squircles.is_empty() {
        frame.squircle_quads(squircles);
    }
}

fn push_radial_gradient(
    out: &mut Vec<GradientQuadInstance>,
    cx: f32,
    cy: f32,
    size: f32,
    color: [f32; 4],
    feather_edge: f32,
) {
    if color[3] <= 0.001 {
        return;
    }
    out.push(GradientQuadInstance {
        rect: [cx - size * 0.5, cy - size * 0.5, size, size],
        color,
        feather: [feather_edge.clamp(0.05, 0.99), 1.0, 0.0, 0.0],
    });
}

/// Darkens the four screen edges so the hero reads against the dimmer.
fn push_edge_vignette(out: &mut Vec<GradientQuadInstance>, w: f32, h: f32, alpha: f32) {
    if alpha <= 0.001 {
        return;
    }
    let edge = 0.38_f32;
    let band = (w.min(h) * edge).max(48.0);
    let c = [0.02, 0.025, 0.05, 0.42 * alpha];
    let feather = [0.92, 0.0, 0.0, 0.0];
    out.push(GradientQuadInstance {
        rect: [0.0, 0.0, w, band],
        color: c,
        feather,
    });
    out.push(GradientQuadInstance {
        rect: [0.0, h - band, w, band],
        color: c,
        feather,
    });
    out.push(GradientQuadInstance {
        rect: [0.0, 0.0, band, h],
        color: c,
        feather,
    });
    out.push(GradientQuadInstance {
        rect: [w - band, 0.0, band, h],
        color: c,
        feather,
    });
}

fn push_pack_brand_wash(
    out: &mut Vec<GradientQuadInstance>,
    w: f32,
    h: f32,
    mut bg: [f32; 4],
    alpha: f32,
) {
    if alpha <= 0.001 {
        return;
    }
    bg[3] *= alpha;
    out.push(GradientQuadInstance {
        rect: [0.0, 0.0, w, h],
        color: bg,
        feather: [0.88, 1.0, 0.0, 0.0],
    });
}

fn push_pack_aura(
    gradients: &mut Vec<GradientQuadInstance>,
    squircles: &mut Vec<GpuInstance>,
    cx: f32,
    cy: f32,
    box_h: f32,
    palette: pack_palette::PackPalette,
    content_alpha: f32,
    seal_pulse: f32,
) {
    let wide = box_h * 1.85;
    let tight = box_h * 1.05;
    let seal_a = content_alpha * (0.10 + 0.07 * seal_pulse);
    let mut seal = palette.seal;
    seal[3] = seal_a;
    push_radial_gradient(gradients, cx, cy, wide, seal, 0.78);

    let mut foil = palette.foil;
    foil[3] = content_alpha * 0.11;
    push_radial_gradient(gradients, cx, cy - box_h * 0.04, tight, foil, 0.62);

    let plate = box_h * 0.72;
    let mut plate_c = palette.seal;
    plate_c[3] = content_alpha * (0.09 + 0.04 * seal_pulse);
    squircles.push(GpuInstance {
        rect: [cx - plate * 0.5, cy - plate * 0.52, plate, plate * 1.04],
        color: plate_c,
        user: 0,
    });
}

fn push_unseal_burst(
    out: &mut Vec<GradientQuadInstance>,
    cx: f32,
    cy: f32,
    min_axis: f32,
    palette: pack_palette::PackPalette,
    content_alpha: f32,
    u: f32,
) {
    let envelope = smoothstep(0.05, 0.2, u) * (1.0 - smoothstep(0.35, 0.5, u));
    let burst_a = content_alpha * envelope * 0.32;
    if burst_a <= 0.001 {
        return;
    }
    let r = min_axis * 0.45 * smoothstep(0.0, 0.35, u);
    let mut seal = palette.seal;
    seal[3] = burst_a;
    push_radial_gradient(out, cx, cy, r * 2.0, seal, 0.55);

    let mut foil = palette.foil;
    foil[0] = (foil[0] + 0.35).min(1.35);
    foil[1] = (foil[1] + 0.35).min(1.35);
    foil[2] = (foil[2] + 0.35).min(1.35);
    foil[3] = burst_a * 0.55;
    push_radial_gradient(out, cx, cy, r * 1.35, foil, 0.42);
}

fn push_deal_row_wash(
    out: &mut Vec<GradientQuadInstance>,
    row_cx: f32,
    row_py: f32,
    tile_size: f32,
    row_span_px: f32,
    foil: [f32; 4],
    content_alpha: f32,
) {
    let a = content_alpha * 0.14;
    if a <= 0.001 {
        return;
    }
    let wash_w = (row_span_px + tile_size * 2.4).max(tile_size * 4.0);
    let wash_h = tile_size * 2.8;
    let mut warm = foil;
    warm[3] = a;
    push_radial_gradient(out, row_cx, row_py, wash_w.max(wash_h), warm, 0.72);
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
    use glam::Vec3;

    let center =
        crate::render::world_space::world_on_camera_ray_plane_z(win_w, win_h, cam, px, py, plane_z);
    let eye = Vec3::from_array(cam.eye);
    let dist = (eye - center).length().max(1.0);
    let h = win_h.max(1e-6);
    let w = win_w.max(1e-6);
    let fov_y = cam.fovy_deg.to_radians();
    let visible_h = 2.0 * dist * (fov_y * 0.5).tan();
    let visible_w = visible_h * (w / h);
    let world_h = (screen_h_px / h) * visible_h;
    let screen_w_px = screen_h_px * crate::core::tile_pack::PACK_ASPECT_W_OVER_H;
    let world_w = (screen_w_px / w) * visible_w;
    [world_w.max(1.0), world_h * 0.10, world_h.max(1.0)]
}

pub(crate) fn pack_closeup_anchor(
    screen: &LayoutResult,
    _positions: &ShopPositions,
    _box_h: f32,
    base_rotation: Mat4,
) -> PlacementAnchor {
    let h = screen.window_h;
    let w = screen.window_w;
    PlacementAnchor::new(
        [w * 0.5, h * PACK_CELEB_ANCHOR_Y_FRAC, 0.0],
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
    push_pack_object3d_at(frame, win_w, win_h, cam, anchor, pos, screen_h_px, pack_kind, color);
}

fn push_pack_object3d_at(
    frame: &mut UiFrame,
    win_w: f32,
    win_h: f32,
    cam: &CameraParams,
    anchor: &PlacementAnchor,
    pos: [f32; 3],
    screen_h_px: f32,
    pack_kind: TilePackKind,
    color: [f32; 4],
) {
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

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn ease_out_cubic(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3)
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
        let rot = anchor.object3d_rotation();
        let oriented = crate::render::table_transform::rot_euler_xyz_rad(rot[0], rot[1], rot[2]);
        let model = translate_rot_scale(center, oriented, Vec3::from(extents));

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
