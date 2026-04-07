//! Mahjuro — UI-first shell: winit + wgpu + cassowary + input + scene system.

pub mod asset_path;
mod audio;
mod bot;
mod core;
pub mod crash_guard;
mod debug_menu;
mod game;
mod persistence;
mod render;
mod scenes;
mod ui;

use std::sync::Arc;
use std::time::Instant;

use debug_menu::{DebugAction, DebugMenuBar};
use game::cascade::CascadeTuning;
use game::event_bus::{EventBus, GameEvent};
use game::run::{RunState, pick_relic_choices};
use render::animation::AnimationController;
use render::draw_cmd::UiFrame;
use render::wgpu_renderer::{GpuInstance, TextLabel, WgpuRenderer};
use scenes::game_over::GameOverScene;
use scenes::results::ResultsScene;
use scenes::splash::SplashScene;
use scenes::{ButtonAction, ButtonDef, DrawCtx, Scene, UpdateCtx};
use ui::input::{InputMode, InputState, UiAction};
use ui::layout::UiLayout;
use ui::modal::{Modal, ModalQueue, ModalTheme};
use ui::tooltip::TooltipState;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

// ── Tuning overlay (debug) ──────────────────────────────────────────

const TUNING_ROW_COUNT: usize = 8; // 7 sliders + Export button
const TUNING_MIN_MS: u64 = 50;
const TUNING_MAX_MS: u64 = 3000;
const TUNING_STEP_MS: u64 = 50;

struct TuningOverlay {
    cursor: usize,
    tuning: CascadeTuning,
}

impl TuningOverlay {
    fn new(tuning: &CascadeTuning) -> Self {
        Self {
            cursor: 0,
            tuning: tuning.clone(),
        }
    }

    /// Returns `true` if the overlay should close, along with an optional export path.
    fn update(&mut self, actions: &[UiAction]) -> TuningResult {
        for a in actions {
            match a {
                UiAction::FocusDown => {
                    self.cursor = (self.cursor + 1) % TUNING_ROW_COUNT;
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + TUNING_ROW_COUNT - 1) % TUNING_ROW_COUNT;
                }
                UiAction::FocusNext | UiAction::NavigateHudNext => {
                    self.adjust(TUNING_STEP_MS as i64);
                }
                UiAction::FocusPrev | UiAction::NavigateHudPrev => {
                    self.adjust(-(TUNING_STEP_MS as i64));
                }
                UiAction::Confirm => {
                    if self.cursor == TUNING_ROW_COUNT - 1 {
                        return TuningResult::Export;
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    return TuningResult::Close;
                }
                _ => {}
            }
        }
        TuningResult::Stay
    }

    fn adjust(&mut self, delta: i64) {
        let field = match self.cursor {
            0 => &mut self.tuning.base_hold_ms,
            1 => &mut self.tuning.step_hold_ms,
            2 => &mut self.tuning.total_hold_ms,
            3 => &mut self.tuning.tick_duration_ms,
            4 => &mut self.tuning.depart_lifetime_ms,
            5 => &mut self.tuning.draw_settle_ms,
            6 => &mut self.tuning.sort_settle_ms,
            _ => return,
        };
        *field = (*field as i64 + delta).clamp(TUNING_MIN_MS as i64, TUNING_MAX_MS as i64) as u64;
    }

