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
//! relic borders. Body text is `PARCHMENT` (warm cream) for readability
//! on deep brown surfaces. Highlights pull toward white rather than cool grays.
//! Think
//! "lacquered rosewood box with brass fittings under candlelight," not
//! flat UI gray.
//!
//! ## Material vs. semantic colors
//!
//! Material tokens (`WALNUT_*`) describe surfaces in the House. Semantic
//! tokens (`JADE`, `RUBY`,
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

/// Named color tokens. Pull from here in scenes via `theme::color::GOLD` etc.
pub mod color {
    // ── Walnut ladder: dark → light, backgrounds, panels, modals, tooltips ─
    /// `#040302` — deepest base, near-black brown.
    pub const WALNUT_INK: [f32; 4] = [0.019, 0.012, 0.009, 1.0];
    /// `#0A0604` — primary modal/panel background and tooltip fill.
    pub const WALNUT_DEEP: [f32; 4] = [0.040, 0.024, 0.018, 1.0];
    /// `#120D09` — raised panel background (one step lighter than WALNUT_DEEP).
    pub const WALNUT_RAISED: [f32; 4] = [0.073, 0.052, 0.036, 1.0];
    /// `#1C140E` — hovered/selected panel background, button rest state.
    pub const WALNUT_SOFT: [f32; 4] = [0.111, 0.080, 0.058, 1.0];
    /// `#271C15` — strongest panel tone, primary button rest / highlights.
    pub const WALNUT_BRIGHT: [f32; 4] = [0.154, 0.112, 0.083, 1.0];

    // ── Golds: use sparingly, hierarchy of warmth ─────────────────────────
    /// `#DAC8A9` — palest gold, hero score numerals & selected-tile rims.
    pub const CHAMPAGNE: [f32; 4] = [0.855, 0.786, 0.666, 1.0];
    /// `#C8B38D` — primary gold accent, headers, currency.
    pub const GOLD: [f32; 4] = [0.785, 0.704, 0.553, 1.0];
    /// `#A89263` — darker gold for borders, inset frames.
    pub const BRASS: [f32; 4] = [0.663, 0.573, 0.392, 1.0];
    /// `#73603F` — deepest gold, almost bronze, used for shadow lines.
    pub const ANTIQUE: [f32; 4] = [0.454, 0.377, 0.248, 1.0];

    // ── Neutrals: text + dividers (warm stone — reads on walnut panels) ──
    /// `#F5F3F0` — body text. Warm cream on dark walnut (not pure white).
    pub const PARCHMENT: [f32; 4] = [0.960, 0.955, 0.940, 1.0];
    /// `#B6AEA4` — secondary text, captions, inactive labels.
    pub const STONE: [f32; 4] = [0.716, 0.683, 0.645, 1.0];
    /// `#625C53` — tertiary text, disabled state, dividers.
    pub const UMBER: [f32; 4] = [0.385, 0.361, 0.328, 1.0];

    // ── Semantic colors (desaturated to sit on warm wood panels) ──────────
    /// `#9CC0B3` — success / target met / positive. **Semantic**, not a
    /// general surface color.
    pub const JADE: [f32; 4] = [0.613, 0.755, 0.702, 1.0];
    /// `#9B6F74` — danger / exit / negative. **Semantic** signal red.
    pub const RUBY: [f32; 4] = [0.611, 0.438, 0.459, 1.0];
    /// `#C5AD8C` — warning / attention.
    pub const AMBER: [f32; 4] = [0.776, 0.680, 0.553, 1.0];

    // ── Lapis: the cool number-signal counterpart to RUBY.
    /// `#AEC0D2` — sky-blue tint for "Chips" score tokens, info chips,
    /// and any other UI signal that wants to read as the *cool* half of
    /// a warm/cool score pair. Paired with `RUBY` (Mult) at every score
    /// readout, and with `RELIC_GOLD` (Gold) / `PARCHMENT` (Final) when all
    /// four cascade kinds appear together. Distinct from the moody
    /// `TWILIGHT_*` family, which is *atmospheric* night-sky color, not a
    /// number-signal accent.
    pub const LAPIS: [f32; 4] = [0.686, 0.757, 0.825, 1.0];

    // ── Chart encodings: more chroma than UI semantics so bars, sparklines,
    //    and outcome strips read at a glance on walnut panels.
    pub mod chart {
        /// `#62B894` — wins, positive series, average reference lines.
        pub const POSITIVE: [f32; 4] = [0.384, 0.722, 0.580, 1.0];
        /// `#C75C66` — losses, negative series.
        pub const NEGATIVE: [f32; 4] = [0.780, 0.361, 0.400, 1.0];
        /// `#B8924A` — secondary chart accent (discovery stamp, warm fills).
        pub const ACCENT: [f32; 4] = [0.722, 0.573, 0.290, 1.0];
        /// `#B8A078` — neutral magnitude bars (score distribution segments).
        pub const FILL: [f32; 4] = [0.722, 0.627, 0.471, 1.0];
    }

