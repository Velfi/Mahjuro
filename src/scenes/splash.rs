//! Splash screen — displayed immediately on app start before anything else.

use std::time::Instant;

use crate::render::wgpu_renderer::{GpuInstance, TextLabel};

use super::{DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx};
use super::start_screen::StartScreenScene;

const MIN_DISPLAY_SECS: f32 = 3.0;

pub struct SplashScene {
    /// When the splash was first shown.
    start: Instant,
    /// Set once we've requested the transition so we don't repeat it.
    done: bool,
}

impl SplashScene {
    pub fn new() -> Self {
        Self { start: Instant::now(), done: false }
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
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

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = (w.min(h)) / 600.0;

        // Dark background.
        let instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: [0.04, 0.05, 0.08, 1.0],
        }];

        let mut text_labels = Vec::new();

        // Title: "MAHJURO" in large capitals.
        let title_h = (120.0 * scale).max(48.0);
        let title_y = h * 0.30;
        text_labels.push(TextLabel {
            rect: [0.0, title_y, w, title_h],
            text: "MAHJURO".into(),
            color: [1.0, 0.95, 0.7, 1.0],
        });

        // Tagline below the title.
        let tagline_h = (28.0 * scale).max(16.0);
        let tagline_y = title_y + title_h + h * 0.04;
        text_labels.push(TextLabel {
            rect: [w * 0.15, tagline_y, w * 0.7, tagline_h],
            text: "Mahjong, reimagined for chaos.".into(),
            color: [0.55, 0.55, 0.65, 0.85],
        });

        SceneDrawOutput {
            background: Default::default(),
            instances,
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels,
            relic_icons: vec![],
            buttons: vec![],
            window_title: "Mahjuro".into(),
            departing_indices: vec![],
            hint_indices: vec![],
        }
    }
}
