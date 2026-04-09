//! Glossary / help overlay — shared between gameplay and shop scenes.
//!
//! Mahjuro picked up a lot of new vocabulary in Patches A–C: shanten, tenpai,
//! the round wind, the 12 yaku, dora, kongs, the Codex loadout, and Zodiac
//! cards. Mouse users get hover tooltips on the gameplay HUD, but keyboard
//! and gamepad users have no equivalent. This overlay is the cross-input
//! answer: a single screen, opened with `?` / `F1` / `H` (or the gamepad
//! Select button, or a `?` badge in the HUD), that explains every term in
//! one place.
//!
//! Closes on Escape, Cancel, Help (toggle off), or by clicking the
//! background. Scrollable: arrow keys / FocusUp/Down step through entries
//! one at a time so the larger text can stay readable on short windows.

use crate::core::yaku::YakuKind;
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::smooth_scroll::SmoothScroll;
use crate::ui::widget::{self, PanelVariant, TextStyle};

use super::ButtonDef;

/// Click id used by the glossary's "Close" button. Picked from a high-numbered
/// range so it can't collide with scene-specific click ids.
pub const GLOSSARY_CLOSE_ID: u32 = 0xF101;

/// Reusable glossary state. Owned by gameplay/shop scenes the same way they
/// own their PauseMenu.
pub struct GlossaryOverlay {
    pub open: bool,
    /// Smooth-scrolling state. Steps in entry units; the visual position
    /// interpolates smoothly toward the target each frame.
    scroll: SmoothScroll,
}

impl GlossaryOverlay {
    pub fn new() -> Self {
        Self {
            open: false,
            scroll: SmoothScroll::new(),
        }
    }

