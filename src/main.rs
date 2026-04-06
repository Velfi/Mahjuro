//! Mahjuro — UI-first shell: winit + wgpu + cassowary + input + scene system.

pub mod asset_path;
mod core;
mod game;
mod persistence;
mod render;
mod scenes;
mod ui;

use std::sync::Arc;
use std::time::Instant;

use game::event_bus::{EventBus, GameEvent};
use game::run::{pick_relic_choices, RunState};
use render::animation::AnimationController;
use render::wgpu_renderer::WgpuRenderer;
use scenes::{ButtonDef, DrawCtx, Scene, UpdateCtx};
use scenes::game_over::GameOverScene;
use scenes::results::ResultsScene;
use scenes::start_screen::StartScreenScene;
use ui::input::{InputMode, InputState, UiAction};
use ui::layout::UiLayout;
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
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            layout_engine: UiLayout::new(),
            input: None,
            run: RunState::new_demo(),
            bus: EventBus::default(),
            anim: AnimationController::new(),
            last_frame: Instant::now(),
            mouse_actions: Vec::new(),
            active_buttons: Vec::new(),
            scene: Scene::StartScreen(StartScreenScene::new()),
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
        };
        let output = self.scene.draw(ctx);

        win.set_title(&output.window_title);
        self.active_buttons = output.buttons;

        renderer.update_hand_tiles(&output.hand_tiles);

        if let Err(e) = renderer.render(
            &output.instances,
            &output.hand_slots,
            output.focus,
            &output.selected_tiles,
            &output.text_labels,
            &output.relic_icons,
        ) {
            log::error!("render: {e:?}");
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
                let mut progress = persistence::load_or_new();
                progress.record_score(self.run.round_score);
                let _ = persistence::save(&progress);
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
                        GameEvent::RoundComplete { .. } => {
                            let count = self.run.blind.relic_choices();
                            let choices = pick_relic_choices(&self.run.relics, count);
                            self.scene = Scene::Results(ResultsScene::new(choices));
                        }
                        GameEvent::GameOver { .. } => {
                            self.scene = Scene::GameOver(GameOverScene::new(
                                self.run.round_score,
                                self.run.target_score,
                            ));
                        }
                        other => log::info!("event: {other:?}"),
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

                // 4. Delegate actions to the active scene.
                let focus = self.input.as_ref().map(|i| i.focused_index()).unwrap_or(0);
                let ctx = UpdateCtx {
                    actions: &actions,
                    run: &mut self.run,
                    bus: &mut self.bus,
                    anim: &mut self.anim,
                    focus_tile_index: focus,
                };
                if let Some(next_scene) = self.scene.update(ctx) {
                    self.scene = next_scene;
                    if let Some(input) = self.input.as_mut() {
                        input.focus_slot = 0;
                    }
                }

                self.draw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left && state == ElementState::Pressed {
                    let cursor = self.input.as_ref()
                        .map(|i| i.last_cursor)
                        .unwrap_or((0.0, 0.0));
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
                        self.mouse_actions.push(UiAction::Confirm);
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
        let cascade_active = matches!(&self.scene, Scene::Gameplay(g) if g.is_animating());
        let needs_redraw = !self.anim.is_idle()
            || self.renderer.as_ref().map(|r| r.is_spinning()).unwrap_or(false)
            || cascade_active;
        if needs_redraw {
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let _progress = persistence::load_or_new();

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new();
    event_loop.run_app(&mut app)?;
    Ok(())
}
