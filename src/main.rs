//! Mahjuro — UI-first shell: winit + wgpu + cassowary + input + scene system.

pub mod asset_path;
mod audio;
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
use game::event_bus::{EventBus, GameEvent};
use game::run::{pick_relic_choices, RunState};
use render::animation::AnimationController;
use render::wgpu_renderer::{GpuInstance, TextLabel, WgpuRenderer};
use scenes::{ButtonDef, DrawCtx, Scene, UpdateCtx};
use scenes::game_over::GameOverScene;
use scenes::results::ResultsScene;
use scenes::start_screen::StartScreenScene;
use ui::input::{InputMode, InputState, UiAction};
use ui::layout::UiLayout;
use ui::modal::{Modal, ModalQueue, ModalTheme};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

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
    /// Modal toast queue — overlays any scene.
    modals: ModalQueue,
    /// Native OS debug menu bar.
    debug_menu: Option<DebugMenuBar>,
}

impl App {
    fn new() -> Self {
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
            active_buttons: Vec::new(),
            scene: Scene::StartScreen(StartScreenScene::new()),
            progress,
            active_profile,
            audio,
            transition_alpha: 1.0,
            pending_scene: None,
            quit_requested: false,
            modals: ModalQueue::default(),
            debug_menu: None,
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

    fn draw(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(win) = self.window.as_ref() else {
            return;
        };

        let size = win.inner_size();
        let layout = self.layout_engine.solve(size.width as f32, size.height as f32);
        let focus = self
            .input
            .as_ref()
            .map(|i| i.focused_index())
            .unwrap_or(0);

        let ctx = DrawCtx {
            layout: &layout,
            anim: &self.anim,
            run: &self.run,
            focus_tile_index: focus,
            progress: &self.progress,
            active_profile: self.active_profile,
            game_in_progress: self.run.is_in_progress(),
        };
        let output = self.scene.draw(ctx);

        win.set_title(&output.window_title);
        self.active_buttons = output.buttons;

        renderer.update_hand_tiles(&output.hand_tiles);

        // Apply transition alpha to all instances and text labels.
        let alpha = self.transition_alpha;
        let mut instances: Vec<GpuInstance> = if alpha < 1.0 {
            output
                .instances
                .iter()
                .map(|i| GpuInstance {
                    rect: i.rect,
                    color: [i.color[0], i.color[1], i.color[2], i.color[3] * alpha],
                })
                .collect()
        } else {
            output.instances
        };
        let mut text_labels: Vec<TextLabel> = if alpha < 1.0 {
            output
                .text_labels
                .iter()
                .map(|l| TextLabel {
                    rect: l.rect,
                    text: l.text.clone(),
                    color: [l.color[0], l.color[1], l.color[2], l.color[3] * alpha],
                })
                .collect()
        } else {
            output.text_labels
        };

        // Render modal overlay on top of everything.
        let size = win.inner_size();
        self.modals.update();
        if let Some((modal_insts, modal_labels, modal_buttons)) =
            self.modals.draw(size.width as f32, size.height as f32)
        {
            instances.extend(modal_insts);
            text_labels.extend(modal_labels);
            // Replace scene buttons with modal buttons so only dismiss works.
            self.active_buttons = modal_buttons;
        }

        if let Err(e) = renderer.render(
            &instances,
            &output.hand_slots,
            output.focus,
            &output.selected_tiles,
            &text_labels,
            &output.relic_icons,
        ) {
            log::error!("render: {e:?}");
        }
    }

    fn handle_debug_action(&mut self, action: DebugAction) {
        use crate::core::tile::Suit;
        use crate::game::game_mode::CardEntry;

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
                log::info!("[Debug] Set player level to {} (runs_completed={})", level, runs);
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
            DebugAction::SetCardInventoryStandard => {
                self.run.mode.card_inventory = None;
                log::info!("[Debug] Card inventory set to Standard (136 tiles)");
            }
            DebugAction::SetCardInventoryNumbersOnly => {
                let mut entries = Vec::new();
                for suit in [Suit::Characters, Suit::Bamboos, Suit::Circles] {
                    for rank in 1..=9 {
                        entries.push(CardEntry { suit, rank, copies: 4 });
                    }
                }
                self.run.mode.card_inventory = Some(entries);
                log::info!("[Debug] Card inventory set to Numbers Only");
            }
            DebugAction::SetCardInventoryHonorsOnly => {
                let mut entries = Vec::new();
                for rank in 1..=4 {
                    entries.push(CardEntry { suit: Suit::Wind, rank, copies: 4 });
                }
                for rank in 1..=3 {
                    entries.push(CardEntry { suit: Suit::Dragon, rank, copies: 4 });
                }
                self.run.mode.card_inventory = Some(entries);
                log::info!("[Debug] Card inventory set to Honors Only");
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

        let mut attrs = Window::default_attributes();
        attrs.title = "Mahjuro".to_string();
        attrs.inner_size = Some(PhysicalSize::new(960, 600).into());

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("create window"),
        );
        self.window = Some(window.clone());

        let renderer = WgpuRenderer::new(window.clone()).expect("wgpu");
        self.renderer = Some(renderer);

        self.input = Some(InputState::new().expect("input"));
        self.debug_menu = Some(DebugMenuBar::new());

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
                self.progress.record_score(self.run.round_score);
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                event_loop.exit();
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
                        GameEvent::RoundComplete { .. } => {
                            self.audio.play_sfx(audio::SfxId::RoundWin);
                            let win_size = self.window.as_ref()
                                .map(|w| w.inner_size())
                                .unwrap_or(PhysicalSize::new(800, 600));
                            let ww = win_size.width as f32;
                            let wh = win_size.height as f32;
                            let modal = Modal::new(
                                "Round Complete!",
                                format!("Score: {} / {}  —  Well played!", self.run.round_score, self.run.target_score),
                                ModalTheme::Success,
                            ).with_fireworks(ww * 0.5, wh * 0.8, ww * 0.6, 5);
                            self.modals.push(modal);
                            let count = self.run.blind.relic_choices();
                            let available = self.progress.available_relics();
                            let choices =
                                pick_relic_choices(&self.run.relics, count, &available);
                            self.pending_scene =
                                Some(Scene::Results(ResultsScene::new(choices)));
                            self.transition_alpha = 1.0;
                        }
                        GameEvent::GameOver { .. } => {
                            // Record run completion and check for level-up.
                            self.progress.runs_completed += 1;
                            self.progress.record_score(self.run.round_score);
                            let level_up = self.progress.check_level_up();
                            let _ = persistence::save_profile(self.active_profile, &self.progress);

                            let win_size = self.window.as_ref()
                                .map(|w| w.inner_size())
                                .unwrap_or(PhysicalSize::new(800, 600));
                            let ww = win_size.width as f32;
                            let wh = win_size.height as f32;

                            // Show level-up modal with fireworks if the player leveled up.
                            if let Some(level) = level_up {
                                log::info!("Level up! Now level {}", level);
                                let modal = Modal::new(
                                    format!("Level Up! — Level {}", level),
                                    "New content unlocked!",
                                    ModalTheme::Success,
                                ).with_fireworks(ww * 0.5, wh * 0.7, ww * 0.7, 8);
                                self.modals.push(modal);
                            }

                            // Game over modal (no fireworks).
                            let modal = Modal::new(
                                "Game Over",
                                format!("Final score: {} / {}", self.run.round_score, self.run.target_score),
                                ModalTheme::Failure,
                            );
                            self.modals.push(modal);

                            self.audio.play_sfx(audio::SfxId::GameOver);
                            self.pending_scene = Some(Scene::GameOver(
                                GameOverScene::new(
                                    self.run.round_score,
                                    self.run.target_score,
                                ),
                            ));
                            self.transition_alpha = 1.0;
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
                    let layout = self.layout_engine.solve(size.width as f32, size.height as f32);
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

                // 3b. If a modal is active, intercept input: dismiss on Confirm/Cancel.
                if self.modals.is_active() {
                    for a in &actions {
                        if matches!(a, UiAction::Confirm | UiAction::Cancel) {
                            self.modals.dismiss();
                            break;
                        }
                    }
                    // Block all actions from reaching the scene.
                    actions.clear();
                }

                // 4. Delegate actions to the active scene.
                let focus = self.input.as_ref().map(|i| i.focused_index()).unwrap_or(0);
                let win_size = self
                    .window
                    .as_ref()
                    .map(|w| w.inner_size())
                    .unwrap_or(PhysicalSize::new(800, 600));
                let update_layout =
                    self.layout_engine
                        .solve(win_size.width as f32, win_size.height as f32);
                let mut quit_requested = false;
                let mut switch_profile_req: Option<usize> = None;
                let cursor_pos = self.input.as_ref()
                    .map(|i| i.last_cursor)
                    .unwrap_or((0.0, 0.0));
                let ctx = UpdateCtx {
                    actions: &actions,
                    run: &mut self.run,
                    bus: &mut self.bus,
                    anim: &mut self.anim,
                    layout: &update_layout,
                    focus_tile_index: focus,
                    quit_requested: &mut quit_requested,
                    switch_profile: &mut switch_profile_req,
                    cursor_pos,
                };
                if let Some(next_scene) = self.scene.update(ctx) {
                    // Start fade-out transition.
                    self.pending_scene = Some(next_scene);
                    self.transition_alpha = 1.0;
                }

                // Sync live audio settings when in Options scene.
                if let Scene::Options(opts) = &self.scene {
                    self.audio.set_master_volume(opts.master_volume);
                    self.audio.set_sfx_volume(opts.sfx_volume);
                    self.audio.set_music_volume(opts.music_volume);
                    self.audio.set_enabled(opts.sfx_enabled);
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

                // Advance transition animation using the animation controller.
                if self.pending_scene.is_some() {
                    self.transition_alpha -= 0.08;
                    if self.transition_alpha <= 0.0 {
                        self.transition_alpha = 0.0;
                        if let Some(next) = self.pending_scene.take() {
                            self.scene = next;
                            if let Some(input) = self.input.as_mut() {
                                input.focus_slot = 0;
                            }
                            // Fade score panel in for the new scene.
                            self.anim.fade(
                                render::animation::ENTITY_SCORE_PANEL,
                                0.0, 1.0, 300,
                            );
                            // Slide hand strip up from below.
                            self.anim.slide_to(
                                render::animation::ENTITY_HAND_STRIP,
                                0.0, -20.0, 400,
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

                self.draw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    let cursor = self.input.as_ref()
                        .map(|i| i.last_cursor)
                        .unwrap_or((0.0, 0.0));

                    if state == ElementState::Pressed {
                        // Check if click hit any button.
                        let mut hit = false;
                        for btn in &self.active_buttons {
                            let (bx, by, bw, bh) = btn.rect;
                            if cursor.0 >= bx && cursor.0 <= bx + bw
                                && cursor.1 >= by && cursor.1 <= by + bh
                            {
                                self.mouse_actions.push(btn.action);
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
                    let layout = self.layout_engine.solve(size.width as f32, size.height as f32);
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
            || self.renderer.as_ref().map(|r| r.is_spinning()).unwrap_or(false)
            || cascade_active
            || transitioning
            || self.modals.needs_redraw();
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

    asset_path::log_all_assets();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<()> {
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