    /// Toggle the overlay. Called when the player triggers `UiAction::Help`
    /// or clicks the `?` badge / Close button.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        // Always re-open at the top — players expect a fresh read.
        if self.open {
            self.scroll.jump(0.0);
        }
    }

    /// Process inputs while the overlay is visible. Returns `true` if the
    /// caller should swallow the rest of the frame's input (i.e. the overlay
    /// was open and stayed open or was just closed by an input).
    pub fn handle_input(
        &mut self,
        actions: &[UiAction],
        button_clicks: &[u32],
        scroll_lines: f32,
    ) -> bool {
        if !self.open {
            return false;
        }
        // Scroll wheel: negative scroll_lines = up, positive = down.
        // Pass the raw float so trackpad momentum isn't rounded away.
        if scroll_lines.abs() > 0.001 {
            self.scroll.scroll_by(-scroll_lines);
        }
        for a in actions {
            match a {
                // Help toggles closed; Cancel/Pause also closes (the
                // canonical "back out" gestures).
                UiAction::Help | UiAction::Cancel | UiAction::Pause => {
                    self.open = false;
                    return true;
                }
                // Scroll: up/down steps one entry. FocusPrev/FocusNext (left/
                // right) also scroll so gamepad d-pad horizontal works.
                UiAction::FocusUp | UiAction::FocusPrev => {
                    self.scroll.step(-1);
                }
                UiAction::FocusDown | UiAction::FocusNext => {
                    self.scroll.step(1);
                }
                _ => {}
            }
        }
        for &cid in button_clicks {
            if cid == GLOSSARY_CLOSE_ID {
                self.open = false;
                return true;
            }
        }
        // Overlay still open — swallow everything else so gameplay doesn't
        // run while the player is reading.
        true
    }

    /// Draw the overlay over the current scene.
    pub fn draw(
        &self,
        window_w: f32,
        window_h: f32,
        instances: &mut Vec<GpuInstance>,
        text_labels: &mut Vec<TextLabel>,
        buttons: &mut Vec<ButtonDef>,
    ) {
        if !self.open {
            return;
        }

        // Clear everything the parent scene queued for this frame so its
        // text/quads/buttons don't bleed through the overlay. The renderer
        // draws all instances *then* all text labels in vec order, so without
        // this clear the scene's text labels would appear on top of our
        // panel even though the dim background quad covers the underlying
        // quads. Buttons get cleared too so hit-tests can't fall through to
        // gameplay controls.
        instances.clear();
        text_labels.clear();
        buttons.clear();

        // Solid dark backdrop — fully opaque so the cleared scene's tile
        // meshes (drawn in a separate pass) don't show through either.
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: color::MIDNIGHT,
        });

        let scale = (window_w.min(window_h)) / 600.0;
        let panel_w = (window_w * 0.92).min(1200.0 * scale);
        let panel_h = (window_h * 0.94).min(900.0 * scale);
        let panel_x = (window_w - panel_w) * 0.5;
        let panel_y = (window_h - panel_h) * 0.5;

        widget::push_panel(
            instances,
            [panel_x, panel_y, panel_w, panel_h],
            PanelVariant::Hero,
        );

        // Title — pinned font_px so it doesn't auto-shrink at narrow windows.
        // Includes the input hints inline so we don't burn a whole row on
        // them.
        let title_font = typography::size(typography::TITLE, window_h).max(24.0);
        let title_h = title_font * 1.5;
        let title_y = panel_y + (10.0 * scale);
        text_labels.push(TextLabel {
            rect: [panel_x, title_y, panel_w, title_h],
            text: "MAHJURO — Glossary".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(title_font),
            ..Default::default()
        });

        // Compact hint row pinned just below the title.
        let hint_font = typography::size(typography::CAPTION, window_h).max(13.0);
        let hint_h = hint_font * 1.4;
        let hint_y = title_y + title_h;
        text_labels.push(TextLabel {
            rect: [panel_x, hint_y, panel_w, hint_h],
            text: "↑/↓ scroll    ·    Esc / H / ? close".into(),
            color: color::PARCHMENT,
            align: TextAlign::Center,
            font_px: Some(hint_font),
            ..Default::default()
        });

        // ── Two-column body ────────────────────────────────────────────
        //
        // Left column: round-level mechanics (the things that come at the
        // player from the outside — round wind, dora, etc.).
        // Right column: yaku reference (the 12 hand patterns).
        let close_reserve = (52.0 * scale).max(40.0);
        let body_top = hint_y + hint_h + (10.0 * scale);
        let body_bot = panel_y + panel_h - close_reserve;
        let col_pad = panel_w * 0.035;
        let col_w = (panel_w - col_pad * 3.0) * 0.5;
        let left_x = panel_x + col_pad;
        let right_x = left_x + col_w + col_pad;

        let entries_left: &[(&str, &str)] = &[
            (
                "Shanten / Tenpai",
                "Shanten = number of tiles you'd need to swap to reach a complete 14-tile hand (4 melds + 1 pair). Tenpai = 1 swap away. Shown in the score header.",
            ),
            (
                "Tenpai Bonus",
                "The first complete hand you score each round earns a chip bonus that scales down the longer you wait (4× on play 1, 1× on play 4).",
            ),
            (
                "Round Wind",
                "Each ante has a wind: East → South → West → North. A triplet (or kong) of that wind fires the Yakuhai yaku for +3 mult and +40 chips.",
            ),
            (
                "Kong (Kan)",
                "Four of a kind. Counts as a triplet for yaku, scores 80 chips (vs 50 for a triplet), and synergizes with Kan Drum.",
            ),
            (
                "Dora",
                "An indicator tile flipped each round; the next-rank tile becomes 'dora' and grants +25 chips per copy in your scored hand.",
            ),
            (
                "Yaku Loadout",
                "You pick 3 yaku to feature each run. Loadout yaku score at full strength; others detect at 50% (Full Hand and Yakuhai are always full).",
            ),
            (
                "Zodiac Cards",
                "Consumable items, one per yaku. Using a Zodiac permanently raises that yaku's level for the run: +0.5 mult and +20 chips per level.",
            ),
        ];

        // Build the right-column entries (the 12 yaku) up front so we can
        // pass them through the same scrolling helper as the left column.
        let yaku_entries: Vec<(String, String)> = YakuKind::all()
            .iter()
            .map(|&yk| {
                (
                    yk.name().to_string(),
                    format!(
                        "+{} mult / +{} chips    {}",
                        yk.mult_bonus(),
                        yk.chip_bonus(),
                        yaku_shape_text(yk),
                    ),
                )
            })
            .collect();

        let entry_h = entry_height(window_h, scale);
        let entry_gap = 6.0 * scale;
        let row_step = entry_h + entry_gap;

        let heading_h = section_heading_h(window_h, scale);
        let entries_top = body_top + heading_h;
        let visible_h = (body_bot - entries_top).max(row_step);
        let visible_rows = ((visible_h / row_step).floor() as usize).max(1);

        let max_entries = entries_left.len().max(yaku_entries.len());
        let max_steps = max_entries.saturating_sub(visible_rows) as u32;
        // Persist for handle_input on the next frame, and clamp the current
        // scroll in case the window just got taller.
        self.scroll.set_max(max_steps);

        // Advance smooth scroll and derive the integer row offset plus a
        // fractional pixel shift for the in-between frames.
        let smooth = self.scroll.tick();
        let scroll = smooth.floor() as usize;
        let frac_offset = -(smooth.fract()) * row_step;

        // Section headings stay pinned at the top of the body — only the
        // entry rows scroll underneath them.
        push_section_heading(
            text_labels,
            "Round Mechanics",
            left_x,
            body_top,
            col_w,
            window_h,
            scale,
        );
        push_section_heading(
            text_labels,
            "Yaku (Hand Patterns)",
            right_x,
            body_top,
            col_w,
            window_h,
            scale,
        );

        // Render visible entries plus one extra row for the partial entry
        // sliding in/out during smooth scroll animation.
        let render_rows = visible_rows + 1;

        // Render left-column entries.
        for (i, (name, body)) in entries_left.iter().enumerate().skip(scroll) {
            let row = i - scroll;
            if row >= render_rows {
                break;
            }
            let y = entries_top + row as f32 * row_step + frac_offset;
            push_glossary_entry(text_labels, name, body, left_x, y, col_w, window_h, scale);
        }

        // Render right-column entries.
        for (i, (name, body)) in yaku_entries.iter().enumerate().skip(scroll) {
            let row = i - scroll;
            if row >= render_rows {
                break;
            }
            let y = entries_top + row as f32 * row_step + frac_offset;
            push_glossary_entry(text_labels, name, body, right_x, y, col_w, window_h, scale);
        }

        // Tiny scroll-position indicator on the panel's right edge — only
        // shown when there's something to scroll.
        if max_steps > 0 {
            let track_x = panel_x + panel_w - (10.0 * scale);
            let track_y = entries_top;
            let track_w = (3.0 * scale).max(2.0);
            let track_h = visible_rows as f32 * row_step;
            instances.push(GpuInstance {
                rect: [track_x, track_y, track_w, track_h],
                color: color::OBSIDIAN,
            });
            let thumb_h = (track_h * (visible_rows as f32 / max_entries as f32)).max(12.0 * scale);
            let thumb_y = track_y + (track_h - thumb_h) * (smooth / max_steps as f32);
            instances.push(GpuInstance {
                rect: [track_x, thumb_y, track_w, thumb_h],
                color: color::GOLD,
            });
        }

        // ── Close button ───────────────────────────────────────────────
        let btn_w = (200.0 * scale).max(120.0);
        let btn_h = (44.0 * scale).max(32.0);
        let btn_x = panel_x + (panel_w - btn_w) * 0.5;
        let btn_y = panel_y + panel_h - btn_h - (8.0 * scale);
        widget::push_panel(
            instances,
            [btn_x, btn_y, btn_w, btn_h],
            PanelVariant::Default,
        );
        let close_font = typography::size(typography::BODY, window_h).max(17.0);
        text_labels.push(TextLabel {
            rect: [btn_x, btn_y, btn_w, btn_h],
            text: "Close".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(close_font),
            ..Default::default()
        });
        buttons.push(ButtonDef::scene(
            (btn_x, btn_y, btn_w, btn_h),
            GLOSSARY_CLOSE_ID,
        ));
    }

    /// Canonical-frame variant of [`Self::draw`] for scenes that have
    /// migrated off the dual-vec `SceneDrawOutput` model.
    ///
    /// Pushes the glossary's quads then its text labels (and the close
    /// button) directly into the supplied [`UiFrame`]. Internally this
    /// reuses [`Self::draw`] by passing fresh empty vecs and then merging
    /// them into the frame in the canonical interleaved order. The
    /// glossary itself has no internal "tooltip-over-text" hazard — its
    /// only quad-then-text overlap is the close button (panel quad +
    /// label text), and pushing all panel quads before all text preserves
    /// that exact ordering.
    ///
    /// Caller is responsible for clearing or reusing `frame.cmds` before
    /// calling this; the glossary does not touch `frame.cmds` other than
    /// appending. Migrated scenes that want a "glossary fully covers the
    /// scene" effect should build a fresh `UiFrame`, optionally push a
    /// background, and then call this method.
    ///
    /// `frame.buttons` is extended with the glossary's own buttons (close
    /// button + any future glossary controls).
    pub fn draw_into_frame(&self, frame: &mut UiFrame, window_w: f32, window_h: f32) {
        if !self.open {
            return;
        }
        let mut quads: Vec<GpuInstance> = Vec::new();
        let mut text: Vec<TextLabel> = Vec::new();
        let mut btns: Vec<ButtonDef> = Vec::new();
        // The legacy draw() begins by clearing each vec — passing fresh
        // empties is a no-op for that step and lets us reuse all the
        // layout/scrolling logic without duplication.
        self.draw(window_w, window_h, &mut quads, &mut text, &mut btns);
        frame.quads(quads);
        frame.texts(text);
        frame.buttons.extend(btns);
    }
}

