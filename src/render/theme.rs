//! Midnight Gold visual theme.
//!
//! Single source of truth for colors, typography scale, and standard metrics.
//! Every scene should pull from here rather than hard-coding `[r, g, b, a]`
//! literals — that way the whole game can be re-skinned by editing one file.
//!
//! ## Palette philosophy
//!
//! Dark walnut panel tones with sparing gold accents. Gold is precious —
//! reserve it for headers, score numerals, selected-tile rims, currency, and
//! relic borders. Body text is `PARCHMENT`, never pure white. Think
//! "lacquered rosewood box with brass fittings under candlelight," not
//! flat UI gray.
//!
//! ## Conversions
//!
//! All colors are stored as `[f32; 4]` in linear-ish 0..1 space matching the
//! existing `GpuInstance.color` and `TextLabel.color` formats. Hex values in
//! the doc comments are the source-of-truth design tokens.

#![allow(dead_code)]

/// Named color tokens. Pull from here in scenes via `theme::color::GOLD` etc.
pub mod color {
    // ── Walnut ladder: dark → light, backgrounds, panels, modals, tooltips ─
    /// `#0A0806` — deepest base, near-black brown.
    pub const WALNUT_INK: [f32; 4] = [0.040, 0.031, 0.024, 1.0];
    /// `#120E0B` — primary modal/panel background and tooltip fill.
    pub const WALNUT_DEEP: [f32; 4] = [0.071, 0.055, 0.043, 1.0];
    /// `#1C1611` — raised panel background (one step lighter than WALNUT_DEEP).
    pub const WALNUT_RAISED: [f32; 4] = [0.110, 0.086, 0.067, 1.0];
    /// `#2A211A` — hovered/selected panel background, button rest state.
    pub const WALNUT_SOFT: [f32; 4] = [0.165, 0.129, 0.102, 1.0];
    /// `#362A21` — strongest panel tone, primary button rest / highlights.
    pub const WALNUT_BRIGHT: [f32; 4] = [0.212, 0.165, 0.129, 1.0];

    // ── Golds: use sparingly, hierarchy of warmth ─────────────────────────
    /// `#F5C674` — palest gold, hero score numerals & selected-tile rims.
    pub const CHAMPAGNE: [f32; 4] = [0.961, 0.776, 0.455, 1.0];
    /// `#E8B14A` — primary gold accent, headers, currency.
    pub const GOLD: [f32; 4] = [0.910, 0.694, 0.290, 1.0];
    /// `#C8901E` — darker gold for borders, inset frames.
    pub const BRASS: [f32; 4] = [0.784, 0.565, 0.118, 1.0];
    /// `#8A5E14` — deepest gold, almost bronze, used for shadow lines.
    pub const ANTIQUE: [f32; 4] = [0.541, 0.369, 0.078, 1.0];

    // ── Neutrals: text + dividers (warm stone — reads on walnut panels) ──
    /// `#F4F1E8` — body text. Warm off-white. NEVER use pure white.
    pub const PARCHMENT: [f32; 4] = [0.957, 0.945, 0.910, 1.0];
    /// `#B8AEA2` — secondary text, captions, inactive labels.
    pub const STONE: [f32; 4] = [0.722, 0.682, 0.635, 1.0];
    /// `#635C52` — tertiary text, disabled state, dividers.
    pub const UMBER: [f32; 4] = [0.388, 0.361, 0.322, 1.0];

    // ── Semantic colors (desaturated to sit on warm wood panels) ──────────
    /// `#5FD4A8` — success / target met / positive.
    pub const JADE: [f32; 4] = [0.373, 0.831, 0.659, 1.0];
    /// `#E85A6B` — danger / exit / negative.
    pub const RUBY: [f32; 4] = [0.910, 0.353, 0.420, 1.0];
    /// `#F0A848` — warning / attention.
    pub const AMBER: [f32; 4] = [0.941, 0.659, 0.282, 1.0];

    /// Fully transparent — used for spacers and invisible hit targets.
    pub const CLEAR: [f32; 4] = [0.0, 0.0, 0.0, 0.0];

    /// Helper: same color but with the alpha channel replaced.
    pub const fn alpha(c: [f32; 4], a: f32) -> [f32; 4] {
        [c[0], c[1], c[2], a]
    }

    /// Helper: dim a color toward black by `t` (0 = unchanged, 1 = black).
    pub fn darken(c: [f32; 4], t: f32) -> [f32; 4] {
        let k = 1.0 - t.clamp(0.0, 1.0);
        [c[0] * k, c[1] * k, c[2] * k, c[3]]
    }

    /// Helper: brighten a color toward white by `t` (0 = unchanged, 1 = white).
    pub fn lighten(c: [f32; 4], t: f32) -> [f32; 4] {
        let k = t.clamp(0.0, 1.0);
        [
            c[0] + (1.0 - c[0]) * k,
            c[1] + (1.0 - c[1]) * k,
            c[2] + (1.0 - c[2]) * k,
            c[3],
        ]
    }