    fn draw(&self, window_w: f32, window_h: f32) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let scale = (window_w.min(window_h)) / 600.0;
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        // Dim overlay background.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: [0.0, 0.0, 0.0, 0.7],
        });

        // Panel dimensions.
        let panel_w = (520.0 * scale).min(window_w * 0.90);
        let row_h = (40.0 * scale).max(26.0);
        let desc_h = (18.0 * scale).max(12.0);
        let row_gap = (10.0 * scale).max(4.0);
        let title_h = (48.0 * scale).max(28.0);
        let diagram_h = (80.0 * scale).max(50.0);
        let row_total_h = row_h + desc_h + row_gap;
        let panel_h = title_h + row_gap
            + diagram_h + row_gap
            + 7.0 * row_total_h  // 7 slider rows
            + (row_h + row_gap)  // export button
            + row_gap * 3.0;
        let panel_x = (window_w - panel_w) * 0.5;
        let panel_y = (window_h - panel_h) * 0.5;

        // Panel background.
        instances.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: [0.08, 0.08, 0.14, 0.95],
        });
        // Panel border.
        let border = 3.0;
        instances.push(GpuInstance {
            rect: [
                panel_x - border,
                panel_y - border,
                panel_w + border * 2.0,
                panel_h + border * 2.0,
            ],
            color: [0.3, 0.45, 0.7, 0.8],
        });
        // Re-draw panel on top of border.
        instances.push(GpuInstance {
            rect: [panel_x, panel_y, panel_w, panel_h],
            color: [0.08, 0.08, 0.14, 0.95],
        });

        // Title.
        labels.push(TextLabel {
            rect: [panel_x, panel_y + row_gap, panel_w, title_h],
            text: "Cascade Tuning".into(),
            color: [1.0, 0.95, 0.7, 1.0],
        });

        let mut cursor_y = panel_y + row_gap + title_h + row_gap;

        // Timing diagram.
        let diag_pad = 12.0 * scale;
        instances.push(GpuInstance {
            rect: [
                panel_x + diag_pad,
                cursor_y,
                panel_w - diag_pad * 2.0,
                diagram_h,
            ],
            color: [0.06, 0.06, 0.10, 0.9],
        });
        // Draw timeline segments proportional to actual values.
        let total_ms =
            self.tuning.base_hold_ms + self.tuning.step_hold_ms * 2 + self.tuning.total_hold_ms;
        let bar_x = panel_x + diag_pad + 8.0 * scale;
        let bar_w = panel_w - diag_pad * 2.0 - 16.0 * scale;
        let bar_h = (16.0 * scale).max(10.0);
        let bar_y = cursor_y + diagram_h * 0.35;
        let colors: [[f32; 4]; 4] = [
            [0.35, 0.65, 0.90, 0.9], // base hold (blue)
            [0.55, 0.80, 0.45, 0.9], // step 1 (green)
            [0.45, 0.70, 0.35, 0.9], // step 2 (green darker)
            [0.90, 0.75, 0.30, 0.9], // total hold (gold)
        ];
        let segments: [u64; 4] = [
            self.tuning.base_hold_ms,
            self.tuning.step_hold_ms,
            self.tuning.step_hold_ms,
            self.tuning.total_hold_ms,
        ];
        let seg_labels = ["Base", "Step", "Step", "Total"];
        let mut seg_x = bar_x;
        for (i, &ms) in segments.iter().enumerate() {
            let seg_w = bar_w * (ms as f32 / total_ms as f32);
            instances.push(GpuInstance {
                rect: [seg_x, bar_y, seg_w, bar_h],
                color: colors[i],
            });
            // Segment label (centered in segment).
            if seg_w > 20.0 {
                labels.push(TextLabel {
                    rect: [seg_x, bar_y, seg_w, bar_h],
                    text: seg_labels[i].to_string(),
                    color: [0.0, 0.0, 0.0, 0.9],
                });
            }
            seg_x += seg_w;
        }
        // Diagram title.
        labels.push(TextLabel {
            rect: [
                panel_x + diag_pad,
                cursor_y + 2.0,
                panel_w - diag_pad * 2.0,
                diagram_h * 0.28,
            ],
            text: "Timeline: Base > Steps (x N) > Total".into(),
            color: [0.6, 0.6, 0.7, 0.8],
        });
        // Tick duration annotation.
        let tick_label_y = bar_y + bar_h + 4.0 * scale;
        labels.push(TextLabel {
            rect: [
                panel_x + diag_pad,
                tick_label_y,
                panel_w - diag_pad * 2.0,
                diagram_h * 0.25,
            ],
            text: format!(
                "Score counter ticks over {}ms per phase",
                self.tuning.tick_duration_ms
            ),
            color: [0.5, 0.5, 0.6, 0.7],
        });

        cursor_y += diagram_h + row_gap;

        // Slider rows with descriptions.
        let label_w = panel_w * 0.38;
        let slider_w = panel_w * 0.35;
        let value_w = panel_w * 0.18;

        let rows: [(&str, &str, u64); 7] = [
            (
                "Base Hold",
                "Pause on base points before steps begin",
                self.tuning.base_hold_ms,
            ),
            (
                "Step Hold",
                "Pause per relic/rule multiplier step",
                self.tuning.step_hold_ms,
            ),
            (
                "Total Hold",
                "Pause on final total before resuming play",
                self.tuning.total_hold_ms,
            ),
            (
                "Tick Duration",
                "Speed of the score counter tick-up animation",
                self.tuning.tick_duration_ms,
            ),
            (
                "Discard Speed",
                "How long discarded tiles float away",
                self.tuning.depart_lifetime_ms,
            ),
            (
                "Draw Speed",
                "How long drawn tiles take to settle in",
                self.tuning.draw_settle_ms,
            ),
            (
                "Sort/Drag Speed",
                "How long sort and drag-reorder animations take",
                self.tuning.sort_settle_ms,
            ),
        ];

        for (i, (name, desc, value)) in rows.iter().enumerate() {
            let row_y = cursor_y + i as f32 * row_total_h;
            let is_focused = self.cursor == i;

            // Row background.
            let bg = if is_focused {
                [0.20, 0.32, 0.50, 0.90]
            } else {
                [0.12, 0.15, 0.24, 0.75]
            };
            instances.push(GpuInstance {
                rect: [panel_x + 4.0, row_y, panel_w - 8.0, row_h + desc_h],
                color: bg,
            });

            // Label.
            let tc = if is_focused {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.6, 0.6, 0.7, 0.9]
            };
            labels.push(TextLabel {
                rect: [panel_x + 12.0 * scale, row_y, label_w, row_h],
                text: name.to_string(),
                color: tc,
            });

            // Description below label.
            labels.push(TextLabel {
                rect: [
                    panel_x + 12.0 * scale,
                    row_y + row_h * 0.75,
                    label_w + slider_w,
                    desc_h,
                ],
                text: desc.to_string(),
                color: [0.45, 0.45, 0.55, 0.7],
            });

            // Slider track.
            let track_x = panel_x + label_w;
            let track_h = (8.0 * scale).max(4.0);
            let track_y = row_y + (row_h - track_h) * 0.5;
            instances.push(GpuInstance {
                rect: [track_x, track_y, slider_w, track_h],
                color: [0.08, 0.08, 0.14, 1.0],
            });

            // Slider fill.
            let t = (*value as f32 - TUNING_MIN_MS as f32) / (TUNING_MAX_MS - TUNING_MIN_MS) as f32;
            let fill_w = slider_w * t.clamp(0.0, 1.0);
            let fill_color = if is_focused {
                [0.35, 0.65, 0.90, 1.0]
            } else {
                [0.22, 0.42, 0.62, 0.85]
            };
            instances.push(GpuInstance {
                rect: [track_x, track_y, fill_w, track_h],
                color: fill_color,
            });

            // Knob.
            let knob_size = track_h * 2.5;
            let knob_x = track_x + fill_w - knob_size * 0.5;
            let knob_y = track_y + (track_h - knob_size) * 0.5;
            instances.push(GpuInstance {
                rect: [knob_x, knob_y, knob_size, knob_size],
                color: if is_focused {
                    [0.9, 0.9, 1.0, 1.0]
                } else {
                    [0.6, 0.6, 0.7, 0.9]
                },
            });

            // Value text.
            let value_x = panel_x + label_w + slider_w + 4.0;
            labels.push(TextLabel {
                rect: [value_x, row_y, value_w, row_h],
                text: format!("{}ms", value),
                color: tc,
            });
        }

        // Export button row.
        let export_y = cursor_y + 7.0 * row_total_h;
        let is_focused = self.cursor == TUNING_ROW_COUNT - 1;
        let bg = if is_focused {
            [0.25, 0.45, 0.30, 0.95]
        } else {
            [0.15, 0.20, 0.18, 0.85]
        };
        instances.push(GpuInstance {
            rect: [panel_x + 4.0, export_y, panel_w - 8.0, row_h],
            color: bg,
        });
        labels.push(TextLabel {
            rect: [panel_x, export_y, panel_w, row_h],
            text: "Export as JSON".into(),
            color: if is_focused {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.6, 0.6, 0.7, 0.9]
            },
        });

        // Hint.
        let hint_y = export_y + row_h + row_gap;
        labels.push(TextLabel {
            rect: [panel_x, hint_y, panel_w, row_h * 0.6],
            text: "Left/Right: adjust   Esc: close".into(),
            color: [0.4, 0.4, 0.5, 0.6],
        });

        (instances, labels)
    }
}