/// Short hand-shape description for each yaku — keeps the glossary entries
/// pithy. Long enough to teach, short enough to fit on one line at typical
/// window sizes.
pub(crate) fn yaku_shape_text(yk: YakuKind) -> &'static str {
    // Suit emoji match tile_suit_emoji: 🎴 Characters, 🎋 Bamboo, 🔴 Circles.
    // Honor emoji: 🐉 Dragon, 🌬 Wind.
    match yk {
        YakuKind::Tanyao => {
            "All tiles 2\u{2013}8, no honors/terminals (e.g. \u{1f3b4}234 \u{1f38b}567 \u{1f534}88)"
        }
        YakuKind::Toitoi => {
            "All triplets/kongs, no sequences (e.g. \u{1f3b4}222 \u{1f38b}555 \u{1f534}999)"
        }
        YakuKind::FullHand => "Complete 14-tile hand: 4 melds + 1 pair",
        YakuKind::Yakuhai => {
            "Triplet of any dragon or round wind (e.g. \u{1f409}\u{1f409}\u{1f409})"
        }
        YakuKind::Iipeikou => {
            "Two identical sequences in one suit (e.g. \u{1f38b}123 \u{1f38b}123)"
        }
        YakuKind::SanshokuDoujun => {
            "Same sequence in all 3 suits (e.g. \u{1f3b4}456 \u{1f38b}456 \u{1f534}456)"
        }
        YakuKind::Ittsu => {
            "1\u{2013}9 straight in one suit (e.g. \u{1f38b}123 \u{1f38b}456 \u{1f38b}789)"
        }
        YakuKind::Honitsu => {
            "One number suit + honors only (e.g. \u{1f38b}234 \u{1f38b}678 \u{1f32c}\u{1f32c}\u{1f32c})"
        }
        YakuKind::Chinitsu => {
            "All one number suit, no honors (e.g. \u{1f38b}123 \u{1f38b}456 \u{1f38b}789 \u{1f38b}11)"
        }
        YakuKind::Junchan => {
            "Every meld has a 1 or 9 (e.g. \u{1f38b}123 \u{1f3b4}789 \u{1f534}111 \u{1f38b}99)"
        }
        YakuKind::Honroutou => {
            "Only 1s, 9s, and honors (e.g. \u{1f38b}111 \u{1f3b4}999 \u{1f32c}\u{1f32c}\u{1f32c})"
        }
        YakuKind::Chiitoitsu => {
            "Seven distinct pairs (e.g. \u{1f3b4}11 \u{1f3b4}33 \u{1f38b}55 \u{1f38b}77 \u{1f534}22 \u{1f534}44 \u{1f32c}\u{1f32c})"
        }
    }
}

