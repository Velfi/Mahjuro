//! Walnut, Brass & Felt visual theme.
//!
//! Single source of truth for colors, typography scale, and standard metrics.
//! Every scene should pull from here rather than hard-coding `[r, g, b, a]`
//! literals — that way the whole game can be re-skinned by editing one file.
//!
//! See `COLOR_THEME.md` at the repo root for the design rationale, anti-patterns,
//! and the wider material story (felt cabinet linings, twilight outside, lacquer
//! framing, candle bloom). This file holds the *constants*; the doc holds the
//! *intent*.
//!
//! ## Palette philosophy
//!
//! Dark walnut panel tones with sparing brass accents. Brass is precious —
//! reserve it for headers, score numerals, selected-tile rims, currency, and
//! relic borders. Body text is `PARCHMENT`, never pure white. Highlights pull
//! toward `TALLOW` (candle bloom), not toward white. Think
//! "lacquered rosewood box with brass fittings under candlelight," not
//! flat UI gray.
//!
//! ## Material vs. semantic colors
//!
//! Material tokens (`WALNUT_*`, `FELT_*`, `TWILIGHT_*`, `LACQUER`, `CINNABAR`,
//! `TALLOW`) describe surfaces in the House. Semantic tokens (`JADE`, `RUBY`,
//! `AMBER`) describe UI signals (success / danger / warning). Don't cross them:
//! `JADE` is "this succeeded," not "this is a tabletop." `RUBY` is "this is
//! dangerous," not "this is celebratory." Materials are wood and felt and ink;
//! semantics are flags painted on top.
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
    /// `#5FD4A8` — success / target met / positive. **Semantic**, not the
    /// tabletop color (use `FELT` for that).
    pub const JADE: [f32; 4] = [0.373, 0.831, 0.659, 1.0];
    /// `#E85A6B` — danger / exit / negative. **Semantic**, not ceremonial
    /// red (use `CINNABAR` for that).
    pub const RUBY: [f32; 4] = [0.910, 0.353, 0.420, 1.0];
    /// `#F0A848` — warning / attention.
    pub const AMBER: [f32; 4] = [0.941, 0.659, 0.282, 1.0];

    // ── Felt: cabinet linings, scoring panels, mahjong tabletop. *Material*
    //    color — green felt as a surface, not a "success" signal. Use `JADE`
    //    when you mean "this succeeded."
    /// `#1A2A20` — in-shadow felt, e.g. cubby corners.
    pub const FELT_DEEP: [f32; 4] = [0.102, 0.165, 0.129, 1.0];
    /// `#2D4A38` — lit tabletop, archive cubby lining.
    pub const FELT: [f32; 4] = [0.176, 0.290, 0.220, 1.0];
    /// `#4A6B52` — felt directly under a candle/lamp.
    pub const FELT_LIT: [f32; 4] = [0.290, 0.420, 0.322, 1.0];

    // ── Twilight: the cool counterpoint. The world *outside* the House.
    //    Sky behind the main menu, shop wall plaster, info-modal backings,
    //    journal page tints — anything that reads as "looking out from
    //    inside." Reach for these before any saturated UI blue.
    /// `#0E1422` — distant night through a window.
    pub const TWILIGHT_INK: [f32; 4] = [0.055, 0.078, 0.133, 1.0];
    /// `#1E2A40` — shop walls, info modal background.
    pub const TWILIGHT: [f32; 4] = [0.118, 0.165, 0.251, 1.0];
    /// `#5A6E94` — distant lit window, hint text on cool surfaces.
    pub const TWILIGHT_GLOW: [f32; 4] = [0.353, 0.431, 0.580, 1.0];

    // ── Lacquer & Cinnabar: deep contrast and ceremony.
    /// `#0A0608` — true black-with-warmth. Counter tops, deepest shadow,
    /// frames around precious things. Distinct from `WALNUT_INK`: lacquer is
    /// *flat* black, walnut ink is *wood-grain* black.
    pub const LACQUER: [f32; 4] = [0.039, 0.024, 0.031, 1.0];
    /// `#B83228` — ceremonial red. Character-suit ink, dragon "Chun",
    /// celebration sigils, temple stamps. **Distinct from `RUBY`** —
    /// `RUBY` means "danger," `CINNABAR` means "ritual."
    pub const CINNABAR: [f32; 4] = [0.722, 0.196, 0.157, 1.0];

    // ── Tallow: candle bloom. The color of the light itself.
    /// `#FFE8B8` — particle cores, lantern halo, score-pop center,
    /// "this just happened" highlights. Highlights pull toward `TALLOW`,
    /// not toward white — light in this game is *made of fire*.
    pub const TALLOW: [f32; 4] = [1.000, 0.910, 0.722, 1.0];

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
    /// Couch-distance rule of thumb: at 1080p reference height, no UI tier
    /// should rasterize below this many CSS pixels. Scales down on shorter
    /// windows (`h/1080`); does not grow past this value on taller displays
    /// (base/tiers already scale up via [`base`]).
    pub const MIN_READABLE_PX_AT_1080: f32 = 24.0;

    /// Minimum font size (px) for on-screen copy at the current window height.
    /// Use with custom layouts that do not go through [`size`].
    #[inline]
    pub fn readable_floor_px(window_h: f32) -> f32 {
        MIN_READABLE_PX_AT_1080 * (window_h / 1080.0).min(1.0)
    }

    /// Base unit derived from window height. ~18px at 600px tall, ~30px at
    /// 1080p. Other tiers are ratios of this.
    pub fn base(window_h: f32) -> f32 {
        (window_h * 0.028).clamp(14.0, 36.0)
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
    pub fn size(tier: f32, window_h: f32) -> f32 {
        let px = tier * base(window_h);
        px.max(readable_floor_px(window_h))
    }
}

#[cfg(test)]
mod typography_tests {
    use super::typography;

    #[test]
    fn smallest_tier_meets_1080_floor() {
        assert!(
            typography::size(typography::MICRO, 1080.0) >= typography::MIN_READABLE_PX_AT_1080
        );
    }

    #[test]
    fn floor_scales_down_below_1080p() {
        let at_720 = typography::readable_floor_px(720.0);
        assert!((at_720 - 16.0).abs() < 0.01);
        assert!(typography::size(typography::MICRO, 720.0) >= at_720);
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

    /// Scene layout scale factor from the smaller window dimension.
    pub fn scene_scale(w: f32, h: f32) -> f32 {
        w.min(h) / 600.0
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
