//! Fullscreen celebration backdrop: dimmer quad, depth reset for 3D, and
//! shared bottom prompt copy. Used by the shop tile-pack flow and
//! [`super::showcase::ZodiacPresenter`].
//!
//! Prefer [`CelebrationOverlayScratch`] so fullscreen passes stay in the right
//! order: **dimmer → optional starfield → depth reset → celebration 3D**.
//!
//! [`ShootingStarCelebrationIntro`] drives the shooting-star cascade wipe used for
//! the zodiac ribbon celebration. When [`Self::force_shooting_star_wipe`] is set
//! (see [`ShootingStarCelebrationIntro::new_zodiac`]), the wipe runs whenever the
//! celebration opens; otherwise it follows [`EffectLayers::transition_fullscreen_fx`].
//! Shader layer counts use the user's Options → Effects tier, not baseline
//! [`EffectLayers::procedural_surface_quality`].

use std::time::{Duration, Instant};

use super::UpdateCtx;

use crate::sfx_id::SfxId;
use crate::effect_layers::EffectLayers;
use crate::game::event_bus::{EventBus, GameEvent};
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::typography;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};

/// Semi-transparent black dimmer (shop pack + zodiac overlay).
pub const DIMMER_RGBA: [f32; 4] = [0.0, 0.0, 0.0, 0.72];

/// Sequences celebration fullscreen layers in a safe order.
///
/// 1. [`Self::push_dimmer_scaled`] — darkens the scene (or replaces clear for overlay scenes).
/// 2. [`AfterCelebrationDimmer::push_starfield_if`] — optional mid-layer (zodiac).
/// 3. [`AfterCelebrationDimmer::push_depth_reset_for_celebration_mesh`] — required
///    before any celebration `Object3d` / showcase tiles.
#[derive(Clone, Copy, Debug)]
pub struct CelebrationOverlayScratch {
    w: f32,
    h: f32,
}

/// Proof that the celebration dimmer ran — call depth reset (and optionally starfield) before 3D.
#[must_use = "call push_starfield_if and push_depth_reset_for_celebration_mesh (or chain them) before celebration 3D"]
#[derive(Clone, Copy, Debug)]
pub struct AfterCelebrationDimmer;

impl CelebrationOverlayScratch {
    #[inline]
    pub fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }

    /// Dimmer with overall opacity multiplier (0 = invisible, 1 = full [`DIMMER_RGBA`] alpha).
    #[inline]
    pub fn push_dimmer_scaled(
        self,
        frame: &mut UiFrame,
        dimmer_alpha_mul: f32,
    ) -> AfterCelebrationDimmer {
        push_dimmer_quad_scaled(frame, self.w, self.h, dimmer_alpha_mul);
        AfterCelebrationDimmer
    }
}

impl AfterCelebrationDimmer {
    /// Optional fullscreen effect between dimmer and depth reset (e.g. zodiac starfield).
    #[inline]
    pub fn push_starfield_if(self, frame: &mut UiFrame, enabled: bool) -> Self {
        if enabled {
            frame.starfield();
        }
        self
    }

    /// Always call this before celebration 3D meshes.
    #[inline]
    pub fn push_depth_reset_for_celebration_mesh(self, frame: &mut UiFrame) {
        apply_depth_reset_for_celebration_mesh(frame);
    }
}

#[inline]
fn push_dimmer_quad_scaled(frame: &mut UiFrame, w: f32, h: f32, dimmer_alpha_mul: f32) {
    let m = dimmer_alpha_mul.clamp(0.0, 1.0);
    let mut c = DIMMER_RGBA;
    c[3] *= m;
    frame.quad(GpuInstance {
        rect: [0.0, 0.0, w, h],
        color: c,
        user: 0,
    });
}

#[inline]
fn apply_depth_reset_for_celebration_mesh(frame: &mut UiFrame) {
    frame.clear_scene_depth();
}

/// Pulse factor for bottom prompts (`t` in seconds).
#[inline]
pub fn prompt_pulse_alpha(t_secs: f32) -> f32 {
    0.5 + 0.5 * (t_secs * 3.0).sin()
}

const PROMPT_NY: f32 = 0.88;