    // ── Glossary keyword tints (`vocabulary_colors`, styled text). Full
    //    chroma so suit names and score jargon pop in tutorial copy; the
    //    muted `JADE` / `RUBY` / `LAPIS` ladder above is for chrome elsewhere.
    pub mod keyword {
        pub const MANZU: [f32; 4] = [0.85, 0.25, 0.20, 1.0];
        pub const SOUZU: [f32; 4] = [0.20, 0.65, 0.30, 1.0];
        pub const PINZU: [f32; 4] = [0.20, 0.40, 0.80, 1.0];
        pub const WIND: [f32; 4] = [0.70, 0.60, 0.20, 1.0];
        pub const DRAGON: [f32; 4] = [0.85, 0.20, 0.18, 1.0];
        pub const FLOWER: [f32; 4] = [0.90, 0.45, 0.55, 1.0];
        pub const SEASON: [f32; 4] = [0.30, 0.70, 0.65, 1.0];
        pub const HONORS: [f32; 4] = [0.961, 0.776, 0.455, 1.0];
        pub const CHIPS: [f32; 4] = [0.55, 0.78, 1.00, 1.0];
        pub const MULT: [f32; 4] = [0.910, 0.353, 0.420, 1.0];
        pub const GOLD: [f32; 4] = [0.94, 0.78, 0.28, 1.0];
        pub const PLAY: [f32; 4] = [0.373, 0.831, 0.659, 1.0];
        pub const TRIGGER: [f32; 4] = [0.784, 0.565, 0.118, 1.0];
    }

    // ── Porcelain: aged ceramic surfaces — temple-merchant pottery, the
    //    coin/consumable dishes on the gameplay table, the worn cream of
    //    a well-loved bowl. Distinct from `PARCHMENT` (paper) and
    //    `PARCHMENT` (text): porcelain is *fired clay with a tea-stain*,
    //    softer than parchment and noticeably less warm than tallow.
    /// `#DED6CB` — aged porcelain cream. Used for the relic dish, the
    /// consumable dish, and the "well-loved ceramic" base color.
    pub const PORCELAIN_AGED: [f32; 4] = [0.871, 0.841, 0.797, 1.0];

    // ── Relic metal tiers: rarity-keyed body materials for relics.
    //    Common → Iron, Uncommon → Copper, Rare → Silver, Legendary → Gold.
    //    Both the per-instance metal-shader base in `relic_material_params`
    //    and the `color::rarity(tier)` accent used by collection / shop /
    //    gameplay UI resolve to these tokens, so a relic's metal and its UI
    //    chip cannot drift apart. Distinct from the brass UI palette
    //    (`GOLD`, `BRASS`, `ANTIQUE`) which is for fixtures (headers,
    //    currency, button borders), not for material identity.
    /// `#6B7078` — Common-tier relic body. Cool steel gray.
    pub const RELIC_IRON: [f32; 4] = [0.423, 0.440, 0.472, 1.0];
    /// `#A27C64` — Uncommon-tier relic body. Warm copper.
    pub const RELIC_COPPER: [f32; 4] = [0.638, 0.488, 0.395, 1.0];
    /// `#D2D6DE` — Rare-tier relic body. Pale cool silver.
    pub const RELIC_SILVER: [f32; 4] = [0.824, 0.840, 0.871, 1.0];
    /// `#D7C692` — Legendary-tier relic body. Warm yellow gold; brighter
    /// than the UI `GOLD` token because legendaries earn the *light*, not
    /// just the fixture.
    pub const RELIC_GOLD: [f32; 4] = [0.843, 0.778, 0.575, 1.0];

    pub const fn alpha(c: [f32; 4], a: f32) -> [f32; 4] {
        [c[0], c[1], c[2], a]
    }

    /// Helper: drop the alpha channel for call sites that take `[f32; 3]`
    /// (point light tints, particle colors, etc.).
    pub const fn rgb(c: [f32; 4]) -> [f32; 3] {
        [c[0], c[1], c[2]]
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
    /// collection scenes don't drift apart, and so the UI chip color
    /// matches the relic body metal at that tier (see `RELIC_*` tokens).
    ///
    /// `tier` is 0..=3: common, uncommon, rare, legendary, mapped to the
    /// metal escalation iron → bronze → silver → gold.
    pub fn rarity(tier: u8) -> [f32; 4] {
        match tier {
            0 => RELIC_IRON,   // common
            1 => RELIC_COPPER, // uncommon
            2 => RELIC_SILVER, // rare
            _ => RELIC_GOLD,   // legendary
        }
    }
}

/// Typography scale. Every named tier is a *fraction of window height*; the
/// rasterized cap-height in pixels is `tier * window_h`, floored at the
/// readable minimum (24 px at 1080p, scaled linearly below that).
///
/// Constants are named `H<N>` where `N` is the divisor — `H20` rasterizes to
/// `window_h / 20`. Tiers span half the window height (`H2`) down to the
/// legibility floor (`H45`). Adjacent steps are roughly 1.2–1.5×, so you can
/// walk one tier up/down without layouts falling apart.
///
/// ```ignore
/// let title_px = theme::typography::size(theme::typography::H20, window_h);
/// ```
pub mod typography {
    /// Couch-distance rule of thumb: at 1080p reference height, no UI tier
    /// should rasterize below this many CSS pixels. Scales down on shorter
    /// windows (`h/1080`); does not grow past this value on taller displays.
    pub const MIN_READABLE_PX_AT_1080: f32 = 24.0;