enum TuningResult {
    Stay,
    Close,
    Export,
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<WgpuRenderer>,
    layout_engine: UiLayout,
    input: Option<InputState>,
    run: RunState,
    bus: EventBus,
    anim: AnimationController,
    last_frame: Instant,
    mouse_actions: Vec<UiAction>,
    /// Scene-defined button click ids fired by mouse clicks since the last
    /// frame; drained into `UpdateCtx::button_clicks` each frame.
    mouse_button_clicks: Vec<u32>,
    /// Button rects from the last draw, for click hit-testing.
    active_buttons: Vec<ButtonDef>,
    scene: Scene,
    progress: crate::core::progression::PlayerProgress,
    active_profile: usize,
    audio: audio::AudioManager,
    /// Scene transition fade: 1.0 = fully visible, 0.0 = fully faded out.
    transition_alpha: f32,
    /// Scene waiting to be swapped in after fade-out completes.
    pending_scene: Option<Scene>,
    /// Set by scenes to request application exit.
    quit_requested: bool,
    /// Whether the window close button has already been pressed once (save performed).
    close_saved: bool,
    /// Modal toast queue — overlays any scene.
    modals: ModalQueue,
    /// Modals deferred until the player dismisses the GameOver scene
    /// (e.g. level-up celebrations shown after defeat/victory).
    pending_post_game_over_modals: Vec<Modal>,
    /// Native OS debug menu bar.
    debug_menu: Option<DebugMenuBar>,
    /// Smoke effect intensity (persisted in settings).
    smoke_intensity: crate::persistence::SmokeIntensity,
    /// Previous cursor position for computing cursor velocity.
    prev_cursor: (f32, f32),
    /// Whether to show the FPS counter (debug toggle).
    show_fps: bool,
    /// Smoothed FPS value for display.
    fps_smoothed: f32,
    /// Paradox-style nested tooltip system.
    tooltips: TooltipState,
    /// Cascade animation timing (tunable from debug menu).
    cascade_tuning: CascadeTuning,
    /// Tuning overlay (None = closed).
    tuning_overlay: Option<TuningOverlay>,
    /// Round-end events held until the active scoring cascade finishes.
    /// Lets the player watch the winning cascade play out before the
    /// Results / GameOver scene fades in.
    deferred_round_end: Option<GameEvent>,
}

impl App {
    fn new() -> Self {
        let t0 = Instant::now();
        let settings = persistence::load_settings();
        let active_profile = settings.active_profile;
        let progress = persistence::load_profile(active_profile);
        let mut run = RunState::new_demo();
        run.available_yaku = progress.available_yaku();
        run.available_rules = progress.available_rules();
        let mut audio = audio::AudioManager::new();
        audio.set_master_volume(settings.master_volume);
        audio.set_sfx_volume(settings.sfx_volume);
        audio.set_music_volume(settings.music_volume);
        if !settings.sfx_enabled {
            audio.set_enabled(false);
        }
        log::info!("App::new() settings + profile loaded in {:?}", t0.elapsed());
        Self {
            window: None,
            renderer: None,
            layout_engine: UiLayout::new(),
            input: None,
            run,
            bus: EventBus::default(),
            anim: AnimationController::new(),
            last_frame: Instant::now(),
            mouse_actions: Vec::new(),
            mouse_button_clicks: Vec::new(),
            active_buttons: Vec::new(),
            scene: Scene::Splash(SplashScene::new()),
            progress,
            active_profile,
            audio,
            transition_alpha: 1.0,
            pending_scene: None,
            quit_requested: false,
            close_saved: false,
            modals: ModalQueue::default(),
            pending_post_game_over_modals: Vec::new(),
            deferred_round_end: None,
            debug_menu: None,
            smoke_intensity: settings.smoke_intensity,
            prev_cursor: (0.0, 0.0),
            show_fps: false,
            fps_smoothed: 60.0,
            tooltips: TooltipState::new(),
            cascade_tuning: CascadeTuning::default(),
            tuning_overlay: None,
        }
    }

