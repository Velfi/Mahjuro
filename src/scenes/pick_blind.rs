//! Pick-blind scene — choose **Play** or **Skip** before the next blind.
//! Renders the [`hallway.glb`](../../assets/3d/hallway.glb) room with embedded
//! punctual lights when the asset is present; copy and focus navigation use
//! the GLB `btn_play_round` / `btn_skip_round` hit targets.
//!
//! Mirrors the shop scene's `draw_frame() -> UiFrame` pattern: a custom
//! camera from [`hallway_glb::hallway_camera_base`], [`DrawCmd::HallwayEnvironment`],
//! and a 2D HUD (side panels, skip context) on top.

use crate::audio::SfxId;
use crate::core::rules::BlindKind;
use crate::game::engine::{GameCommand, GameEngine};
use crate::game::event_bus::GameEvent;
use crate::render::draw_cmd::{ScenePunctualLight, UiFrame};
use crate::render::hallway_glb::{self, BTN_PLAY_ROUND, BTN_SKIP_ROUND};
use crate::render::shop_glb;
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextLabel};
use crate::ui::focus_nav::push_focus_ring;
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::gameplay::GameplayScene;
use super::pause_menu::PauseMenu;
use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

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
}

impl PickBlindScene {
    pub fn new() -> Self {
        let mut tree = TreeState::new();
        tree.set_focus(BlindAction::PlayBlind.id());
        Self {
            tree,
            pause_menu: PauseMenu::new(),
        }
    }

    fn can_skip(blind: BlindKind) -> bool {
        !matches!(blind, BlindKind::Boss)
    }

    fn flat_items_hallway(
        win_w: f32,
        win_h: f32,
        cam: &crate::render::draw_cmd::CameraParams,
        env_h: f32,
        can_skip: bool,
    ) -> Vec<FlatItem<BlindAction>> {
        let play_rect = hallway_button_screen_rect(win_w, win_h, cam, env_h, BTN_PLAY_ROUND)
            .map(inflate_hit_rect)
            .unwrap_or_else(|| [win_w * 0.12, win_h * 0.52, win_w * 0.30, win_h * 0.20]);
        let mut items = vec![FlatItem::new(
            BlindAction::PlayBlind.id(),
            play_rect,
            BlindAction::PlayBlind,
        )];
        if can_skip {
            let skip_rect = hallway_button_screen_rect(win_w, win_h, cam, env_h, BTN_SKIP_ROUND)
                .map(inflate_hit_rect)
                .unwrap_or_else(|| [win_w * 0.58, win_h * 0.52, win_w * 0.30, win_h * 0.20]);
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

#[inline]
fn pick_blind_hallway_loaded() -> bool {
    hallway_glb::with_hallway_glb_cpu(|o| o.is_some())
}

/// Screen-space hit rect for a named hallway marker (projected mesh AABB, with legacy min size).
fn hallway_button_screen_rect(
    win_w: f32,
    win_h: f32,
    cam: &crate::render::draw_cmd::CameraParams,
    env_height_scale: f32,
    node_name: &str,
) -> Option<[f32; 4]> {
    let min_rw = (win_w * 0.11).max(80.0);
    let min_rh = (win_h * 0.07).max(52.0);
    hallway_glb::with_hallway_glb_cpu(|opt| {
        let cpu = opt?;
        if let Some(r) = shop_glb::screen_rect_for_marker_mesh_bounds(
            win_w,
            win_h,
            cam,
            env_height_scale,
            cpu,
            node_name,
            min_rw,
            min_rh,
        ) {
            return Some(r);
        }
        let tw = hallway_glb::hallway_marker_world(win_h, env_height_scale, cpu, node_name)?;
        let (cx, cy) = cam.project_world_to_screen(win_w, win_h, tw);
        Some([cx - min_rw * 0.5, cy - min_rh * 0.5, min_rw, min_rh])
    })
}

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
            *ctx.overlay_request = Some(super::OverlayRequest::Push(Box::new(Scene::Showcase(
                super::ShowcaseScene::new(super::ShowcasePresenter::Zodiac(
                    super::ZodiacPresenter::new(kind, yaku.name(), new_level),
                )),
            ))));
            return None;
        }

        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            return t;
        }

        let pick = GameEngine::read_pick_blind(ctx.run);
        let upcoming = pick.upcoming_blind;
        let can_skip = Self::can_skip(upcoming);

