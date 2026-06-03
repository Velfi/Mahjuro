//! Splash screen — unified loading plate until the main-menu hub is ready.

use crate::render::wgpu_renderer::loading_screen::{
    self, append_splash_frame, current_splash_alphas, request_skip, splash_logo_sequence_complete,
};

use crate::render::draw_cmd::UiFrame;
use crate::ui::input::UiAction;

use super::main_menu::MainMenuScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

pub struct SplashScene {
    /// Set once we've requested the transition so we don't repeat it.
    done: bool,
    /// False until the first frame with a live renderer is drawn.
    visible: bool,
}

impl Default for SplashScene {
    fn default() -> Self {
        Self::new()
    }
}

impl SplashScene {
    pub fn new() -> Self {
        Self {
            done: false,
            visible: false,
        }
    }

    /// Mark the loading plate as on-screen and start the production-logo timeline.
    pub fn mark_visible(&mut self) {
        if !self.visible {
            loading_screen::touch_splash_logo_frame();
        }
        self.visible = true;
    }
}

impl SceneBehavior for SplashScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if self.done {
            return None;
        }

        if ctx
            .actions
            .iter()
            .any(|a| matches!(a, UiAction::Confirm | UiAction::Cancel))
        {
            request_skip();
        }

        if ctx.loading_done && self.visible && splash_logo_sequence_complete() {
            self.done = true;
            log::info!("splash: transitioning to start screen");
            return Some(Scene::MainMenu(MainMenuScene::new()));
        }

        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let alphas = current_splash_alphas();
        let hub = ctx.loading_hub_progress;
        let progress = loading_screen::combined_progress(hub);

        let mut frame = UiFrame::new();
        if !ctx.modal_active {
            append_splash_frame(&mut frame, w, h, progress, alphas);
        } else {
            frame.quad(crate::render::wgpu_renderer::GpuInstance {
                rect: [0.0, 0.0, w, h],
                color: [0.0, 0.0, 0.0, 1.0],
                user: 0,
            });
        }

        frame.window_title = "Mahjuro".into();
        frame
    }
}