    /// Switch to a different profile, reloading progress.
    fn switch_profile(&mut self, new_index: usize) {
        // Save current profile first.
        let _ = persistence::save_profile(self.active_profile, &self.progress);
        self.active_profile = new_index;
        self.progress = persistence::load_profile(new_index);
        self.run = RunState::new_demo();
        self.run.available_yaku = self.progress.available_yaku();
        self.run.available_rules = self.progress.available_rules();
        // Persist the active profile choice.
        let mut settings = persistence::load_settings();
        settings.active_profile = new_index;
        let _ = persistence::save_settings(&settings);
    }

    /// Process a `RoundComplete` or `GameOver` event that was held while the
    /// scoring cascade was still playing. Pushes celebration modals, plays the
    /// appropriate sting, and queues the next scene.
    fn handle_round_end_event(&mut self, ev: GameEvent) {
        let win_size = self
            .window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or(PhysicalSize::new(800, 600));
        let ww = win_size.width as f32;
        let wh = win_size.height as f32;
        match ev {
            GameEvent::RoundComplete { .. } => {
                self.audio.play_sfx(audio::SfxId::RoundWin);
                let modal = Modal::new(
                    "Round Complete!",
                    format!(
                        "Score: {} / {}  —  Well played!",
                        self.run.round_score, self.run.target_score
                    ),
                    ModalTheme::Success,
                )
                .with_fireworks(ww * 0.5, wh * 0.8, ww * 0.6, 5);
                self.modals.push(modal);
                let count = self.run.blind.relic_choices();
                let available = self.progress.available_relics();
                let choices = pick_relic_choices(&self.run.relics, count, &available);
                self.pending_scene = Some(Scene::Results(ResultsScene::new(choices)));
                self.transition_alpha = 1.0;
            }
            GameEvent::GameOver { .. } => {
                self.progress.runs_completed += 1;
                self.progress.record_score(self.run.round_score);
                let level_up = self.progress.check_level_up();
                let _ = persistence::save_profile(self.active_profile, &self.progress);

                if let Some(level) = level_up {
                    log::info!("Level up! Now level {}", level);
                    let modal = Modal::new(
                        format!("Level Up! — Level {}", level),
                        "New content unlocked!",
                        ModalTheme::Success,
                    )
                    .with_fireworks(ww * 0.5, wh * 0.7, ww * 0.7, 8);
                    self.pending_post_game_over_modals.push(modal);
                }

                self.audio.play_sfx(audio::SfxId::GameOver);
                self.pending_scene = Some(Scene::GameOver(GameOverScene::new(
                    self.run.round_score,
                    self.run.target_score,
                )));
                self.transition_alpha = 1.0;
            }
            _ => {}
        }
    }

