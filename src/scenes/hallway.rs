//! Pick-blind scene — choose **Play** or **Skip** before the next blind.
//! Renders the [`hallway.glb`](../../assets/3d/hallway.glb) room with embedded
//! punctual lights when the asset is present; copy and focus navigation use
//! the GLB play/skip hit targets ([`hallway_glb::hallway_pick_chamber_play_button_node`],
//! `btn_skip_round`).
//!
//! Mirrors the shop scene's `draw_frame() -> UiFrame` pattern: a custom
//! camera from [`hallway_glb::hallway_camera_pick_chamber`], [`DrawCmd::HallwayEnvironment`],
//! and a 2D HUD (side panels, skip context) on top.

use crate::core::rules::ChamberKind;
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::render::decal::{load_ui_font, measure_label_advances};
use crate::render::draw_cmd::{ImageQuad, ScenePunctualLight, UiFrame};
use crate::render::hallway_glb::{self, BTN_SKIP_ROUND};
use crate::render::room_glb;
use crate::render::scene_keys;
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, TextAlign, TextLabel};
use crate::sfx_id::SfxId;
use crate::ui::colored_keywords;
use crate::ui::controller_hints::{HintStyle, menu_footer_row, push_screen_footer_hint};
use crate::ui::focus_nav::push_focus_ring;
use crate::ui::input::UiAction;
use crate::ui::score_format::format_score;
use crate::ui::ordeal_icons::ordeal_icon_source;
use crate::ui::skip_tag_icons::skip_tag_icon_source;
use crate::ui::widget::wrap_text;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::pause_menu::PauseMenu;
use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChamberAction {
    PlayChamber,
    SkipChamber,
}

impl ChamberAction {
    fn id(self) -> FocusId {
        FocusId(self as u32 + 1)
    }
}

pub struct HallwayScene {
    tree: TreeState,
    pause_menu: PauseMenu,
}

impl Default for HallwayScene {
    fn default() -> Self {
        Self::new()
    }
}

impl HallwayScene {
    pub fn new() -> Self {
        let mut tree = TreeState::new();
        tree.set_focus(ChamberAction::PlayChamber.id());
        Self {
            tree,
            pause_menu: PauseMenu::new(),
        }
    }

    fn can_skip(blind: ChamberKind) -> bool {
        !matches!(blind, ChamberKind::Ordeal)
    }

    fn flat_items_hallway(
        win_w: f32,
        win_h: f32,
        cam: &crate::render::draw_cmd::CameraParams,
        env_h: f32,
        can_skip: bool,
        play_node: &'static str,
    ) -> Vec<FlatItem<ChamberAction>> {
        let play_rect = hallway_button_screen_rect(win_w, win_h, cam, env_h, play_node)
            .map(inflate_hit_rect)
            .unwrap_or_else(|| [win_w * 0.12, win_h * 0.52, win_w * 0.30, win_h * 0.20]);
        let mut items = vec![FlatItem::new(
            ChamberAction::PlayChamber.id(),
            play_rect,
            ChamberAction::PlayChamber,
        )];
        if can_skip {
            let skip_rect = hallway_button_screen_rect(win_w, win_h, cam, env_h, BTN_SKIP_ROUND)
                .map(inflate_hit_rect)
                .unwrap_or_else(|| [win_w * 0.58, win_h * 0.52, win_w * 0.30, win_h * 0.20]);
            items.push(FlatItem::new(
                ChamberAction::SkipChamber.id(),
                skip_rect,
                ChamberAction::SkipChamber,
            ));
        }
        items
    }

    fn skip_focused(&self) -> bool {
        self.tree.focused() == Some(ChamberAction::SkipChamber.id())
    }

    fn play_focused(&self) -> bool {
        self.tree.focused() == Some(ChamberAction::PlayChamber.id())
    }
}

#[inline]
fn pick_chamber_hallway_loaded() -> bool {
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
        if let Some(r) =
            room_glb::screen_rect_for_marker_mesh_bounds(&room_glb::MarkerScreenRectParams {
                win_w,
                win_h,
                cam,
                env_height_scale,
                cpu,
                node_name,
                min_rw,
                min_rh,
            })
        {
            return Some(r);
        }
        let tw = hallway_glb::hallway_marker_world(win_h, env_height_scale, cpu, node_name)?;
        let (cx, cy) = cam.project_world_to_screen(win_w, win_h, tw);
        Some([cx - min_rw * 0.5, cy - min_rh * 0.5, min_rw, min_rh])
    })
}

