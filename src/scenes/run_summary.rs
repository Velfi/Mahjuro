//! Game over scene — shown when the player exhausts plays without reaching the target.

use std::time::Instant;

use crate::core::memorial_talisman::{MemorialTalismanKind, select_memorial};
use crate::core::progression::{
    LEVEL_UP_POINTS_FOR_LOSS, LEVEL_UP_POINTS_FOR_WIN, MAX_PROGRESS_LEVEL, POINTS_PER_LEVEL,
    PlayerProgress, meta_depth_roman,
};
use crate::game::engine::GameEngine;
use crate::game::event_bus::{GameEvent, GameOverReason};
use crate::game::memorial_run::snapshot_from_run;
use crate::game::run::RunState;
use crate::persistence;
use crate::render::draw_cmd::UiFrame;
use crate::render::main_menu_glb;
use crate::render::theme::color;
use crate::render::wgpu_renderer::GpuInstance;
use crate::sfx_id::SfxId;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::archive_career::format_score;
use super::run_summary_panel::{
    RunSummaryPanelContent, RunSummaryPanelLayout, RunSummaryPanelLevel, RunSummaryPanelScroll,
    RunSummaryPanelTheme, RunSummaryStats, push_run_summary_panel,
};
use super::{DrawCtx, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DismissAction;

fn chambers_cleared_before_death(blind: crate::core::rules::ChamberKind) -> u32 {
    use crate::core::rules::ChamberKind;
    match blind {
        ChamberKind::Small => 0,
        ChamberKind::Big => 1,
        ChamberKind::Ordeal => 2,
    }
}

/// Chambers won vs chambers faced this run (`wins`, `chambers`).
fn run_chamber_wins_and_total(run: &RunState) -> (u32, u32) {
    if !run.chronicle.encounters.is_empty() {
        let wins = run
            .chronicle
            .encounters
            .iter()
            .filter(|e| e.outcome == "Cleared")
            .count() as u32;
        let chambers = run.chronicle.encounters.len() as u32;
        return (wins, chambers.max(1));
    }
    use crate::game::run::FINAL_WING;
    if run.is_run_complete() {
        let n = FINAL_WING.saturating_mul(3);
        (n, n)
    } else {
        let completed_antes = run.wing.saturating_sub(1);
        let wins = completed_antes
            .saturating_mul(3)
            .saturating_add(chambers_cleared_before_death(run.chamber));
        (wins, wins.saturating_add(1))
    }
}

impl RunSummaryStats {
    fn from_run(run: &RunState) -> Self {
        let best_structure = if run.best_structure_score > 0 {
            format!(
                "{} ({})",
                run.best_structure_name,
                format_score(run.best_structure_score)
            )
        } else {
            "None".to_string()
        };
        let most_played_structure = run
            .yaku_times_played
            .iter()
            .max_by(|(ya, ca), (yb, cb)| ca.cmp(cb).then_with(|| yb.name().cmp(ya.name())))
            .map(|(yaku, count)| format!("{} ({}x)", yaku.name(), count))
            .unwrap_or_else(|| "None".to_string());
        let (wins, chambers) = run_chamber_wins_and_total(run);
        let pct = if chambers > 0 {
            (wins as f32 / chambers as f32 * 100.0).round() as u32
        } else {
            0
        };

        Self {
            best_structure,
            most_played_structure,
            total_score: format_score(run.total_score_earned),
            completion: format!("{pct}%"),
        }
    }
}

pub struct RunSummaryScene {
    pub final_score: u64,
    pub target_score: u32,
    pub won: bool,
    pub loss_reason: Option<GameOverReason>,
    /// Remnant the player is becoming (defeat only).
    pub memorial_kind: Option<MemorialTalismanKind>,
    memorial_subtitle: Option<String>,
    summary: RunSummaryStats,
    tree: TreeState,
    panel_scroll: RunSummaryPanelScroll,
    opened_at: Instant,
    outcome_sfx_fired: bool,
}

/// Delay between the game-over screen appearing and its outcome stinger.
const OUTCOME_SFX_DELAY_SECS: f32 = 1.0;

impl RunSummaryScene {
    pub fn defeat(run: &RunState, reason: GameOverReason) -> Self {
        let gameplay = GameEngine::read(run);
        let snap = snapshot_from_run(&run.defeat_journal, reason, run);
        let memorial = run
            .defeat_memorial_kind
            .or_else(|| Some(select_memorial(&snap)));
        let memorial_subtitle =
            memorial.map(|k| format!("{} — {}", k.name(), k.defeat_subtitle(&snap)));
        Self {
            final_score: gameplay.round_score,
            target_score: gameplay.target_score,
            won: false,
            loss_reason: Some(reason),
            memorial_kind: memorial,
            memorial_subtitle,
            summary: RunSummaryStats::from_run(run),
            tree: TreeState::new(),
            panel_scroll: RunSummaryPanelScroll::new(),
            opened_at: Instant::now(),
            outcome_sfx_fired: false,
        }
    }

    /// Construct a victory screen shown after defeating the final-ante Boss.
    pub fn victory(run: &RunState) -> Self {
        let gameplay = GameEngine::read(run);
        Self {
            final_score: gameplay.round_score,
            target_score: gameplay.target_score,
            won: true,
            loss_reason: None,
            memorial_kind: None,
            memorial_subtitle: None,
            summary: RunSummaryStats::from_run(run),
            tree: TreeState::new(),
            panel_scroll: RunSummaryPanelScroll::new(),
            opened_at: Instant::now(),
            outcome_sfx_fired: false,
        }
    }

    fn flat_items(&self, w: f32, h: f32) -> [FlatItem<DismissAction>; 1] {
        [FlatItem::new(FocusId(0), [0.0, 0.0, w, h], DismissAction)]
    }

    fn panel_content(&self, progress: &PlayerProgress) -> RunSummaryPanelContent {
        let points_short = (self.target_score as u64).saturating_sub(self.final_score);
        let defeat_loss_line = if self.won {
            None
        } else {
            let cause = self
                .loss_reason
                .map(GameOverReason::loss_summary)
                .unwrap_or("unknown cause");
            let mut line = format!("Cause: {cause}");
            if let Some(ref memorial) = self.memorial_subtitle {
                line.push('\n');
                line.push_str(memorial);
            }
            Some(line)
        };
        let pre_depth_text = if self.won {
            Some("The House's hold is broken — for now.".to_string())
        } else {
            defeat_loss_line
        };
        let subtitle = if self.won {
            String::new()
        } else if points_short == 1 {
            "1 point short".to_string()
        } else {
            format!("{} points short", format_score(points_short))
        };
        let points_earned = if self.won {
            LEVEL_UP_POINTS_FOR_WIN
        } else {
            LEVEL_UP_POINTS_FOR_LOSS
        };
        let total_points = progress.level_progress_points;
        let current_level = progress.current_level();
        let prev_points = total_points.saturating_sub(points_earned);
        let prev_level = 1u32
            .saturating_add(prev_points / POINTS_PER_LEVEL)
            .min(MAX_PROGRESS_LEVEL);
        let min_for_level = PlayerProgress::min_points_for_level(current_level);
        let into_level = total_points
            .saturating_sub(min_for_level)
            .min(POINTS_PER_LEVEL);
        let progress_label = if current_level >= MAX_PROGRESS_LEVEL {
            "You have reached the limits of the house".to_string()
        } else {
            "Onward and downward".to_string()
        };
        let progress_value = "".to_string();
        let level_transition = if current_level > prev_level {
            Some(format!(
                "{} → {}",
                meta_depth_roman(prev_level),
                meta_depth_roman(current_level)
            ))
        } else {
            None
        };

        let stats_rows = vec![
            ("Total score".to_string(), self.summary.total_score.clone()),
            (
                "Best structure".to_string(),
                self.summary.best_structure.clone(),
            ),
            (
                "Most played".to_string(),
                self.summary.most_played_structure.clone(),
            ),
            ("Completion".to_string(), self.summary.completion.clone()),
        ];

        RunSummaryPanelContent {
            headline: if self.won {
                "The Moon's light welcomes you into the cool night.".to_string()
            } else {
                "Loser!".to_string()
            },
            subtitle,
            pre_depth_text,
            hint: "Is your fate settled then?".to_string(),
            stats_rows,
            level: RunSummaryPanelLevel {
                current_level,
                prev_level,
                points_earned,
                into_level,
                progress_label,
                progress_value,
                level_transition,
            },
        }
    }
}

impl SceneBehavior for RunSummaryScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if ctx.headless {
            self.opened_at = Instant::now() - std::time::Duration::from_secs(2);
        }
        if !self.outcome_sfx_fired
            && self.opened_at.elapsed().as_secs_f32() >= OUTCOME_SFX_DELAY_SECS
        {
            let sfx = if self.won {
                if self.final_score.is_multiple_of(2) {
                    SfxId::Victory
                } else {
                    SfxId::Victory2
                }
            } else {
                SfxId::Defeat
            };
            ctx.bus.push(GameEvent::UiSound(sfx));
            self.outcome_sfx_fired = true;
        }
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let content = self.panel_content(ctx.progress);
        let theme = if self.won {
            RunSummaryPanelTheme::victory()
        } else {
            RunSummaryPanelTheme::defeat()
        };
        let layout = RunSummaryPanelLayout::compute(w, h, &content, &theme);
        self.panel_scroll.sync(&layout);
        self.panel_scroll
            .handle_wheel(ctx.scroll_lines, ctx.cursor_pos, &layout, ctx.input_mode);
        let block_dismiss =
            self.panel_scroll
                .handle_mouse(ctx.cursor_pos, ctx.mouse_left_down, &layout);

        let items = self.flat_items(w, h);
        let button_clicks = if block_dismiss {
            &[][..]
        } else {
            ctx.button_clicks
        };
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }
        if action.is_some() {
            ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
            let settings = persistence::load_settings();
            GameEngine::reset_to_demo(ctx.run, ctx.progress, &settings);
            return Some(SceneIntent::MainMenu);
        }
        None
    }

    fn draw_frame(&self, mut ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let content = self.panel_content(ctx.progress);
        let theme = if self.won {
            RunSummaryPanelTheme::victory()
        } else {
            RunSummaryPanelTheme::defeat()
        };
        let layout = RunSummaryPanelLayout::compute(w, h, &content, &theme);
        self.panel_scroll.sync(&layout);
        let scroll_offset = self.panel_scroll.offset_px();

        let items = self.flat_items(w, h);
        let mut buttons = Vec::new();
        self.tree.register_flat_buttons(&items, &mut buttons);

        let mut frame = UiFrame::new();
        let backdrop = if !self.won && self.memorial_kind.is_some() {
            [0.0, 0.0, 0.0, 1.0]
        } else {
            color::WALNUT_INK
        };
        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: backdrop,
            user: 0,
        });
        if self.won {
            if ctx.effect_layers.fullscreen_water_backdrop {
                frame.moonlit_water();
            }
            if crate::render::room_gpu_resident::victory_uses_3d_moon(ctx.graphics_mode)
                && ctx.victory_moon_gpu_ready
                && main_menu_glb::main_menu_room_draw_ready()
            {
                let env_scale =
                    main_menu_glb::main_menu_env_height_scale(ctx.room_gltf_height_scale);
                if let Some((cam, model_delta)) = main_menu_glb::victory_summary_moon_setup(
                    w,
                    h,
                    env_scale,
                    ctx.victory_moon_debug.rotation_xyz,
                ) {
                    frame.moonlit_water_hide_disc = ctx.effect_layers.fullscreen_water_backdrop;
                    frame.main_menu_environment();
                    frame.main_menu_env_moon_only = true;
                    frame.main_menu_env_model_delta = model_delta;
                    frame.camera_override = Some(cam);
                    let room_glb = main_menu_glb::main_menu_glb_has_embedded_lights();
                    frame.scene_lighting.embedded_gltf_punctual = room_glb;
                    frame.scene_lighting.room_glb_brdf = room_glb;
                }
            }
        } else if let Some(kind) = self.memorial_kind {
            super::defeat_tableau::push_defeat_memorial_tableau(&mut frame, ctx.layout, kind);
        } else if ctx.effect_layers.fullscreen_water_backdrop {
            frame.sunlit_water();
        }

        push_run_summary_panel(
            &mut frame,
            &ctx,
            &layout,
            &content,
            &theme,
            self.opened_at,
            scroll_offset,
        );

        frame.buttons = buttons;
        frame.window_title = if self.won {
            "Victory! — Final wing cleared".to_string()
        } else {
            format!(
                "Game Over — {} / {}",
                format_score(self.final_score),
                format_score(self.target_score as u64)
            )
        };
        ctx.stash_focus_nav_tree_flat(&self.tree, &items, |_| "Continue".into());
        frame
    }
}

/// Victory run-end screen (`Scene::Victory`).
pub struct VictoryScene(RunSummaryScene);

impl VictoryScene {
    pub fn new(run: &RunState) -> Self {
        Self(RunSummaryScene::victory(run))
    }
}

impl SceneBehavior for VictoryScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.0.update(ctx)
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        self.0.draw_frame(ctx)
    }
}

/// Defeat run-end screen (`Scene::Defeat`).
pub struct DefeatScene(RunSummaryScene);

impl DefeatScene {
    pub fn new(run: &RunState, reason: GameOverReason) -> Self {
        Self(RunSummaryScene::defeat(run, reason))
    }
}

impl SceneBehavior for DefeatScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.0.update(ctx)
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        self.0.draw_frame(ctx)
    }
}
