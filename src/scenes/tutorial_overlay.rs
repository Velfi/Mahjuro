//! Tutorial hint overlay — renders contextual banners and pulsing
//! highlights during the tutorial to guide new players through each lesson.
//!
//! Owned by `GameplayScene`. Updated each frame from the current tutorial
//! state and rendered as 2D quads + text labels layered on top of the
//! gameplay HUD.

use crate::core::rules::BlindKind;
use crate::game::run::RunState;
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};

/// Which HUD element should receive a pulsing highlight ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighlightTarget {
    HandTiles,
    PlayButton,
    DiscardBowl,
    ScorePanel,
}

/// Lightweight overlay that renders tutorial hint banners and pulsing
/// highlights. Does not manage its own scene lifecycle — the gameplay
/// scene owns it and calls `update` / `draw` each frame.
pub struct TutorialOverlay {
    /// Primary hint text to show this frame.
    pub hint_text: String,
    /// Italic flavor line above the hint.
    pub flavor_text: String,
    /// Which element to highlight (if any).
    pub highlight: Option<HighlightTarget>,
    /// Whether the banner is dismissed (player clicked/pressed to hide).
    pub dismissed: bool,
    /// Set by the gameplay scene when a scoring cascade is active.
    pub cascade_active: bool,
    /// Monotonic timer for pulsing animations (seconds).
    pub pulse_time: f32,
    /// Fade-in alpha (0.0 → 1.0 over the first 0.5s of each lesson).
    pub fade_alpha: f32,
}

impl TutorialOverlay {
    pub fn new() -> Self {
        Self {
            hint_text: String::new(),
            flavor_text: String::new(),
            highlight: None,
            dismissed: false,
            cascade_active: false,
            pulse_time: 0.0,
            fade_alpha: 0.0,
        }
    }

    /// Update the overlay from the current run state. Call once per frame.
    pub fn update(
        &mut self,
        run: &RunState,
        dt: f32,
        _window_w: f32,
        _window_h: f32,
        _ui_scale: f32,
    ) {
        self.pulse_time += dt;

        // Fade in over 0.5s.
        self.fade_alpha = (self.fade_alpha + dt * 2.0).min(1.0);

        let tutorial = match &run.tutorial {
            Some(t) if t.is_active() => t,
            _ => {
                self.hint_text.clear();
                self.flavor_text.clear();
                self.highlight = None;
                return;
            }
        };

        let lesson = tutorial.current_lesson_def();
        self.flavor_text = lesson.flavor_text.to_string();

        // Derive the step_prompts index and highlight target from current
        // game state. Text is read from the lesson's `step_prompts` array
        // so the overlay never duplicates strings from the definitions.
        let has_selection = run.selected_count() > 0;
        let has_discards = run.discards_remaining > 0;
        let score_progress = run.round_score > 0;
        let prompts = lesson.step_prompts;

        // ── Compute step index + highlight per lesson ─────────────────
        let (step, highlight): (usize, Option<HighlightTarget>) = match tutorial.current_lesson {
            // Lesson 4 (discarding): three states with a dedicated highlight.
            4 => {
                if !score_progress && !has_selection {
                    (0, Some(HighlightTarget::DiscardBowl))
                } else if has_selection && has_discards {
                    (1, Some(HighlightTarget::DiscardBowl))
                } else {
                    (2, Some(HighlightTarget::PlayButton))
                }
            }
            // Lesson 5 (chips × mult): cascade-aware.
            5 => {
                if self.cascade_active {
                    (1, Some(HighlightTarget::ScorePanel))
                } else {
                    (0, Some(HighlightTarget::PlayButton))
                }
            }
            // All other lessons: generic [0]=no selection [1]=has selection
            // [2]=after-scoring contextual tip (if defined)
            // [3]=acknowledgment after opening the Meld Guide (if defined).
            _ => {
                if score_progress && !has_selection && prompts.len() > 2 {
                    let step = if tutorial.meld_guide_opened && prompts.len() > 3 {
                        3
                    } else {
                        2
                    };
                    (step, None)
                } else if has_selection {
                    (1, Some(HighlightTarget::PlayButton))
                } else {
                    (0, Some(HighlightTarget::HandTiles))
                }
            }
        };

        self.hint_text = prompts
            .get(step)
            .copied()
            .unwrap_or(lesson.intro_text)
            .to_string();
        self.highlight = highlight;

        // On the very first boss blind (lesson 1), override the flavor text
        // to introduce what a boss is. Later lessons don't repeat this.
        if run.blind == BlindKind::Boss && tutorial.current_lesson == 1 && !score_progress {
            self.flavor_text = "Boss Blind!".to_string();
            self.hint_text =
                "Bosses have higher targets and special rules. Beat this one to advance!"
                    .to_string();
        }
    }