fn wrapped_text_height(text: &str, col_w: f32, font_px: f32, _line_h: f32) -> f32 {
    let default = [0.0; 4];
    let wrapped =
        colored_keywords::wrap_colored_text_multiline(text, col_w, font_px / 0.99, default, false);
    colored_keywords::colored_wrapped_rows_height(&wrapped, font_px)
}

struct WrappedColumnLine<'a> {
    texts: &'a mut Vec<TextLabel>,
    x: f32,
    y: &'a mut f32,
    col_w: f32,
    font_px: f32,
    line_h: f32,
    text: &'a str,
    color: [f32; 4],
    align: TextAlign,
}

fn push_wrapped_column_line(line: WrappedColumnLine<'_>) {
    let WrappedColumnLine {
        texts,
        x,
        y,
        col_w,
        font_px,
        line_h: _line_h,
        text,
        color,
        align,
    } = line;
    let wrapped = colored_keywords::wrap_colored_text_multiline(text, col_w, font_px / 0.99, color, false);
    let block_h = colored_keywords::colored_wrapped_rows_height(&wrapped, font_px);
    colored_keywords::push_colored_rows_in_width(
        texts,
        colored_keywords::ColoredRowsLayout {
            text_left: x,
            top_y: *y,
            inner_w: col_w,
            line_h: font_px,
            fallback_plain: text,
            fallback_color: color,
            italic: false,
        },
        &wrapped,
        align,
    );
    *y += block_h;
}