    fn draw(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(win) = self.window.as_ref() else {
            return;
        };

        let size = win.inner_size();
        let layout = self
            .layout_engine
            .solve(size.width as f32, size.height as f32);
        let focus = self.input.as_ref().map(|i| i.focused_index()).unwrap_or(0);

        let ctx = DrawCtx {
            layout: &layout,
            anim: &self.anim,
            run: &self.run,
            focus_tile_index: focus,
            progress: &self.progress,
            active_profile: self.active_profile,
            game_in_progress: self.run.is_in_progress(),
            projected_hand_rects: renderer.projected_hand_rects(),
        };
        let output = self.scene.draw(ctx);

        win.set_title(&output.window_title);
        self.active_buttons = output
            .buttons
            .iter()
            .map(|b| ButtonDef {
                rect: b.rect,
                action: b.action,
            })
            .collect();

        // Spawn departure animations before updating hand tiles (old data still in renderer).
        if !output.departing_indices.is_empty() {
            let depart_lifetime = self.cascade_tuning.depart_lifetime_ms as f32 / 1000.0;
            renderer.depart_tiles(&output.departing_indices, depart_lifetime);
        }
        renderer.update_hand_tiles(&output.hand_tiles);

        // Snapshot the scene's text labels and relic icons for tooltip
        // hover-region scanning. We capture them before any modal/overlay
        // cmds get pushed onto the frame so glossary detection only fires
        // on scene content.
        let scene_text_labels: Vec<TextLabel> = output
            .text_labels
            .iter()
            .map(|l| TextLabel {
                rect: l.rect,
                text: l.text.clone(),
                color: l.color,
            })
            .collect();
        let scene_relic_icons: Vec<crate::render::wgpu_renderer::RelicIcon> = output
            .relic_icons
            .iter()
            .map(|i| crate::render::wgpu_renderer::RelicIcon {
                rect: i.rect,
                relic_id: i.relic_id,
            })
            .collect();

        // Convert SceneDrawOutput → UiFrame in the canonical scene order
        // (background → hand backdrop → smoke → scene quads → hand faces →
        // scene text → relic icons). Modal/tuning/fps/tooltip cmds are
        // appended to the end of `frame.cmds` below — pushed earlier =
        // renders under, pushed later = renders on top.
        let mut frame: UiFrame = output.into_frame();

        // Apply transition alpha to everything that's part of the scene
        // (after into_frame so all scene cmds exist; before overlays are
        // appended so they fade in cleanly).
        let alpha = self.transition_alpha;
        frame.apply_alpha(alpha);

        let size = win.inner_size();
        self.modals.update();
        if let Some((modal_insts, modal_labels, modal_buttons)) =
            self.modals.draw(size.width as f32, size.height as f32)
        {
            frame.quads(modal_insts);
            frame.texts(modal_labels);
            // Replace scene buttons with modal buttons so only dismiss works.
            self.active_buttons = modal_buttons;
        }

        // Tuning overlay — on top of modals.
        if let Some(ref overlay) = self.tuning_overlay {
            let (tuning_insts, tuning_labels) = overlay.draw(size.width as f32, size.height as f32);
            frame.quads(tuning_insts);
            frame.texts(tuning_labels);
            self.active_buttons.clear(); // Block scene buttons.
        }

        // Tooltip overlay — pushed *after* modals/tuning so it sits on top
        // of all scene/modal content. Disabled on overlay screens like Options.
        let skip_tooltips = self.modals.is_active() || matches!(&self.scene, Scene::Options(_));
        if !skip_tooltips {
            let cursor = self
                .input
                .as_ref()
                .map(|i| i.last_cursor)
                .unwrap_or((0.0, 0.0));
            let ww = size.width as f32;
            let wh = size.height as f32;
            let btn_rects: Vec<(f32, f32, f32, f32)> =
                self.active_buttons.iter().map(|b| b.rect).collect();
            self.tooltips.update_and_draw_into(
                &mut frame,
                cursor,
                &scene_text_labels,
                &btn_rects,
                &scene_relic_icons,
                ww,
                wh,
            );
        } else {
            self.tooltips.clear();
        }

        // FPS counter overlay (debug) — pushed last so it's always on top.
        if self.show_fps {
            let dt = self.last_frame.elapsed().as_secs_f32().max(0.001);
            let instant_fps = 1.0 / dt;
            // Exponential moving average for smooth display.
            self.fps_smoothed = self.fps_smoothed * 0.9 + instant_fps * 0.1;
            let w = size.width as f32;
            let h = size.height as f32;
            let label_h = (h * 0.03).max(20.0);
            let label_w = label_h * 4.0;
            let margin = label_h * 0.3;
            frame.quad(GpuInstance {
                rect: [w - label_w - margin, margin, label_w, label_h],
                color: [0.0, 0.0, 0.0, 0.55],
            });
            frame.text(TextLabel {
                rect: [w - label_w - margin, margin, label_w, label_h],
                text: format!("{:.0} FPS", self.fps_smoothed),
                color: [0.9, 0.9, 0.3, 1.0],
            });
        }

        // Convert settle ms to exponential decay speed (inversely proportional).
        // Default: 500ms → speed 8.0, 400ms → speed 10.0.
        let draw_settle_speed = 8.0 * (500.0 / self.cascade_tuning.draw_settle_ms.max(1) as f32);
        let sort_settle_speed = 10.0 * (400.0 / self.cascade_tuning.sort_settle_ms.max(1) as f32);

        if let Err(e) = renderer.render(
            &frame,
            self.smoke_intensity,
            draw_settle_speed,
            sort_settle_speed,
        ) {
            log::error!("render: {e:?}");
        }
    }