fn bottom_prompt_label(h: f32, w: f32, text: impl Into<String>, alpha: f32) -> TextLabel {
    let prompt_font = typography::size(typography::H42, h);
    let prompt_y = h * PROMPT_NY;
    TextLabel {
        text: text.into(),
        rect: [0.0, prompt_y, w, prompt_font * 1.5],
        font_px: Some(prompt_font),
        color: [1.0, 1.0, 1.0, alpha],
        align: TextAlign::Center,
        ..Default::default()
    }
}

pub fn label_confirm_to_continue(h: f32, w: f32, t_secs: f32, overall_alpha: f32) -> TextLabel {
    bottom_prompt_label(
        h,
        w,
        "Click or press confirm to continue",
        overall_alpha * prompt_pulse_alpha(t_secs),
    )
}

/// TPOS2 anticipation prompt.
pub fn label_confirm_to_unseal(h: f32, w: f32, t_secs: f32, overall_alpha: f32) -> TextLabel {
    bottom_prompt_label(
        h,
        w,
        "Click or press confirm to unseal",
        overall_alpha * prompt_pulse_alpha(t_secs),
    )
}

/// Zodiac level-up header (warm gold, fades with `alpha`).
pub fn label_zodiac_level_title(h: f32, w: f32, text: String, alpha: f32) -> TextLabel {
    let title_font = typography::size(typography::H24, h);
    let title_y = h * 0.10;
    TextLabel {
        text,
        rect: [0.0, title_y, w, title_font * 1.5],
        font_px: Some(title_font),
        color: [0.95, 0.78, 0.25, alpha],
        align: TextAlign::Center,
        ..Default::default()
    }
}

/// Intro motion applied to celebration hero meshes and title bands.
#[derive(Clone, Copy, Debug, Default)]
pub struct CelebrationContentDrift {
    pub scale: f32,
    pub xy: [f32; 2],
}

impl CelebrationContentDrift {
    #[inline]
    pub fn identity() -> Self {
        Self {
            scale: 1.0,
            xy: [0.0, 0.0],
        }
    }

    #[inline]
    pub fn apply_to_pos(&self, pos: [f32; 3]) -> [f32; 3] {
        [
            pos[0] + self.xy[0],
            pos[1] + self.xy[1],
            pos[2],
        ]
    }
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Wall-clock intro that pairs a shooting-star cascade wipe with fading-in celebration content.
///
/// Append [`Self::push_shooting_star_cascade_if_active`] **last** in the frame so the wipe
/// composites over celebration geometry.
#[derive(Clone, Copy, Debug)]
pub struct ShootingStarCelebrationIntro {
    started_at: Instant,
    sound_emitted: bool,
    /// Zodiac / meta level-up: always run the cascade + content fade-in when the overlay appears.
    force_shooting_star_wipe: bool,
}

impl ShootingStarCelebrationIntro {
    /// Total wipe duration (matches typical app transition timing).
    pub const DURATION_SECS: f32 = 1.7;
    const CONTENT_FADE_START: f32 = 0.42;
    const CONTENT_FADE_END: f32 = 0.92;