pub(crate) fn section_heading_h(window_h: f32, scale: f32) -> f32 {
    section_heading_font(window_h) * 1.7 + (6.0 * scale)
}

fn section_heading_font(window_h: f32) -> f32 {
    typography::size(typography::HEADING, window_h).max(20.0)
}

/// Total height (name row + body rows + bottom padding) of one glossary
/// entry. Must match `push_glossary_entry`'s layout exactly so the scroll
/// machinery in `draw` can compute row positions ahead of time.
pub(crate) fn entry_height(window_h: f32, scale: f32) -> f32 {
    let name_font = name_font(window_h);
    let name_h = (name_font * 1.5).max(22.0);
    let body_font = body_font(window_h);
    let line_step = body_font * 1.35;
    let body_h = line_step * 3.0;
    name_h + body_h + (4.0 * scale)
}

fn name_font(window_h: f32) -> f32 {
    typography::size(typography::BODY, window_h).max(17.0)
}

fn body_font(window_h: f32) -> f32 {
    // Matches the BODY tier we hand to push_text_block. The min keeps very
    // small windows from crushing the text below ~15px.
    typography::size(typography::BODY, window_h).max(15.0)
}

pub(crate) fn push_section_heading(
    labels: &mut Vec<TextLabel>,
    text: &str,
    x: f32,
    y: f32,
    w: f32,
    window_h: f32,
    _scale: f32,
) {
    let font = section_heading_font(window_h);
    let h = font * 1.7;
    labels.push(TextLabel {
        rect: [x, y, w, h],
        text: text.into(),
        color: color::GOLD,
        font_px: Some(font),
        ..Default::default()
    });
}