    /// Minimum font size (px) for on-screen copy at the current window height.
    /// Use with custom layouts that do not go through [`size`].
    #[inline]
    pub fn readable_floor_px(window_h: f32) -> f32 {
        MIN_READABLE_PX_AT_1080 * (window_h / 1080.0).min(1.0)
    }

    // ── Scale ratios (window-height fractions) ──────────────────────────
    // Constant name encodes the divisor: `H_N` rasterizes to `window_h / N`.
    // 1080p column shows the resulting pixel size before the readable floor.

    /// `window_h / 2` — splash hero, end-of-act glyph. (540 px @ 1080p)
    pub const H2: f32 = 1.0 / 2.0;
    /// `window_h / 4` — celebration numerals, run-end totals. (270 px)
    pub const H4: f32 = 1.0 / 4.0;
    /// `window_h / 5` — victory/defeat headline. (216 px)
    pub const H5: f32 = 1.0 / 5.0;
    /// `window_h / 6` — oversized scene-transition text. (180 px)
    pub const H6: f32 = 1.0 / 6.0;
    /// `window_h / 12` — primary score display, big numerals. (90 px)
    pub const H12: f32 = 1.0 / 12.0;
    /// `window_h / 16` — large modal title. (~68 px)
    pub const H16: f32 = 1.0 / 16.0;
    /// `window_h / 20` — standard screen titles. (54 px)
    pub const H20: f32 = 1.0 / 20.0;
    /// `window_h / 24` — subtitles, secondary modal headers. (45 px)
    pub const H24: f32 = 1.0 / 24.0;
    /// `window_h / 28` — section heads, card names, button labels. (~39 px)
    pub const H28: f32 = 1.0 / 28.0;
    /// `window_h / 32` — large body, sub-headings. (~34 px)
    pub const H32: f32 = 1.0 / 32.0;
    /// `window_h / 36` — default body text. (30 px)
    pub const H36: f32 = 1.0 / 36.0;
    /// `window_h / 42` — captions, secondary info. (~26 px)
    pub const H42: f32 = 1.0 / 42.0;
    /// `window_h / 45` — smallest readable text (sits at the floor at 1080p). (24 px)
    pub const H45: f32 = 1.0 / 45.0;

    /// Compute the absolute pixel height for a tier at a given window height.
    /// Floors the result at the readable minimum so even `H45` stays legible.
    pub fn size(tier: f32, window_h: f32) -> f32 {
        (window_h * tier).max(readable_floor_px(window_h))
    }

    /// All tiers, largest cap-height → smallest.
    pub const LADDER: &[f32] = &[H2, H4, H5, H6, H12, H16, H20, H24, H28, H32, H36, H42, H45];

    /// Largest [`size`] on the ladder that still fits within `max_px`.
    pub fn tier_at_most(max_px: f32, window_h: f32) -> f32 {
        for &tier in LADDER {
            let px = size(tier, window_h);
            if px <= max_px + 0.01 {
                return px;
            }
        }
        size(H45, window_h)
    }
}

#[cfg(test)]
mod typography_tests {
    use super::typography;

    #[test]
    fn smallest_tier_meets_1080_floor() {
        assert!(typography::size(typography::H45, 1080.0) >= typography::MIN_READABLE_PX_AT_1080);
    }

    #[test]
    fn floor_scales_down_below_1080p() {
        let at_720 = typography::readable_floor_px(720.0);
        assert!((at_720 - 16.0).abs() < 0.01);
        assert!(typography::size(typography::H45, 720.0) >= at_720);
    }

    #[test]
    fn tier_at_most_picks_largest_fitting_step() {
        let h = 1080.0;
        assert!(
            (typography::tier_at_most(30.0, h) - typography::size(typography::H36, h)).abs() < 1.0
        );
        assert!(
            (typography::tier_at_most(200.0, h) - typography::size(typography::H6, h)).abs() < 1.0
        );
    }
}

/// Standard layout metrics — padding, borders, button heights. Helps every
/// scene look proportionally consistent.
pub mod metrics {
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
        ButtonState::Disabled => (
            darken(bg_rest, 0.35),
            darken(border_rest, 0.4),
            alpha(UMBER, 0.6),
        ),
    };

    ButtonColors { bg, border, text }
}