    #[inline]
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            sound_emitted: false,
            force_shooting_star_wipe: false,
        }
    }

    /// Ribbon celebration on [`crate::scenes::ShowcasePresenter::Zodiac`]: play the wipe even when
    /// [`EffectLayers::transition_fullscreen_fx`] is off (e.g. baseline graphics).
    #[inline]
    pub fn new_zodiac() -> Self {
        Self {
            started_at: Instant::now(),
            sound_emitted: false,
            force_shooting_star_wipe: true,
        }
    }

    /// Meta profile level-up showcase: same forced wipe as zodiac so the cascade still plays when
    /// [`EffectLayers::transition_fullscreen_fx`] is off.
    #[inline]
    pub fn new_meta_level_up() -> Self {
        Self::new_zodiac()
    }

    /// TPOS2 tile-pack opening: always run the cascade when the overlay appears.
    #[inline]
    pub fn new_pack_opening() -> Self {
        Self::new_zodiac()
    }

    #[inline]
    fn wipe_active(&self, layers: &EffectLayers) -> bool {
        self.force_shooting_star_wipe || layers.transition_fullscreen_fx
    }

    /// Headless / fast-forward: treat the wipe as finished and skip audio.
    #[inline]
    pub fn jump_to_done(&mut self) {
        self.started_at = Instant::now() - Duration::from_secs_f32(Self::DURATION_SECS + 1.0);
        self.sound_emitted = true;
    }

    #[inline]
    fn norm_t(&self) -> f32 {
        (Instant::now()
            .saturating_duration_since(self.started_at)
            .as_secs_f32()
            / Self::DURATION_SECS)
            .clamp(0.0, 1.0)
    }

    /// Shader progress for [`DrawCmd::ShootingStarCascade`].
    #[inline]
    pub fn transition_progress(&self) -> f32 {
        self.norm_t()
    }

    #[inline]
    pub fn is_complete(&self) -> bool {
        self.norm_t() >= 1.0
    }

    /// When no wipe is active for this intro, content is fully visible immediately.
    #[inline]
    pub fn is_done_for(&self, layers: &EffectLayers) -> bool {
        !self.wipe_active(layers) || self.is_complete()
    }

    /// Fade factor for ribbon, text, and dimmer during the wipe.
    #[inline]
    pub fn content_alpha_for(&self, layers: &EffectLayers) -> f32 {
        if !self.wipe_active(layers) {
            return 1.0;
        }
        let t = self.transition_progress();
        smoothstep(Self::CONTENT_FADE_START, Self::CONTENT_FADE_END, t)
    }

    /// Subtle scale + vertical drift while celebration content eases in after the wipe.
    #[inline]
    pub fn content_drift_for(&self, _w: f32, h: f32, layers: &EffectLayers) -> CelebrationContentDrift {
        if !self.wipe_active(layers) {
            return CelebrationContentDrift::identity();
        }
        let t = self.transition_progress();
        let ease = smoothstep(Self::CONTENT_FADE_START, Self::CONTENT_FADE_END, t);
        let inv = 1.0 - ease;
        CelebrationContentDrift {
            scale: 0.92 + 0.08 * ease,
            xy: [0.0, h * 0.04 * inv],
        }
    }

    #[inline]
    pub fn tick_audio(&mut self, bus: &mut EventBus, headless: bool, layers: &EffectLayers) {
        if headless || !self.wipe_active(layers) || self.sound_emitted {
            return;
        }
        bus.push(GameEvent::UiSound(SfxId::StarShimmer));
        self.sound_emitted = true;
    }

    /// Sets [`UiFrame::transition_progress`] and queues the cascade pass (no-op when inactive).
    #[inline]
    pub fn push_shooting_star_cascade_if_active(&self, frame: &mut UiFrame, layers: &EffectLayers) {
        if !self.wipe_active(layers) || self.is_complete() {
            return;
        }
        frame.transition_progress = self.transition_progress();
        frame.shooting_star_cascade();
    }
}

impl Default for ShootingStarCelebrationIntro {
    fn default() -> Self {
        Self::new()
    }
}

/// Audio, headless fast-forward, and post-wipe grace window shared by zodiac and meta level-up showcases.
pub struct CelebrationShowcaseIntroGate {
    pub intro: ShootingStarCelebrationIntro,
    intro_grace_start: Option<Instant>,
}

impl CelebrationShowcaseIntroGate {
    pub const GRACE_AFTER_DONE_SECS: f32 = 0.35;

    #[inline]
    pub fn new(intro: ShootingStarCelebrationIntro) -> Self {
        Self {
            intro,
            intro_grace_start: None,
        }
    }

    pub fn tick(&mut self, ctx: &mut UpdateCtx<'_>) {
        self.intro
            .tick_audio(ctx.bus, ctx.headless, &ctx.effect_layers);
        if ctx.headless {
            self.intro.jump_to_done();
            self.intro_grace_start = Some(Instant::now() - Duration::from_secs_f32(1.0));
        }
        if self.intro.is_done_for(&ctx.effect_layers) && self.intro_grace_start.is_none() {
            self.intro_grace_start = Some(Instant::now());
        }
    }

    #[inline]
    pub fn ready_for_dismiss(&self, layers: &EffectLayers) -> bool {
        if !self.intro.is_done_for(layers) {
            return false;
        }
        match self.intro_grace_start {
            Some(t) => {
                Instant::now().saturating_duration_since(t).as_secs_f32()
                    >= Self::GRACE_AFTER_DONE_SECS
            }
            None => false,
        }
    }
}