/// Hallway button caption: one letter per line (e.g. `P` / `L` / `A` / `Y`), upright
/// on screen (no quad rotation). Uses `raw` screen AABB — no portrait→landscape swap,
/// so side-wall hit boxes keep height for the vertical stack.
fn hallway_button_text_label(raw: [f32; 4], text: &str, color: [f32; 4]) -> TextLabel {
    let rect = raw;
    let w = rect[2];
    let h = rect[3];
    let letters: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let stacked: String = letters
        .chars()
        .map(|ch| ch.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let line_count = stacked.lines().count().max(1) as f32;
    let max_line_chars = stacked
        .lines()
        .map(|line| line.chars().count().max(1))
        .max()
        .unwrap_or(1) as f32;
    // Multiline stacks on rect height — share vertical budget across lines (same
    // spirit as `rasterize_block`'s height / line_count).
    let font_from_height = (h * 0.55 / line_count).max(8.0);
    let font_from_width = w * 1.35 / max_line_chars;
    let font_px = font_from_height.min(font_from_width).max(10.0);
    TextLabel {
        rect,
        text: stacked,
        color,
        font_px: Some(font_px),
        align: TextAlign::Center,
        rotation_quarters: 0,
        ..Default::default()
    }
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

impl SceneBehavior for HallwayScene {
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

        if ctx.run.upcoming_chamber == ChamberKind::Ordeal {
            ctx.run.ensure_ordeal_revealed();
        }
        let pick = GameEngine::read_hallway(ctx.run);
        let upcoming = pick.upcoming_chamber;
        let upcoming_ordeal = upcoming == ChamberKind::Ordeal;
        let can_skip = Self::can_skip(upcoming);
        let hallway_loaded = pick_chamber_hallway_loaded();
        let play_node = if hallway_loaded {
            hallway_glb::hallway_pick_chamber_play_button_node(upcoming_ordeal)
        } else {
            hallway_glb::BTN_PLAY_ROUND
        };

        let cam = hallway_glb::hallway_camera_pick_chamber(
            ctx.layout.window_w,
            ctx.layout.window_h,
            ctx.room_gltf_height_scale,
            upcoming_ordeal,
        );
        let items = Self::flat_items_hallway(
            ctx.layout.window_w,
            ctx.layout.window_h,
            &cam,
            ctx.room_gltf_height_scale,
            can_skip,
            play_node,
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
            if matches!(a, UiAction::Cancel) && can_skip && !self.skip_focused() {
                // Move focus to Skip; confirming still uses Confirm / click (Esc no longer skips).
                self.tree.set_focus(ChamberAction::SkipChamber.id());
                ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
                break;
            }
        }

        match action {
            Some(ChamberAction::SkipChamber) if can_skip => {
                GameEngine::skip_upcoming_chamber(ctx.run, ctx.bus);
                Some(SceneIntent::Hallway)
            }
            Some(ChamberAction::PlayChamber) => {
                ctx.bus.push(GameEvent::UiSound(SfxId::RoundStart));
                if upcoming == ChamberKind::Ordeal
                    && let Some(bk) = pick.ordeal_kind
                {
                    ctx.bus.push(GameEvent::OrdealEncountered(bk));
                }
                Some(SceneIntent::GameplayHallwayPlay)
            }
            Some(ChamberAction::SkipChamber) => None,
            None => None,
        }
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;

        let pick = GameEngine::read_hallway(ctx.run);
        let upcoming = pick.upcoming_chamber;
        let upcoming_ordeal = upcoming == ChamberKind::Ordeal;
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
        let hallway = pick_chamber_hallway_loaded();
        let play_node = if hallway {
            hallway_glb::hallway_pick_chamber_play_button_node(upcoming_ordeal)
        } else {
            hallway_glb::BTN_PLAY_ROUND
        };
        let cam = hallway_glb::hallway_camera_pick_chamber(
            w,
            h,
            ctx.room_gltf_height_scale,
            upcoming_ordeal,
        );
        frame.camera_override = Some(cam);

        if hallway {
            frame.hallway_environment();
            frame.hallway_distortion = Some(if let Some(snap) = ctx.hallway_distortion_debug {
                snap.resolve(upcoming)
            } else {
                hallway_glb::HallwayDistortion::from_pick_chamber(
                    upcoming,
                    pick.run_seed,
                    pick.run_number,
                    pick.wing,
                )
            });
            if let Some(ref mut dist) = frame.hallway_distortion {
                hallway_glb::hallway_distortion_apply_glb_depth_extent(
                    dist,
                    h,
                    ctx.room_gltf_height_scale,
                );
            }
            let room_glb = hallway_glb::hallway_glb_has_embedded_lights();
            frame.scene_lighting.embedded_gltf_punctual = room_glb;
            frame.scene_lighting.room_glb_brdf = room_glb;
            frame
                .scene_lighting
                .set_gltf_embedded_spot_lights(if room_glb {
                    hallway_glb::hallway_embedded_spot_lights_runtime(
                        w,
                        h,
                        ctx.room_gltf_height_scale,
                        &ctx.room_env_for(scene_keys::HALLWAY).0,
                    )
                } else {
                    Vec::new()
                });
            let (mut inverse_punctual, mut punctual_gltf_nodes): (
                Vec<ScenePunctualLight>,
                Vec<Option<String>>,
            ) = if room_glb {
                crate::render::room_gltf_punctual::tagged_to_scene_punctual(
                    hallway_glb::hallway_embedded_point_lights_runtime_tagged(
                        w,
                        h,
                        ctx.room_gltf_height_scale,
                        &ctx.room_env_for(scene_keys::HALLWAY).0,
                    ),
                )
            } else {
                (Vec::new(), Vec::new())
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
                hallway_button_screen_rect(w, h, &cam, ctx.room_gltf_height_scale, play_node)
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
            if can_skip
                && let Some(r) = hallway_button_screen_rect(
                    w,
                    h,
                    &cam,
                    ctx.room_gltf_height_scale,
                    BTN_SKIP_ROUND,
                )
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
            let proc_count = point_lights.len();
            inverse_punctual.extend(point_lights.into_iter().map(ScenePunctualLight::Smooth));
            punctual_gltf_nodes.extend(std::iter::repeat_n(None, proc_count));
            frame.scene_lighting.punctual = inverse_punctual;
            frame.scene_lighting.punctual_gltf_nodes = punctual_gltf_nodes;
        } else {
            frame.scene_lighting.embedded_gltf_punctual = false;
            frame.scene_lighting.room_glb_brdf = false;
            frame.scene_lighting.clear_spot_lights();
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
        // Outer edge columns: blind details (left), skip tag (right).
        let mut quads: Vec<GpuInstance> = Vec::new();
        let mut texts: Vec<TextLabel> = Vec::new();
        let mut icon_cmds: Vec<ImageQuad> = Vec::new();
        let mut buttons: Vec<ButtonDef> = Vec::new();
        let scale = metrics::scene_scale(w, h);
        let hide_scene_hud = self.pause_menu.paused || ctx.hallway_distortion_debug.is_some();

        if !hide_scene_hud {
            // ── Caption labels: hallway side panels ─
            let play_focused_label = self.play_focused();
            let skip_focused_label = self.skip_focused();

            let (play_anchor_rect, skip_anchor_rect) = {
                let pr =
                    hallway_button_screen_rect(w, h, &cam, ctx.room_gltf_height_scale, play_node)
                        .unwrap_or([w * 0.12, h * 0.50, w * 0.14, h * 0.10]);
                let sr = if can_skip {
                    hallway_button_screen_rect(
                        w,
                        h,
                        &cam,
                        ctx.room_gltf_height_scale,
                        BTN_SKIP_ROUND,
                    )
                    .unwrap_or([w * 0.74, h * 0.50, w * 0.14, h * 0.10])
                } else {
                    [0.0, 0.0, 1.0, 1.0]
                };
                // Pin `font_px` so long lines (ante / rewards) are not crushed by the legacy
                // auto-shrink width/char cap (see font-scaling agent note).
                let px_blind = typography::size(typography::H20, h);
                let px_detail = typography::size(typography::H32, h);
                let h_blind = px_blind * 1.42;
                let h_detail = px_detail * 1.36;
                let target_value = pick.upcoming_target;
                let chamber_display = if upcoming == ChamberKind::Ordeal {
                    pick.ordeal_name
                        .clone()
                        .unwrap_or_else(|| "Ordeal Chamber".to_string())
                } else {
                    upcoming.name().to_string()
                };
                let season_suffix = match ctx.run.mode.season {
                    crate::core::season::Season::Spring => String::new(),
                    other => format!(" · {}", other.label()),
                };
                let edge_margin = (w * 0.025).max(14.0);
                let col_gap = 18.0;
                let play_desc_gap = (w * 0.04).max(36.0);
                let min_col_w = 120.0;
                let play_col_right = pr[0] - play_desc_gap;
                let play_col_w = (play_col_right - edge_margin).max(min_col_w);
                let lx_play = play_col_right - play_col_w;
                let ordeal_icon_px =
                    if upcoming == ChamberKind::Ordeal && pick.ordeal_kind.is_some() {
                        Some((h * 0.072).clamp(48.0, 80.0))
                    } else {
                        None
                    };
                let ordeal_icon_gap = 10.0_f32;
                let (text_x_play, text_w_play) = if let Some(ipx) = ordeal_icon_px {
                    (
                        lx_play + ipx + ordeal_icon_gap,
                        (play_col_w - ipx - ordeal_icon_gap).max(80.0),
                    )
                } else {
                    (lx_play, play_col_w)
                };
                let mut play_stack_h =
                    wrapped_text_height(&chamber_display, text_w_play, px_blind, h_blind)
                        + 6.0
                        + wrapped_text_height(
                            &format!(
                                "Wing {}/{} · Target {}",
                                pick.wing,
                                crate::game::run::FINAL_WING,
                                format_score(target_value as u64),
                            ),
                            text_w_play,
                            px_detail,
                            h_detail,
                        )
                        + 4.0
                        + wrapped_text_height(
                            &format!("Reward ¥{}{}", upcoming.clear_reward(), season_suffix),
                            text_w_play,
                            px_detail,
                            h_detail,
                        );
                if upcoming == ChamberKind::Ordeal
                    && let Some(desc) = pick.ordeal_description.as_deref()
                {
                    play_stack_h +=
                        4.0 + wrapped_text_height(desc, text_w_play, px_detail, h_detail * 1.35);
                }
                let play_cy = pr[1] + pr[3] * 0.5;
                // Side-wall PLAY AABB sits low; lift the label onto SKIP's row for visual centering.
                let hud_row_cy = if can_skip {
                    sr[1] + sr[3] * 0.5
                } else {
                    play_cy
                };
                let pr_play_label = {
                    let y_raw = pr[1] + (hud_row_cy - play_cy);
                    let min_y = 6.0f32;
                    let max_y = (h - pr[3] - 6.0).max(min_y);
                    [pr[0], y_raw.clamp(min_y, max_y), pr[2], pr[3]]
                };
                let mut ly_play = (hud_row_cy - play_stack_h * 0.5).max(10.0);
                let play_stack_label = if upcoming_ordeal { "BOSS" } else { "PLAY" };
                texts.push(hallway_button_text_label(
                    pr_play_label,
                    play_stack_label,
                    if play_focused_label {
                        color::CHAMPAGNE
                    } else {
                        color::PARCHMENT
                    },
                ));
                let play_color = if play_focused_label {
                    color::GOLD
                } else {
                    color::STONE
                };
                if let (Some(ipx), Some(bk)) = (ordeal_icon_px, pick.ordeal_kind) {
                    // Right-aligned title leaves empty space on the left; pin the
                    // icon to the measured left edge of the first line (like skip + tag).
                    let title_lines = wrap_text(&chamber_display, text_w_play, px_blind / 0.99);
                    let first_line = title_lines
                        .first()
                        .map(|s| s.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(chamber_display.as_str());
                    let title_w: f32 = if let Some(font) = load_ui_font() {
                        let (_, _, advances) = measure_label_advances(
                            font,
                            first_line,
                            8192,
                            h_blind.max(1.0) as u32,
                            Some(px_blind),
                        );
                        advances.iter().copied().sum::<f32>().min(text_w_play)
                    } else {
                        let n = first_line.chars().count().max(1) as f32;
                        (px_blind * 0.52 * n).min(text_w_play)
                    };
                    let mut icon_x = text_x_play + text_w_play - title_w - ordeal_icon_gap - ipx;
                    icon_x = icon_x.max(edge_margin);
                    let title_line_count = title_lines.len().max(1) as f32;
                    let name_block_h = h_blind * title_line_count;
                    let icon_y = ly_play + (name_block_h - ipx) * 0.5;
                    icon_cmds.push(ImageQuad {
                        inst: GpuInstance {
                            rect: [icon_x, icon_y, ipx, ipx],
                            color: color::alpha(play_color, 0.98),
                            user: 0,
                        },
                        source: ordeal_icon_source(bk),
                    });
                }
                push_wrapped_column_line(WrappedColumnLine {
                    texts: &mut texts,
                    x: text_x_play,
                    y: &mut ly_play,
                    col_w: text_w_play,
                    font_px: px_blind,
                    line_h: h_blind,
                    text: &chamber_display,
                    color: play_color,
                    align: TextAlign::Right,
                });
                ly_play += 6.0;
                push_wrapped_column_line(WrappedColumnLine {
                    texts: &mut texts,
                    x: text_x_play,
                    y: &mut ly_play,
                    col_w: text_w_play,
                    font_px: px_detail,
                    line_h: h_detail,
                    text: &format!(
                        "Wing {}/{} · Target {}",
                        pick.wing,
                        crate::game::run::FINAL_WING,
                        format_score(target_value as u64),
                    ),
                    color: play_color,
                    align: TextAlign::Right,
                });
                ly_play += 4.0;
                push_wrapped_column_line(WrappedColumnLine {
                    texts: &mut texts,
                    x: text_x_play,
                    y: &mut ly_play,
                    col_w: text_w_play,
                    font_px: px_detail,
                    line_h: h_detail,
                    text: &format!("Reward ¥{}{}", upcoming.clear_reward(), season_suffix),
                    color: play_color,
                    align: TextAlign::Right,
                });
                if upcoming == ChamberKind::Ordeal {
                    ly_play += 4.0;
                    if let Some(desc) = pick.ordeal_description.as_deref() {
                        push_wrapped_column_line(WrappedColumnLine {
                            texts: &mut texts,
                            x: text_x_play,
                            y: &mut ly_play,
                            col_w: text_w_play,
                            font_px: px_detail,
                            line_h: h_detail * 1.35,
                            text: desc,
                            color: color::AMBER,
                            align: TextAlign::Right,
                        });
                    }
                }
                if can_skip {
                    let lx_skip = sr[0] + sr[2] + col_gap;
                    let skip_col_w = (w - edge_margin - lx_skip).max(min_col_w);
                    let skip_icon_px = (h * 0.072).clamp(48.0, 80.0);
                    let skip_icon_gap = 10.0;
                    let skip_stack_h = if let Some(tag) = skip_tag {
                        let text_col_w = (skip_col_w - skip_icon_px - skip_icon_gap).max(80.0);
                        let text_block_h =
                            wrapped_text_height(tag.name(), text_col_w, px_blind, h_blind)
                                + 6.0
                                + wrapped_text_height(
                                    tag.description(),
                                    text_col_w,
                                    px_detail,
                                    h_detail,
                                );
                        text_block_h.max(skip_icon_px)
                    } else {
                        wrapped_text_height("Tribute · Esc", skip_col_w, px_detail, h_detail)
                    };
                    let skip_cy = sr[1] + sr[3] * 0.5;
                    let mut ly_skip = (skip_cy - skip_stack_h * 0.5).max(10.0);
                    texts.push(hallway_button_text_label(
                        sr,
                        "SKIP",
                        if skip_focused_label {
                            color::CHAMPAGNE
                        } else {
                            color::PARCHMENT
                        },
                    ));
                    let skip_color = if skip_focused_label {
                        color::GOLD
                    } else {
                        color::STONE
                    };
                    if let Some(tag) = skip_tag {
                        let text_col_w = (skip_col_w - skip_icon_px - skip_icon_gap).max(80.0);
                        let name_h = wrapped_text_height(tag.name(), text_col_w, px_blind, h_blind);
                        let desc_h =
                            wrapped_text_height(tag.description(), text_col_w, px_detail, h_detail);
                        let text_block_h = name_h + 6.0 + desc_h;
                        let block_top = ly_skip;
                        let icon_y = block_top + (skip_stack_h - skip_icon_px) * 0.5;
                        let text_x = lx_skip + skip_icon_px + skip_icon_gap;
                        let mut ly_text = block_top + (skip_stack_h - text_block_h) * 0.5;
                        icon_cmds.push(ImageQuad {
                            inst: GpuInstance {
                                rect: [lx_skip, icon_y, skip_icon_px, skip_icon_px],
                                color: color::alpha(skip_color, 0.98),
                                user: 0,
                            },
                            source: skip_tag_icon_source(tag),
                        });
                        push_wrapped_column_line(WrappedColumnLine {
                            texts: &mut texts,
                            x: text_x,
                            y: &mut ly_text,
                            col_w: text_col_w,
                            font_px: px_blind,
                            line_h: h_blind,
                            text: tag.name(),
                            color: skip_color,
                            align: TextAlign::Left,
                        });
                        ly_text += 6.0;
                        push_wrapped_column_line(WrappedColumnLine {
                            texts: &mut texts,
                            x: text_x,
                            y: &mut ly_text,
                            col_w: text_col_w,
                            font_px: px_detail,
                            line_h: h_detail,
                            text: tag.description(),
                            color: skip_color,
                            align: TextAlign::Left,
                        });
                    } else {
                        push_wrapped_column_line(WrappedColumnLine {
                            texts: &mut texts,
                            x: lx_skip,
                            y: &mut ly_skip,
                            col_w: skip_col_w,
                            font_px: px_detail,
                            line_h: h_detail,
                            text: "Tribute",
                            color: skip_color,
                            align: TextAlign::Left,
                        });
                    }
                }
                (pr, sr)
            };

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

            // Register focus-tree click targets for PlayChamber + SkipChamber.
            let items = Self::flat_items_hallway(
                w,
                h,
                &cam,
                ctx.room_gltf_height_scale,
                can_skip,
                play_node,
            );
            self.tree.register_flat_buttons(&items, &mut buttons);
        }

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
            crate::scenes::options::options_scroll_fade_backdrop(true),
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
        if !icon_cmds.is_empty() {
            frame.image_quads(icon_cmds);
        }

        frame.buttons = buttons;
        if !hide_scene_hud {
            push_screen_footer_hint(
                &mut frame,
                &ctx,
                menu_footer_row(ctx.input_mode),
                HintStyle::standard(h),
            );
        }
        frame.window_title = "Mahjuro".to_string();

        frame
    }
}
