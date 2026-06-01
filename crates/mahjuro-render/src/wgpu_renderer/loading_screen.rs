//! Shared boot + splash loading plate: production logo sequence, progress bar,
//! and SDF "loading..." label layout.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::draw_cmd::{ImageQuad, ImageQuadSource, UiFrame};
use crate::wgpu_renderer::TextLabel;
use crate::wgpu_renderer::ui_instances::GpuInstance;

pub const LOADING_LOGO_ASSET: &str = "textures/loading/zelda_built_this.png";

const LOGO_FADE_IN_SECS: f32 = 1.0;
const LOGO_HOLD_SECS: f32 = 3.0;
const LOGO_FADE_OUT_SECS: f32 = 1.0;
const LOADING_FADE_IN_SECS: f32 = 1.0;
const LOGO_FADE_OUT_START: f32 = LOGO_FADE_IN_SECS + LOGO_HOLD_SECS;
const LOADING_FADE_IN_START: f32 = LOGO_FADE_OUT_START + LOGO_FADE_OUT_SECS;
const LOGO_SEQUENCE_SECS: f32 = LOADING_FADE_IN_START + LOADING_FADE_IN_SECS;
const SKIP_BLEND_SECS: f32 = 0.3;

const BOOT_PROGRESS_WEIGHT: f32 = 0.85;
const SPLASH_PROGRESS_WEIGHT: f32 = 0.15;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LoadingAlphas {
    pub logo: f32,
    pub loading_ui: f32,
}

#[derive(Debug, Default)]
struct LoadingScreenClock {
    /// Logo timeline while the minimal boot presenter is up (wgpu init).
    boot_start: Option<Instant>,
    /// Logo timeline for the full-renderer splash; starts on first presented frame.
    splash_start: Option<Instant>,
    skip_at: Option<Instant>,
    boot_progress: f32,
}

static CLOCK: OnceLock<Mutex<LoadingScreenClock>> = OnceLock::new();

fn clock() -> &'static Mutex<LoadingScreenClock> {
    CLOCK.get_or_init(|| Mutex::new(LoadingScreenClock::default()))
}

pub fn touch_boot_frame() {
    let mut c = clock().lock().expect("loading clock");
    if c.boot_start.is_none() {
        c.boot_start = Some(Instant::now());
    }
}

/// Begin (or continue) the production-logo timeline on the full-renderer splash.
pub fn touch_splash_logo_frame() {
    let mut c = clock().lock().expect("loading clock");
    if c.splash_start.is_none() {
        c.splash_start = Some(Instant::now());
    }
}

pub fn request_skip() {
    let mut c = clock().lock().expect("loading clock");
    if c.skip_at.is_none() {
        c.skip_at = Some(Instant::now());
    }
}

pub fn set_boot_progress(progress: f32) {
    clock().lock().expect("loading clock").boot_progress = progress.clamp(0.0, 1.0);
}

pub fn combined_progress(splash_hub: f32) -> f32 {
    let c = clock().lock().expect("loading clock");
    (c.boot_progress * BOOT_PROGRESS_WEIGHT + splash_hub.clamp(0.0, 1.0) * SPLASH_PROGRESS_WEIGHT)
        .clamp(0.0, 1.0)
}

pub fn current_alphas() -> LoadingAlphas {
    let c = clock().lock().expect("loading clock");
    let start = c.boot_start.unwrap_or_else(Instant::now);
    alphas_for_elapsed(start, c.skip_at)
}

/// Alphas for the unified splash plate (independent of boot-init duration).
pub fn current_splash_alphas() -> LoadingAlphas {
    let c = clock().lock().expect("loading clock");
    let start = c
        .splash_start
        .or(c.boot_start)
        .unwrap_or_else(Instant::now);
    alphas_for_elapsed(start, c.skip_at)
}

pub fn logo_sequence_complete() -> bool {
    splash_logo_sequence_complete()
}

pub fn splash_logo_sequence_complete() -> bool {
    current_splash_alphas().loading_ui >= 1.0 - 1e-3
}