    /// Rarity color for relic/yaku/blind cards. Centralized so the shop and
    /// collection scenes don't drift apart.
    ///
    /// `tier` is 0..=3: common, uncommon, rare, legendary.
    pub fn rarity(tier: u8) -> [f32; 4] {
        match tier {
            0 => STONE,         // common — neutral
            1 => JADE,          // uncommon — green
            2 => WALNUT_BRIGHT, // rare — lighter walnut highlight
            _ => CHAMPAGNE,     // legendary — gold
        }
    }
}

/// Typography scale. All sizes are computed against a base derived from the
/// window's smaller dimension so the UI stays readable at any resolution.
///
/// Usage: `let scale = theme::typography::scale(window_h);` then multiply by
/// the named tier ratio:
///
/// ```ignore
/// let title_h = theme::typography::TITLE * scale;
/// ```
pub mod typography {
    /// Base unit derived from window height. ~18px at 600px tall, ~30px at
    /// 1080p. Other tiers are ratios of this. The `ui_scale` multiplier
    /// (from the player's Visual settings) boosts the result for TV / couch
    /// viewing — it also raises the upper clamp so 4K screens benefit.
    pub fn base(window_h: f32, ui_scale: f32) -> f32 {
        (window_h * 0.028).clamp(14.0, 36.0 * ui_scale) * ui_scale
    }

    /// Hero numerals: score panel display number. ~3x base.
    pub const DISPLAY: f32 = 3.0;
    /// Screen titles, modal headers. ~1.75x base.
    pub const TITLE: f32 = 1.75;
    /// Section headings, card names, button labels. ~1.25x base.
    pub const HEADING: f32 = 1.25;
    /// Default body text. 1.0x base.
    pub const BODY: f32 = 1.0;
    /// Captions, secondary info, tooltip subtext. ~0.85x base.
    pub const CAPTION: f32 = 0.85;
    /// Smallest readable text — version strings, debug labels. ~0.7x base.
    pub const MICRO: f32 = 0.7;

    /// Compute the absolute pixel height for a tier at a given window height.
    pub fn size(tier: f32, window_h: f32, ui_scale: f32) -> f32 {
        tier * base(window_h, ui_scale)
    }
}

/// Standard layout metrics — padding, borders, button heights. Helps every
/// scene look proportionally consistent.
pub mod metrics {
    /// Standard padding inside a panel, in window-h units. Multiply by
    /// `window_h` to get pixels: e.g. `PANEL_PADDING * window_h`.
    pub const PANEL_PADDING: f32 = 0.018;
    /// Inner padding inside a text rect — keeps text from kissing edges.
    pub const TEXT_PADDING: f32 = 0.012;
    /// Standard button height as a ratio of window height.
    pub const BUTTON_HEIGHT: f32 = 0.064;
    /// Standard button corner inset (mock rounded look using inset border quads).
    pub const BORDER_INSET: f32 = 0.0025;
    /// Standard gap between stacked menu buttons.
    pub const BUTTON_GAP: f32 = 0.022;

    /// Scene layout scale factor incorporating the user's UI scale preference.
    /// Replaces the common `(w.min(h) / 600.0)` pattern.
    pub fn scene_scale(w: f32, h: f32, ui_scale: f32) -> f32 {
        (w.min(h) / 600.0) * ui_scale
    }
}

/// Visual variant for a button — drives which color set it draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    /// Default neutral walnut-toned button.
    Default,
    /// Affirmative action: continue, resume, confirm.
    Primary,
    /// Destructive action: exit, delete, abandon run.
    Danger,
    /// Subtle/secondary action: back, cancel.
    Subtle,
}

/// Interaction state of a button this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Rest,
    Hover,
    Press,
    Disabled,
}

/// Resolved color triple for a button in a particular variant + state.
#[derive(Clone, Copy, Debug)]
pub struct ButtonColors {
    pub bg: [f32; 4],
    pub border: [f32; 4],
    pub text: [f32; 4],
}

/// Look up the canonical button colors for a (variant, state) pair. Every
/// button in the game should funnel through this so the visual language stays
/// consistent.
pub fn button_colors(variant: ButtonVariant, state: ButtonState) -> ButtonColors {
    use color::*;
    // Base background per variant at rest. Hover lightens, press darkens.
    let (bg_rest, border_rest, text_rest) = match variant {
        ButtonVariant::Default => (WALNUT_SOFT, BRASS, PARCHMENT),
        ButtonVariant::Primary => (WALNUT_BRIGHT, GOLD, CHAMPAGNE),
        ButtonVariant::Danger => (WALNUT_RAISED, RUBY, alpha(RUBY, 1.0)),
        ButtonVariant::Subtle => (WALNUT_DEEP, UMBER, STONE),
    };

    let (bg, border, text) = match state {
        ButtonState::Rest => (bg_rest, border_rest, text_rest),
        ButtonState::Hover => (lighten(bg_rest, 0.15), GOLD, CHAMPAGNE),
        ButtonState::Press => (darken(bg_rest, 0.18), BRASS, text_rest),
        ButtonState::Disabled => (
            darken(bg_rest, 0.35),
            darken(border_rest, 0.4),
            alpha(UMBER, 0.6),
        ),
    };

    ButtonColors { bg, border, text }
}