    fn handle_debug_action(&mut self, action: DebugAction) {
        match action {
            DebugAction::SetLevel(level) => {
                // Set runs_completed to the minimum value for this level.
                let runs = match level {
                    1 => 0,
                    2 => 1,
                    3 => 3,
                    4 => 6,
                    5 => 10,
                    6 => 15,
                    7 => 20,
                    _ => 0,
                };
                self.progress.runs_completed = runs;
                self.progress.check_level_up();
                self.run.available_yaku = self.progress.available_yaku();
                self.run.available_rules = self.progress.available_rules();
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                log::info!(
                    "[Debug] Set player level to {} (runs_completed={})",
                    level,
                    runs
                );
            }
            DebugAction::SetGold(amount) => {
                self.run.gold = amount;
                log::info!("[Debug] Set gold to {}", amount);
            }
            DebugAction::AddRelic(relic_id) => {
                if !self.run.relics.active.contains(&relic_id) {
                    if self.run.relics.is_full() {
                        // Expand capacity to fit.
                        self.run.relics.max_slots += 1;
                    }
                    self.run.relics.active.push(relic_id);
                    log::info!("[Debug] Added relic {:?}", relic_id);
                } else {
                    log::info!("[Debug] Relic {:?} already active", relic_id);
                }
            }
            DebugAction::ClearRelics => {
                self.run.relics.active.clear();
                log::info!("[Debug] Cleared all relics");
            }
            DebugAction::ToggleShowFps => {
                self.show_fps = !self.show_fps;
                log::info!("[Debug] Show FPS: {}", self.show_fps);
            }
            DebugAction::OpenTuning => {
                if self.tuning_overlay.is_none() {
                    self.tuning_overlay = Some(TuningOverlay::new(&self.cascade_tuning));
                    log::info!("[Debug] Opened cascade tuning overlay");
                }
            }
        }
        // Request redraw to reflect changes immediately.
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let t_resumed = Instant::now();

        let mut attrs = Window::default_attributes();
        attrs.title = "Mahjuro".to_string();
        attrs.inner_size = Some(PhysicalSize::new(1920, 1080).into());

        let t0 = Instant::now();
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.window = Some(window.clone());
        log::info!("window created in {:?}", t0.elapsed());

        let renderer = WgpuRenderer::new(window.clone()).expect("wgpu");
        self.renderer = Some(renderer);

        let t0 = Instant::now();
        self.input = Some(InputState::new().expect("input"));
        self.debug_menu = Some(DebugMenuBar::new());
        log::info!("input + debug menu init in {:?}", t0.elapsed());

        log::info!("App::resumed() total: {:?}", t_resumed.elapsed());
        window.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let now = Instant::now();
        self.last_frame = now;
        self.anim.update(now);

        match event {
            WindowEvent::CloseRequested => {
                if self.close_saved {
                    log::info!("CloseRequested received again — exiting immediately");
                    event_loop.exit();
                } else {
                    log::info!("CloseRequested — saving profile and exiting");
                    self.progress.record_score(self.run.round_score);
                    let _ = persistence::save_profile(self.active_profile, &self.progress);
                    self.close_saved = true;
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size);
                }
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                // 1. Drain event bus — bus events can trigger scene transitions.
                for ev in self.bus.drain() {
                    match ev {
                        GameEvent::TileDrawn(_) => {
                            self.audio.play_sfx(audio::SfxId::TilePlace);
                        }
                        GameEvent::TileDiscarded { .. } => {
                            self.audio.play_sfx(audio::SfxId::TileDiscard);
                        }
                        GameEvent::ScoreUpdated(_) => {
                            self.audio.play_sfx(audio::SfxId::ScoreReveal);
                        }
                        ev @ GameEvent::RoundComplete { .. } => {
                            // Hold the win sting + scene transition until the
                            // scoring cascade has finished playing out — the
                            // player should get to watch the winning hand pop.
                            self.deferred_round_end = Some(ev);
                        }
                        ev @ GameEvent::GameOver { .. } => {
                            // Same as RoundComplete: hold until the final
                            // cascade has finished animating.
                            self.deferred_round_end = Some(ev);
                        }
                        other => log::info!("event: {other:?}"),
                    }
                }

                // 1b. Poll debug menu actions.
                if let Some(ref debug_menu) = self.debug_menu {
                    for action in debug_menu.poll() {
                        self.handle_debug_action(action);
                    }
                }

                // 2. Collect input actions.
                let mut actions = Vec::new();
                let mut button_clicks: Vec<u32> = Vec::new();
                button_clicks.append(&mut self.mouse_button_clicks);
                let mut hide_cursor = false;
                if let Some(input) = self.input.as_mut() {
                    if input.poll_gamepads(&mut actions) {
                        hide_cursor = true;
                    }
                    actions.append(&mut self.mouse_actions);

                    let size = self
                        .window
                        .as_ref()
                        .map(|w| w.inner_size())
                        .unwrap_or(PhysicalSize::new(800, 600));
                    let layout = self
                        .layout_engine
                        .solve(size.width as f32, size.height as f32);
                    let slots: Vec<(f32, f32, f32, f32)> = layout
                        .hand_slots
                        .iter()
                        .map(|r| (r.x, r.y, r.w, r.h))
                        .collect();
                    input.update_pointer_hover(input.last_cursor, &slots);

                    // 3. Update focus slot (App-level, shared across scenes).
                    for a in &actions {
                        match a {
                            UiAction::FocusNext => {
                                input.focus_slot = (input.focus_slot + 1)
                                    .min(game::run::HAND_SIZE.saturating_sub(1));
                            }
                            UiAction::FocusPrev => {
                                input.focus_slot = input.focus_slot.saturating_sub(1);
                            }
                            _ => {}
                        }
                    }
                }

                if hide_cursor {
                    if let Some(w) = self.window.as_ref() {
                        w.set_cursor_visible(false);
                    }
                }

                // 3b. If the tuning overlay is open, intercept input.
                if let Some(ref mut overlay) = self.tuning_overlay {
                    match overlay.update(&actions) {
                        TuningResult::Stay => {
                            // Apply live tuning changes.
                            self.cascade_tuning = overlay.tuning.clone();
                        }
                        TuningResult::Close => {
                            // Apply final tuning and close.
                            self.cascade_tuning = overlay.tuning.clone();
                            self.tuning_overlay = None;
                            log::info!("[Debug] Closed cascade tuning overlay");
                        }
                        TuningResult::Export => {
                            let json = serde_json::to_string_pretty(&overlay.tuning)
                                .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"));
                            let path = "cascade_tuning.json";
                            match std::fs::write(path, &json) {
                                Ok(()) => log::info!("[Debug] Exported tuning to {path}"),
                                Err(e) => log::error!("[Debug] Failed to export tuning: {e}"),
                            }
                        }
                    }
                    actions.clear();
                    button_clicks.clear();
                }

                // 3c. If a modal is active, intercept input: dismiss on Confirm/Cancel.
                if self.modals.is_active() {
                    for a in &actions {
                        if matches!(a, UiAction::Confirm | UiAction::Cancel) {
                            self.modals.dismiss();
                            break;
                        }
                    }
                    // Block all actions from reaching the scene.
                    actions.clear();
                    button_clicks.clear();
                }

                // 4. Delegate actions to the active scene.
                let focus = self.input.as_ref().map(|i| i.focused_index()).unwrap_or(0);
                let win_size = self
                    .window
                    .as_ref()
                    .map(|w| w.inner_size())
                    .unwrap_or(PhysicalSize::new(800, 600));
                let update_layout = self
                    .layout_engine
                    .solve(win_size.width as f32, win_size.height as f32);
                let mut quit_requested = false;
                let mut switch_profile_req: Option<usize> = None;
                let cursor_pos = self
                    .input
                    .as_ref()
                    .map(|i| i.last_cursor)
                    .unwrap_or((0.0, 0.0));
                let loading_done = self.renderer.as_ref().map_or(true, |r| !r.is_loading());
                let ctx = UpdateCtx {
                    actions: &actions,
                    button_clicks: &button_clicks,
                    run: &mut self.run,
                    bus: &mut self.bus,
                    anim: &mut self.anim,
                    layout: &update_layout,
                    focus_tile_index: focus,
                    quit_requested: &mut quit_requested,
                    switch_profile: &mut switch_profile_req,
                    cursor_pos,
                    loading_done,
                    cascade_tuning: &self.cascade_tuning,
                };
                if let Some(next_scene) = self.scene.update(ctx) {
                    // Start fade-out transition.
                    self.pending_scene = Some(next_scene);
                    self.transition_alpha = 1.0;
                }

                // Sync live audio/graphics settings when in Options scene.
                if let Scene::Options(opts) = &self.scene {
                    self.audio.set_master_volume(opts.master_volume);
                    self.audio.set_sfx_volume(opts.sfx_volume);
                    self.audio.set_music_volume(opts.music_volume);
                    self.audio.set_enabled(opts.sfx_enabled);
                    self.smoke_intensity = opts.smoke_intensity;
                }

                // Handle profile switch request.
                if let Some(idx) = switch_profile_req {
                    let new_idx = if idx == usize::MAX {
                        // Previous profile (wrapping), from start screen arrows.
                        (self.active_profile + 3 - 1) % 3
                    } else if idx == usize::MAX - 1 {
                        // Next profile (wrapping), from start screen arrows.
                        (self.active_profile + 1) % 3
                    } else {
                        // Absolute index, from profile select scene.
                        idx.min(2)
                    };
                    if new_idx != self.active_profile {
                        self.switch_profile(new_idx);
                    }
                }

                // If we deferred a round-end event so the player could watch
                // the scoring cascade play out, fire it now that the gameplay
                // scene has gone idle.
                if self.deferred_round_end.is_some() {
                    let cascade_done = match &self.scene {
                        Scene::Gameplay(g) => !g.is_animating(),
                        _ => true,
                    };
                    if cascade_done {
                        if let Some(ev) = self.deferred_round_end.take() {
                            self.handle_round_end_event(ev);
                        }
                    }
                }

                // Advance transition animation using the animation controller.
                if self.pending_scene.is_some() {
                    self.transition_alpha -= 0.08;
                    if self.transition_alpha <= 0.0 {
                        self.transition_alpha = 0.0;
                        if let Some(next) = self.pending_scene.take() {
                            // If we're transitioning out of the GameOver scene,
                            // surface any deferred celebration modals now.
                            if matches!(self.scene, Scene::GameOver(_))
                                && !self.pending_post_game_over_modals.is_empty()
                            {
                                for modal in self.pending_post_game_over_modals.drain(..) {
                                    self.modals.push(modal);
                                }
                            }
                            self.scene = next;
                            if let Some(input) = self.input.as_mut() {
                                input.focus_slot = 0;
                            }
                            // Fade score panel in for the new scene.
                            self.anim
                                .fade(render::animation::ENTITY_SCORE_PANEL, 0.0, 1.0, 300);
                            // Slide hand strip up from below.
                            self.anim.slide_to(
                                render::animation::ENTITY_HAND_STRIP,
                                0.0,
                                -20.0,
                                400,
                            );
                        }
                    }
                } else if self.transition_alpha < 1.0 {
                    self.transition_alpha = (self.transition_alpha + 0.08).min(1.0);
                }

                // Handle quit request from scene.
                if quit_requested {
                    self.quit_requested = true;
                }

                // Inject fluid impulses from cursor movement.
                if self.smoke_intensity != crate::persistence::SmokeIntensity::Off {
                    if let Some(ref mut renderer) = self.renderer {
                        if let Some(ref mut fluid) = renderer.fluid {
                            let cursor = self
                                .input
                                .as_ref()
                                .map(|i| i.last_cursor)
                                .unwrap_or((0.0, 0.0));
                            let now = Instant::now();
                            let dt = now
                                .saturating_duration_since(self.last_frame)
                                .as_secs_f32()
                                .max(1.0 / 120.0);
                            let vx = (cursor.0 - self.prev_cursor.0) / dt;
                            let vy = (cursor.1 - self.prev_cursor.1) / dt;
                            let speed = (vx * vx + vy * vy).sqrt();
                            self.prev_cursor = cursor;

                            // Only inject when cursor is moving noticeably.
                            if speed > 5.0 {
                                fluid.inject_impulse(
                                    cursor.0,
                                    cursor.1,
                                    4.0,
                                    vx * 0.3,
                                    vy * 0.3,
                                    [0.85, 0.55, 0.3], // amber
                                    0.5,
                                );
                            }
                        }
                    }
                }

                self.draw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    let cursor = self
                        .input
                        .as_ref()
                        .map(|i| i.last_cursor)
                        .unwrap_or((0.0, 0.0));

                    if state == ElementState::Pressed {
                        // Check if click hit any button.
                        let mut hit = false;
                        for btn in &self.active_buttons {
                            let (bx, by, bw, bh) = btn.rect;
                            if cursor.0 >= bx
                                && cursor.0 <= bx + bw
                                && cursor.1 >= by
                                && cursor.1 <= by + bh
                            {
                                match btn.action {
                                    ButtonAction::Ui(a) => self.mouse_actions.push(a),
                                    ButtonAction::Scene(id) => self.mouse_button_clicks.push(id),
                                }
                                hit = true;
                                break;
                            }
                        }
                        if !hit {
                            // Check if we're clicking on a hand tile to start drag.
                            if let Some(input) = self.input.as_mut() {
                                if let Some(slot) = input.pointer_slot {
                                    input.drag = Some(ui::input::DragState {
                                        from_slot: slot,
                                        start_pos: cursor,
                                        current_pos: cursor,
                                    });
                                    // Only confirm (toggle-select tile) if a hand tile was clicked.
                                    self.mouse_actions.push(UiAction::Confirm);
                                }
                            }
                        }
                    } else if state == ElementState::Released {
                        // End drag — swap tiles if dropped on a different slot.
                        // Require minimum drag distance to avoid accidental swaps.
                        if let Some(input) = self.input.as_mut() {
                            if let Some(drag) = input.drag.take() {
                                let dx = cursor.0 - drag.start_pos.0;
                                let dy = cursor.1 - drag.start_pos.1;
                                let dist = (dx * dx + dy * dy).sqrt();
                                if dist > 10.0 {
                                    if let Some(target_slot) = input.pointer_slot {
                                        if target_slot != drag.from_slot {
                                            self.run.swap_tiles(drag.from_slot, target_slot);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (Some(input), Some(win)) = (self.input.as_mut(), self.window.as_ref()) {
                    let was_hidden = input.mode != InputMode::Cursor;
                    input.mode = InputMode::Cursor;
                    input.last_cursor = (position.x as f32, position.y as f32);
                    let size = win.inner_size();
                    let layout = self
                        .layout_engine
                        .solve(size.width as f32, size.height as f32);
                    let slots: Vec<(f32, f32, f32, f32)> = layout
                        .hand_slots
                        .iter()
                        .map(|r| (r.x, r.y, r.w, r.h))
                        .collect();
                    input.update_pointer_hover(input.last_cursor, &slots);
                    // Update drag position if dragging.
                    if let Some(ref mut drag) = input.drag {
                        drag.current_pos = input.last_cursor;
                    }
                    if was_hidden {
                        win.set_cursor_visible(true);
                    }
                    win.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    let mut v = Vec::new();
                    let mode_changed = if let Some(input) = self.input.as_mut() {
                        input.on_key(event.physical_key, &mut v)
                    } else {
                        false
                    };
                    self.mouse_actions.extend(v);
                    if mode_changed {
                        if let Some(w) = self.window.as_ref() {
                            w.set_cursor_visible(false);
                        }
                    }
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.quit_requested {
            let _ = persistence::save_profile(self.active_profile, &self.progress);
            let _ = persistence::save_settings(&persistence::load_settings());
            _event_loop.exit();
            return;
        }
        let cascade_active = matches!(&self.scene, Scene::Gameplay(g) if g.is_animating());
        let transitioning = self.pending_scene.is_some() || self.transition_alpha < 1.0;
        let needs_redraw = !self.anim.is_idle()
            || self
                .renderer
                .as_ref()
                .map(|r| r.is_spinning())
                .unwrap_or(false)
            || cascade_active
            || transitioning
            || self.modals.needs_redraw()
            || self.smoke_intensity != crate::persistence::SmokeIntensity::Off
            || self.tooltips.is_active();
        if needs_redraw {
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    crash_guard::install();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Headless bot mode for tuning. Examples:
    //   mahjuro --bot 200
    //   mahjuro --bot 200 --base-target 250 --target-scale 1.3 --plays 5
    //   mahjuro --sweep                              (default sweep grid)
    //   mahjuro --sweep --runs 50
    let args: Vec<String> = std::env::args().collect();
    let arg_value = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let parse_f32 = |name: &str| arg_value(name).and_then(|s| s.parse::<f32>().ok());
    let parse_u32 = |name: &str| arg_value(name).and_then(|s| s.parse::<u32>().ok());

    let bot_config = bot::BotConfig {
        base_target: parse_u32("--base-target"),
        target_scaling: parse_f32("--target-scale"),
        starting_plays: parse_u32("--plays"),
        starting_discards: parse_u32("--discards"),
        starting_gold: parse_u32("--gold"),
    };

    if args.iter().any(|a| a == "--sweep") {
        let runs = parse_u32("--runs").unwrap_or(40);
        // Default sweep grid — covers ranges most likely useful for tuning.
        let bases: &[u32] = &[200, 250, 300, 350];
        let scales: &[f32] = &[1.20, 1.30, 1.40, 1.50];
        let plays: &[u32] = &[4, 5];
        bot::run_sweep(runs, bases, scales, plays);
        return Ok(());
    }
    if let Some(pos) = args.iter().position(|a| a == "--bot") {
        let n: u32 = args
            .get(pos + 1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(100);
        bot::run_headless(n, bot_config);
        return Ok(());
    }

    asset_path::log_all_assets();

    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<()> {
            let event_loop = EventLoop::new()?;
            event_loop.set_control_flow(ControlFlow::Poll);

            let mut app = App::new();
            event_loop.run_app(&mut app)?;
            Ok(())
        }));

    match result {
        Ok(inner) => inner,
        Err(_) => {
            crash_guard::show_crash_report();
            std::process::exit(1);
        }
    }
}