    /// Reset state for a new lesson (called when lesson advances).
    #[allow(dead_code)]
    pub fn reset_for_lesson(&mut self) {
        self.dismissed = false;
        self.pulse_time = 0.0;
        self.fade_alpha = 0.0;
    }

    /// Push draw commands for the tutorial banner into the given vecs.
    /// `window_w` / `window_h` are the current window dimensions.
    pub fn draw(
        &self,
        window_w: f32,
        window_h: f32,
        ui_scale: f32,
        quads: &mut Vec<GpuInstance>,
        labels: &mut Vec<TextLabel>,
    ) {
        if self.hint_text.is_empty() || self.dismissed {
            return;
        }

        let alpha = self.fade_alpha;
        if alpha < 0.01 {
            return;
        }

        // ── Font sizes (pinned via font_px for readability) ──────────
        let flavor_px = typography::size(typography::HEADING, window_h, ui_scale);
        let hint_px = typography::size(typography::TITLE, window_h, ui_scale);
        let pad = (16.0 * ui_scale).max(10.0);

        // ── Word-wrap hint text to fit the banner width ──────────────
        let banner_w = window_w * 0.80;
        let text_w = banner_w - pad * 2.0;
        let max_chars = (text_w / (hint_px * 0.5)).max(10.0) as usize;
        let wrapped_hint = wrap_text(&self.hint_text, max_chars);
        let hint_lines = wrapped_hint.matches('\n').count() + 1;
        let hint_block_h = hint_lines as f32 * hint_px * 1.3;

        // ── Banner background ──────────────────────────────────────────
        let banner_h = pad + flavor_px + pad * 0.5 + hint_block_h + pad;
        let banner_y = window_h * 0.01;
        let banner_x = window_w * 0.10;

        // Subtle gold border (drawn behind the panel).
        let border = 2.0;
        quads.push(GpuInstance {
            rect: [
                banner_x - border,
                banner_y - border,
                banner_w + border * 2.0,
                banner_h + border * 2.0,
            ],
            color: [
                color::BRASS[0],
                color::BRASS[1],
                color::BRASS[2],
                0.4 * alpha,
            ],
        });

        // Semi-transparent dark panel.
        quads.push(GpuInstance {
            rect: [banner_x, banner_y, banner_w, banner_h],
            color: [
                color::MIDNIGHT[0],
                color::MIDNIGHT[1],
                color::MIDNIGHT[2],
                0.88 * alpha,
            ],
        });

        // ── Text ───────────────────────────────────────────────────────
        // Flavor text (smaller heading, gold).
        let flavor_y = banner_y + pad;
        labels.push(TextLabel {
            rect: [banner_x + pad, flavor_y, text_w, flavor_px * 1.5],
            text: self.flavor_text.clone(),
            color: [color::GOLD[0], color::GOLD[1], color::GOLD[2], 0.8 * alpha],
            font_px: Some(flavor_px),
            align: TextAlign::Center,
            ..Default::default()
        });

        // Hint text (title-sized, champagne, word-wrapped + centered).
        let hint_y = flavor_y + flavor_px + pad * 0.5;
        labels.push(TextLabel {
            rect: [banner_x + pad, hint_y, text_w, hint_block_h],
            text: wrapped_hint,
            color: [
                color::CHAMPAGNE[0],
                color::CHAMPAGNE[1],
                color::CHAMPAGNE[2],
                alpha,
            ],
            font_px: Some(hint_px),
            align: TextAlign::Center,
            ..Default::default()
        });
    }

    /// Whether the overlay is currently showing anything.
    #[allow(dead_code)]
    pub fn is_visible(&self) -> bool {
        !self.hint_text.is_empty() && !self.dismissed
    }
}

/// Greedy word-wrap by character budget. Returns lines joined with `\n`.
fn wrap_text(text: &str, max_chars: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            if word.chars().count() > max_chars {
                let mut buf = String::new();
                for ch in word.chars() {
                    buf.push(ch);
                    if buf.chars().count() == max_chars {
                        lines.push(std::mem::take(&mut buf));
                    }
                }
                current = buf;
            } else {
                current.push_str(word);
            }
        } else if current.chars().count() + 1 + word.chars().count() <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines.join("\n")
}