        let cam = hallway_glb::hallway_camera_base(
            ctx.layout.window_w,
            ctx.layout.window_h,
            ctx.room_gltf_height_scale,
        );
        let items = Self::flat_items_hallway(
            ctx.layout.window_w,
            ctx.layout.window_h,
            &cam,
            ctx.room_gltf_height_scale,
            can_skip,
        );
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
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
            Some(BlindAction::PlayBlind) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::RoundStart));
                if upcoming == BlindKind::Boss
                    && let Some(bk) = pick.boss_kind
                {
                    ctx.bus.push(GameEvent::BossEncountered(bk));
                }
                Some(Scene::Gameplay(GameplayScene::with_pending_blind(upcoming)))
            }
            Some(BlindAction::SkipBlind) => None,
            None => None,
        }
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;

        let pick = GameEngine::read_pick_blind(ctx.run);
        let upcoming = pick.upcoming_blind;
        let can_skip = Self::can_skip(upcoming);
        let skip_tag = pick.skip_tag;

        let mut frame = UiFrame::new();
        // Pure black temple background via the synthetic 1×1 black
        // background texture, so the fill draws in pass A *before* the
        // smoke composite. A fullscreen quad would be reordered into the
        // late HUD overlay pass and paint over the smoke. (Earlier we
        // layered dark indigo + vignettes, but the gamma-encoded linear
        // floor of even [0.002] reads as visible indigo on screen.)
        frame.background(BackgroundId::Black);
        if ctx.effect_layers.ember_drift {
            frame.ember_drift();
        }
        let hallway = pick_blind_hallway_loaded();
        let cam = hallway_glb::hallway_camera_base(w, h, ctx.room_gltf_height_scale);
        frame.camera_override = Some(cam);

        if hallway {
            frame.hallway_environment();
            let room_glb = hallway_glb::hallway_glb_has_embedded_lights();
            frame.scene_lighting.embedded_gltf_punctual = room_glb;
            frame.scene_lighting.room_shop_glb_brdf = room_glb;
            frame.scene_lighting.spot_lights = if room_glb {
                hallway_glb::hallway_embedded_spot_lights_runtime(
                    w,
                    h,
                    ctx.room_gltf_height_scale,
                    &ctx.shop_env_lighting,
                )
            } else {
                Vec::new()
            };
            let mut inverse_punctual: Vec<ScenePunctualLight> = if room_glb {
                hallway_glb::hallway_embedded_point_lights_runtime(
                    w,
                    h,
                    ctx.room_gltf_height_scale,
                    &ctx.shop_env_lighting,
                )
                .into_iter()
                .map(ScenePunctualLight::InverseSquare)
                .collect()
            } else {
                Vec::new()
            };
            let mut point_lights: Vec<PointLight> = if room_glb {
                Vec::new()
            } else {
                vec![
                    PointLight {
                        pos: [w * 0.5, h * 0.40, h * 0.50],
                        radius: h * 1.30,
                        color: [1.0, 0.94, 0.82],
                        intensity: 1.70,
                    },
                    PointLight {
                        pos: [w * 0.5, h * 0.78, h * 0.22],
                        radius: h * 2.0,
                        color: [0.42, 0.52, 0.88],
                        intensity: 0.40,
                    },
                ]
            };
            let play_on = self.play_focused();
            let skip_on = self.skip_focused() && can_skip;
            let play_b = if play_on { 1.9 } else { 1.0 };
            let skip_b = if skip_on { 1.9 } else { 1.0 };
            if let Some(r) =
                hallway_button_screen_rect(w, h, &cam, ctx.room_gltf_height_scale, BTN_PLAY_ROUND)
            {
                let cx = r[0] + r[2] * 0.5;
                let cy = r[1] + r[3] * 0.5;
                point_lights.push(PointLight {
                    pos: [cx, cy - 18.0, h * 0.19 + 55.0],
                    radius: r[2].max(r[3]) * 2.4,
                    color: [1.0, 0.90, 0.58],
                    intensity: (if room_glb { 0.42 } else { 1.22 }) * play_b,
                });
            }
            if can_skip {
                if let Some(r) =
                    hallway_button_screen_rect(w, h, &cam, ctx.room_gltf_height_scale, BTN_SKIP_ROUND)
                {
                    let cx = r[0] + r[2] * 0.5;
                    let cy = r[1] + r[3] * 0.5;
                    point_lights.push(PointLight {
                        pos: [cx, cy - 18.0, h * 0.19 + 50.0],
                        radius: r[2].max(r[3]) * 2.2,
                        color: [1.0, 0.86, 0.52],
                        intensity: (if room_glb { 0.36 } else { 1.05 }) * skip_b,
                    });
                }
            }
            inverse_punctual.extend(
                point_lights
                    .into_iter()
                    .map(ScenePunctualLight::Smooth),
            );
            frame.scene_lighting.punctual = inverse_punctual;
        } else {
            frame.scene_lighting.embedded_gltf_punctual = false;
            frame.scene_lighting.room_shop_glb_brdf = false;
            frame.scene_lighting.spot_lights.clear();
            frame.scene_lighting.set_smooth_points(vec![
                PointLight {
                    pos: [w * 0.5, h * 0.40, h * 0.50],
                    radius: h * 1.30,
                    color: [1.0, 0.94, 0.82],
                    intensity: 1.70,
                },
                PointLight {
                    pos: [w * 0.5, h * 0.78, h * 0.22],
                    radius: h * 2.0,
                    color: [0.42, 0.52, 0.88],
                    intensity: 0.40,
                },
            ]);
        }

        // ── Minimal 2D HUD ────────────────────────────────────────────
        // Side columns beside the GLB play/skip buttons (ante, target, reward;
        // skip tag when present).
        let mut quads: Vec<GpuInstance> = Vec::new();
        let mut texts: Vec<TextLabel> = Vec::new();
        let mut buttons: Vec<ButtonDef> = Vec::new();

        // ── Caption labels: hallway side panels ─
        let play_focused_label = self.play_focused();
        let skip_focused_label = self.skip_focused();

        let (play_anchor_rect, skip_anchor_rect) = {
            let pr = hallway_button_screen_rect(w, h, &cam, ctx.room_gltf_height_scale, BTN_PLAY_ROUND)
                .unwrap_or_else(|| [w * 0.12, h * 0.50, w * 0.14, h * 0.10]);
            let sr = if can_skip {
                hallway_button_screen_rect(w, h, &cam, ctx.room_gltf_height_scale, BTN_SKIP_ROUND)
                    .unwrap_or_else(|| [w * 0.74, h * 0.50, w * 0.14, h * 0.10])
            } else {
                [0.0, 0.0, 1.0, 1.0]
            };
            // Hallway side columns: pin `font_px` so long lines (ante / rewards) are not
            // crushed by the legacy auto-shrink width/char cap (see font-scaling agent note).
            let side_w = (w * 0.30).clamp(280.0, 520.0);
            let px_action = typography::size(typography::HEADING, h) * 1.45;
            let px_blind = typography::size(typography::TITLE, h) * 1.12;
            let px_detail = typography::size(typography::BODY, h) * 1.18;
            let h_action = (px_action * 1.38).max(22.0);
            let h_blind = (px_blind * 1.42).max(26.0);
            let h_detail = (px_detail * 1.36).max(22.0);
            let base_target = pick.base_target;
            let upcoming_run_number = pick.run_number;
            let blind_display = if upcoming == BlindKind::Boss {
                pick.boss_name.clone().unwrap_or_else(|| "Boss Blind".to_string())
            } else {
                upcoming.name().to_string()
            };
            let target_value = base_target.saturating_mul(upcoming_run_number);
            let stake_suffix = match ctx.run.mode.stake {
                crate::core::stake::Stake::Spring => String::new(),
                other => format!(" · {}", other.label()),
            };
            let lx_play = (pr[0] + pr[2] + 18.0).min(w - side_w - 10.0);
            let mut ly_play = pr[1].max(10.0);
            texts.push(TextLabel {
                rect: [lx_play, ly_play, side_w, h_action],
                text: "Play round".to_string(),
                color: if play_focused_label {
                    color::CHAMPAGNE
                } else {
                    color::PARCHMENT
                },
                font_px: Some(px_action),
                ..Default::default()
            });
            ly_play += h_action + 4.0;
            texts.push(TextLabel {
                rect: [lx_play, ly_play, side_w, h_blind],
                text: blind_display,
                color: if play_focused_label { color::GOLD } else { color::STONE },
                no_glossary: true,
                font_px: Some(px_blind),
                ..Default::default()
            });
            ly_play += h_blind + 6.0;
            texts.push(TextLabel {
                rect: [lx_play, ly_play, side_w, h_detail],
                text: format!(
                    "Ante {}/{} · Target {}",
                    pick.ante,
                    crate::game::run::FINAL_ANTE,
                    target_value,
                ),
                color: if play_focused_label { color::GOLD } else { color::STONE },
                no_glossary: true,
                font_px: Some(px_detail),
                ..Default::default()
            });
            ly_play += h_detail + 4.0;
            texts.push(TextLabel {
                rect: [lx_play, ly_play, side_w, h_detail],
                text: format!("Reward ${}{}", upcoming.clear_reward(), stake_suffix),
                color: if play_focused_label { color::GOLD } else { color::STONE },
                no_glossary: true,
                font_px: Some(px_detail),
                ..Default::default()
            });
            ly_play += h_detail + 4.0;
            if upcoming == BlindKind::Boss {
                if let Some(desc) = pick.boss_description.as_deref() {
                    texts.push(TextLabel {
                        rect: [lx_play, ly_play, side_w, h_detail * 1.35],
                        text: desc.to_string(),
                        color: color::AMBER,
                        font_px: Some(px_detail),
                        ..Default::default()
                    });
                    ly_play += h_detail * 1.35 + 4.0;
                }
                if let Some(tier) = pick.boss_tier_label {
                    texts.push(TextLabel {
                        rect: [lx_play, ly_play, side_w, h_detail],
                        text: format!("[{}]", tier),
                        color: color::AMBER,
                        font_px: Some(px_detail),
                        ..Default::default()
                    });
                }
            }
            if can_skip {
                let lx_skip = (sr[0] - side_w - 18.0).max(10.0);
                let mut ly_skip = sr[1].max(10.0);
                if let Some(tag) = skip_tag {
                    texts.push(TextLabel {
                        rect: [lx_skip, ly_skip, side_w, h_action],
                        text: "Skip round".to_string(),
                        color: if skip_focused_label {
                            color::CHAMPAGNE
                        } else {
                            color::PARCHMENT
                        },
                        font_px: Some(px_action),
                        ..Default::default()
                    });
                    ly_skip += h_action + 4.0;
                    texts.push(TextLabel {
                        rect: [lx_skip, ly_skip, side_w, h_blind],
                        text: tag.name().to_string(),
                        color: if skip_focused_label { color::GOLD } else { color::STONE },
                        font_px: Some(px_blind),
                        ..Default::default()
                    });
                    ly_skip += h_blind + 6.0;
                    texts.push(TextLabel {
                        rect: [lx_skip, ly_skip, side_w, h_detail * 1.45],
                        text: tag.description().to_string(),
                        color: if skip_focused_label { color::GOLD } else { color::STONE },
                        font_px: Some(px_detail),
                        ..Default::default()
                    });
                } else {
                    texts.push(TextLabel {
                        rect: [lx_skip, ly_skip, side_w, h_action],
                        text: "Skip round".to_string(),
                        color: if skip_focused_label {
                            color::CHAMPAGNE
                        } else {
                            color::PARCHMENT
                        },
                        font_px: Some(px_action),
                        ..Default::default()
                    });
                    ly_skip += h_action + 4.0;
                    texts.push(TextLabel {
                        rect: [lx_skip, ly_skip, side_w, h_detail],
                        text: "Tribute · Esc".to_string(),
                        color: if skip_focused_label { color::GOLD } else { color::STONE },
                        font_px: Some(px_detail),
                        ..Default::default()
                    });
                }
            }
            (pr, sr)
        };

        let scale = metrics::scene_scale(w, h);

        // ── Gold outline around the focused action ────────────────────
        // A chunky gold border (3× the normal focus ring thickness)
        // around whichever control is currently selected so the player
        // can immediately read which action they're about to confirm.
        let big_ring_scale = scale * 3.0;
        if play_focused_label {
            push_focus_ring(play_anchor_rect, big_ring_scale, w, h, &mut quads);
        }
        if skip_focused_label && can_skip {
            push_focus_ring(skip_anchor_rect, big_ring_scale, w, h, &mut quads);
        }

        // Register focus-tree click targets for PlayBlind + SkipBlind.
        let items = Self::flat_items_hallway(w, h, &cam, ctx.room_gltf_height_scale, can_skip);
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
            },
            scale,
            &mut quads,
            &mut texts,
            &mut buttons,
        );
        if self.pause_menu.paused {
            buttons.push(ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }

        // Push 2D layers + metadata onto the frame.
        frame.quads(quads);
        frame.texts(texts);

        frame.buttons = buttons;
        frame.window_title = "Mahjuro".to_string();

        frame
    }
}
