//! Splash screen — displayed immediately on app start before anything else.

use std::time::Instant;

use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};

use crate::render::draw_cmd::UiFrame;

use super::start_screen::StartScreenScene;
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
            return Some(Scene::StartScreen(StartScreenScene::new()));
        }

        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w.min(h)) / 600.0 * ctx.ui_scale;

        let mut frame = UiFrame::new();

        // Dark background.
        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::OBSIDIAN,
        });

        // Title: "MAHJURO" in large capitals.
        let title_h = (120.0 * scale).max(48.0);
        let title_y = h * 0.30;
        frame.text(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "MAHJURO".into(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });

        // Tagline below the title.
        let tagline_h = (28.0 * scale).max(16.0);
        let tagline_y = title_y + title_h + h * 0.04;
        frame.text(TextLabel {
            rect: [w * 0.15, tagline_y, w * 0.7, tagline_h],
            text: "Mahjong, reimagined for chaos.".into(),
            color: color::MIST,
            ..Default::default()
        });

        frame.window_title = "Mahjuro".into();
        frame
    }
}