fn alphas_for_elapsed(start: Instant, skip_at: Option<Instant>) -> LoadingAlphas {
    let elapsed = start.elapsed().as_secs_f32();
    if let Some(skip_at) = skip_at {
        let skip_elapsed = skip_at.elapsed().as_secs_f32();
        if skip_elapsed >= SKIP_BLEND_SECS {
            return LoadingAlphas {
                logo: 0.0,
                loading_ui: 1.0,
            };
        }
        let at_skip = alphas_at_time(elapsed - skip_elapsed);
        let t = (skip_elapsed / SKIP_BLEND_SECS).clamp(0.0, 1.0);
        return LoadingAlphas {
            logo: at_skip.logo * (1.0 - t),
            loading_ui: at_skip.loading_ui + (1.0 - at_skip.loading_ui) * t,
        };
    }
    alphas_at_time(elapsed)
}

fn alphas_at_time(t: f32) -> LoadingAlphas {
    if t <= LOGO_FADE_IN_SECS {
        LoadingAlphas {
            logo: (t / LOGO_FADE_IN_SECS).clamp(0.0, 1.0),
            loading_ui: 0.0,
        }
    } else if t <= LOGO_FADE_OUT_START {
        LoadingAlphas {
            logo: 1.0,
            loading_ui: 0.0,
        }
    } else if t <= LOADING_FADE_IN_START {
        let u = (t - LOGO_FADE_OUT_START) / LOGO_FADE_OUT_SECS;
        LoadingAlphas {
            logo: (1.0 - u).clamp(0.0, 1.0),
            loading_ui: 0.0,
        }
    } else if t <= LOGO_SEQUENCE_SECS {
        let u = (t - LOADING_FADE_IN_START) / LOADING_FADE_IN_SECS;
        LoadingAlphas {
            logo: 0.0,
            loading_ui: u.clamp(0.0, 1.0),
        }
    } else {
        LoadingAlphas {
            logo: 0.0,
            loading_ui: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LoadingLayout {
    pub logo_rect: [f32; 4],
    pub label_rect: [f32; 4],
    pub bar_rect: [f32; 4],
    pub track_color: [f32; 4],
    pub fill_color: [f32; 4],
    pub text_color: [f32; 4],
}

pub fn layout(screen_w: f32, screen_h: f32) -> LoadingLayout {
    let w = screen_w.max(1.0);
    let h = screen_h.max(1.0);
    let scale = w.min(h) / 600.0;

    let logo_side = (w.min(h) * 0.55).max(120.0);
    let logo_x = (w - logo_side) * 0.5;
    let logo_y = (h - logo_side) * 0.5;

    let label_h = (32.0 * scale).max(18.0);
    let label_w = label_h * 3.35;
    let label_x = (w - label_w) * 0.5;
    let label_y = (h - label_h) * 0.5 - (24.0 * scale).max(12.0);

    let bar_w = (w * 0.38).max(120.0);
    let bar_h = (5.0 * scale).max(3.0);
    let bar_x = (w - bar_w) * 0.5;
    let bar_y = label_y + label_h + (14.0 * scale).max(8.0);

    let stone = boot_stone_color();

    LoadingLayout {
        logo_rect: [logo_x, logo_y, logo_side, logo_side],
        label_rect: [label_x, label_y, label_w, label_h],
        bar_rect: [bar_x, bar_y, bar_w, bar_h],
        track_color: [0.22, 0.20, 0.18, 0.85],
        fill_color: stone,
        text_color: stone,
    }
}

fn boot_stone_color() -> [f32; 4] {
    static STONE: OnceLock<[f32; 4]> = OnceLock::new();
    *STONE.get_or_init(|| {
        let json = mahjuro_assets::asset_path::get("data/boot_loading_msdf.json")
            .expect("boot_loading_msdf.json missing; run scripts/bake_boot_loading_msdf.py");
        #[derive(serde::Deserialize)]
        struct Meta {
            color_stone: [f32; 4],
            text_w: f32,
            text_h: f32,
        }
        let meta: Meta = serde_json::from_slice(&json.data).expect("boot_loading_msdf.json parse");
        let _ = (meta.text_w, meta.text_h);
        meta.color_stone
    })
}

pub fn boot_label_aspect() -> f32 {
    static ASPECT: OnceLock<f32> = OnceLock::new();
    *ASPECT.get_or_init(|| {
        let json = mahjuro_assets::asset_path::get("data/boot_loading_msdf.json")
            .expect("boot_loading_msdf.json missing; run scripts/bake_boot_loading_msdf.py");
        #[derive(serde::Deserialize)]
        struct Meta {
            text_w: f32,
            text_h: f32,
        }
        let meta: Meta = serde_json::from_slice(&json.data).expect("boot_loading_msdf.json parse");
        meta.text_w / meta.text_h.max(1.0)
    })
}

pub fn layout_with_msdf_label(screen_w: f32, screen_h: f32) -> LoadingLayout {
    let mut layout = layout(screen_w, screen_h);
    let aspect = boot_label_aspect();
    let label_h = layout.label_rect[3];
    let label_w = label_h * aspect;
    layout.label_rect[0] = (screen_w - label_w) * 0.5;
    layout.label_rect[2] = label_w;
    layout
}

/// Append the unified loading plate to a full-renderer [`UiFrame`].
pub fn append_splash_frame(frame: &mut UiFrame, screen_w: f32, screen_h: f32, progress: f32, alphas: LoadingAlphas) {
    frame.quad(GpuInstance {
        rect: [0.0, 0.0, screen_w, screen_h],
        color: [0.0, 0.0, 0.0, 1.0],
        user: 0,
    });

    if alphas.logo > 0.004 {
        let layout = layout(screen_w, screen_h);
        frame.image_quads([ImageQuad {
            inst: GpuInstance {
                rect: layout.logo_rect,
                color: [1.0, 1.0, 1.0, alphas.logo],
                user: 0,
            },
            source: ImageQuadSource::Asset {
                path: LOADING_LOGO_ASSET,
            },
        }]);
    }

    if alphas.loading_ui > 0.004 {
        let layout = layout_with_msdf_label(screen_w, screen_h);
        let ui_a = alphas.loading_ui;

        frame.quad(GpuInstance {
            rect: layout.bar_rect,
            color: [
                layout.track_color[0],
                layout.track_color[1],
                layout.track_color[2],
                layout.track_color[3] * ui_a,
            ],
            user: 0,
        });

        let fill_w = (layout.bar_rect[2] * progress).max(0.0);
        if fill_w > 0.5 {
            frame.quad(GpuInstance {
                rect: [layout.bar_rect[0], layout.bar_rect[1], fill_w, layout.bar_rect[3]],
                color: [
                    layout.fill_color[0],
                    layout.fill_color[1],
                    layout.fill_color[2],
                    layout.fill_color[3] * ui_a,
                ],
                user: 0,
            });
        }

        let label_h = layout.label_rect[3];
        frame.text(TextLabel {
            rect: layout.label_rect,
            text: "loading...".into(),
            color: [
                layout.text_color[0],
                layout.text_color[1],
                layout.text_color[2],
                layout.text_color[3] * ui_a,
            ],
            font_px: Some(label_h * 0.95),
            ..Default::default()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_sequence_timeline() {
        assert_eq!(alphas_at_time(0.0).logo, 0.0);
        assert!((alphas_at_time(0.5).logo - 0.5).abs() < 0.01);
        assert!((alphas_at_time(1.0).logo - 1.0).abs() < 0.01);
        assert!((alphas_at_time(2.5).logo - 1.0).abs() < 0.01);
        assert!((alphas_at_time(4.5).logo - 0.5).abs() < 0.01);
        assert!((alphas_at_time(5.0).logo).abs() < 0.01);
        assert!((alphas_at_time(5.5).loading_ui - 0.5).abs() < 0.01);
        assert!((alphas_at_time(6.0).loading_ui - 1.0).abs() < 0.01);
    }

    #[test]
    fn splash_logo_stays_up_during_hold() {
        let start = Instant::now() - std::time::Duration::from_secs_f32(2.5);
        let alphas = alphas_for_elapsed(start, None);
        assert!((alphas.logo - 1.0).abs() < 0.01, "logo should hold at full opacity");
        assert!(alphas.loading_ui < 0.1);
    }
}