/// Render one glossary entry (gold name on top, parchment body below) at
/// the given top-left. Layout must match `entry_height` so the scroll
/// stepping in `GlossaryOverlay::draw` aligns rows perfectly.
pub(crate) fn push_glossary_entry(
    labels: &mut Vec<TextLabel>,
    name: &str,
    body: &str,
    x: f32,
    y: f32,
    w: f32,
    window_h: f32,
    _scale: f32,
) {
    let nf = name_font(window_h);
    let name_h = (nf * 1.5).max(22.0);
    labels.push(TextLabel {
        rect: [x, y, w, name_h],
        text: name.into(),
        color: color::CHAMPAGNE,
        font_px: Some(nf),
        ..Default::default()
    });

    // Body: word-wrapped multi-line block at pinned CAPTION size. Reserve
    // enough vertical space for ~3 lines of body — most entries fit in two,
    // longest in three.
    let bf = body_font(window_h);
    let line_step = bf * 1.4;
    let body_h = line_step * 3.0;
    // BODY tier (not CAPTION) so descriptions render at the same size as
    // the entry name — this is the readability bump the glossary needed.
    widget::push_text_block(
        labels,
        [x, y + name_h, w, body_h],
        body,
        TextStyle {
            tier: typography::BODY,
            color: color::PARCHMENT,
            padding: 0.0,
            align: TextAlign::Left,
        },
        window_h,
    );
}
