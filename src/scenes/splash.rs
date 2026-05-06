//! Splash screen — displayed immediately on app start before anything else.

use std::time::Instant;

use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};

use crate::render::draw_cmd::UiFrame;

use super::main_menu_exterior::MainMenuExteriorScene;
use super::{DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

const MIN_DISPLAY_SECS: f32 = 0.5;

pub struct SplashScene {
    /// When the splash was first shown.
    start: Instant,
    /// Set once we've requested the transition so we don't repeat it.
    done: bool,
}

impl SplashScene {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            done: false,
        }
    }
}

impl SceneBehavior for SplashScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        if self.done {
            return None;
        }

        let elapsed = self.start.elapsed().as_secs_f32();

        // Wait for both the minimum display time and background loading to finish.
        if ctx.loading_done && elapsed >= MIN_DISPLAY_SECS {
            self.done = true;
            log::info!("splash: transitioning to start screen after {elapsed:.2}s");
            return Some(Scene::MainMenuExterior(MainMenuExteriorScene::new()));
        }

        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w.min(h)) / 600.0 * ctx.ui_scale;

        let mut frame = UiFrame::new();

        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: [0.0, 0.0, 0.0, 1.0],
        });

        if !ctx.modal_active {
            let label_h = (32.0 * scale).max(18.0);
            let label_y = (h - label_h) * 0.5;
            frame.text(TextLabel {
                rect: [0.0, label_y, w, label_h],
                text: "loading...".into(),
                color: color::STONE,
                ..Default::default()
            });
        }

        frame.window_title = "Mahjuro".into();
        frame
    }
}
