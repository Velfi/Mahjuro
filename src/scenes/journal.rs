//! Yaku Journal overlay — Balatro-style "run stats" page for hands.
//!
//! Lists every yaku in the run with its current level, the leveled
//! mult/chip bonuses, the number of times the player has scored it this
//! run, and a one-line construction hint. Replaces the in-play tablet row
//! as the primary place to learn yaku, so the play area can stay focused
//! on the *firing* yaku for the current selection.
//!
//! Structurally a near-twin of [`super::glossary::GlossaryOverlay`]: same
//! open/close gestures, same scroll model, same layout helpers. The only
//! interesting differences are (a) the per-entry body is built from live
//! `RunState` data, and (b) `draw_into_frame` takes a `&RunState`.
//!
//! Opened by clicking a 3D book on the gameplay table or shop counter.
//! Closes on Esc, Cancel, Help, or the in-overlay Close button.

use std::cell::Cell;

use crate::core::yaku::YakuKind;
use crate::game::run::RunState;
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::input::UiAction;

use super::ButtonDef;
use super::glossary::yaku_shape_text;

/// Click id used by the journal's "Close" button. Picked from the same
/// high-numbered range glossary uses so it can't collide with scene
/// click ids.
pub const JOURNAL_CLOSE_ID: u32 = 0xF201;

/// Reusable journal state. Owned by gameplay/shop scenes the same way
/// they own their `GlossaryOverlay`.
pub struct JournalOverlay {
    pub open: bool,
    /// Number of whole entries scrolled past — same scroll model as the
    /// glossary so partial-entry rendering can't happen (no scissor in
    /// the renderer). `Cell` because `draw` is `&self`.
    scroll_steps: Cell<u32>,
    /// Updated each `draw` so `handle_input` knows the legal scroll
    /// range when the window resizes.
    max_scroll_steps: Cell<u32>,
}

impl JournalOverlay {
    pub fn new() -> Self {
        Self {
            open: false,
            scroll_steps: Cell::new(0),
            max_scroll_steps: Cell::new(0),
        }
    }

    /// Toggle the overlay. Always re-opens at the top so the player gets
    /// a fresh read.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.scroll_steps.set(0);
        }
    }

    /// Process inputs while the overlay is visible. Returns `true` if the
    /// caller should swallow the rest of the frame's input.
    pub fn handle_input(&mut self, actions: &[UiAction], button_clicks: &[u32]) -> bool {
        if !self.open {
            return false;
        }
        for a in actions {
            match a {
                UiAction::Help | UiAction::Cancel | UiAction::Pause => {
                    self.open = false;
                    return true;
                }
                UiAction::FocusUp | UiAction::FocusPrev => {
                    self.scroll_steps
                        .set(self.scroll_steps.get().saturating_sub(1));
                }
                UiAction::FocusDown | UiAction::FocusNext => {
                    let cur = self.scroll_steps.get();
                    if cur < self.max_scroll_steps.get() {
                        self.scroll_steps.set(cur + 1);
                    }
                }
                _ => {}
            }
        }
        for &cid in button_clicks {
            if cid == JOURNAL_CLOSE_ID {
                self.open = false;
                return true;
            }
        }
        true
    }

    /// Draw the overlay as an open ledger spread — two parchment pages
    /// joined at a center gutter, brass binding around the edge, dark
    /// serif text on cream pages. Distinct from the glossary's dark
    /// hero-panel look so the player reads them as different artifacts:
    /// the glossary is a quick-reference cheat sheet, the journal is
    /// the run's actual record book.
    ///
    /// Layout (per page):
    ///   ┌─ brass border ───────────────────────────┐
    ///   │  ╭─ parchment page ────────────────────╮ │
    ///   │  │  ┌─ chapter heading ──┐             │ │
    ///   │  │  │  Yaku I–VI         │             │ │
    ///   │  │  └────────────────────┘             │ │
    ///   │  │  Tanyao              Lv 2  +2.5 m   │ │
    ///   │  │  ─────────────────── +50 c · 3×     │ │
    ///   │  │  All tiles 2–8 …                    │ │
    ///   │  │  …                                  │ │
    ///   │  ╰─────────────────────────────────────╯ │
    ///   └──────────────────────────────────────────┘
    pub fn draw(
        &self,
        window_w: f32,
        window_h: f32,
        run: &RunState,
        instances: &mut Vec<GpuInstance>,
        text_labels: &mut Vec<TextLabel>,
        buttons: &mut Vec<ButtonDef>,
    ) {
        if !self.open {
            return;
        }

        instances.clear();
        text_labels.clear();
        buttons.clear();

        // Backdrop — opaque so the cleared scene's 3D meshes don't bleed
        // through. Slightly darker than glossary's MIDNIGHT to read as
        // "dim study" rather than "modal panel."
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: color::OBSIDIAN,
        });

        let scale = (window_w.min(window_h)) / 600.0;
        // Fill almost the entire window — the journal is meant to be
        // *the* thing the player is looking at, and the extra space
        // lets every yaku entry breathe at a readable font size.
        let book_w = window_w * 0.97;
        let book_h = window_h * 0.97;
        let book_x = (window_w - book_w) * 0.5;
        let book_y = (window_h - book_h) * 0.5;

        // ── Book stack: drop shadow + offset cream layers behind the
        //    pages so the book reads as a thick volume sitting on a
        //    surface, not a flat panel. ─────────────────────────────────
        let shadow_off = 6.0 * scale;
        instances.push(GpuInstance {
            rect: [
                book_x + shadow_off,
                book_y + shadow_off * 1.6,
                book_w,
                book_h,
            ],
            color: color::alpha([0.0, 0.0, 0.0, 1.0], 0.55),
        });
        // Stack of three faintly offset page-edge layers, darker than
        // the parchment top. These suggest the thickness of all the
        // pages underneath the open spread.
        for i in 0..3 {
            let off = (3 - i) as f32 * (2.0 * scale);
            let cream = color::darken(color::PARCHMENT, 0.18 - i as f32 * 0.04);
            instances.push(GpuInstance {
                rect: [book_x - off, book_y - off, book_w + off * 2.0, book_h + off * 2.0],
                color: cream,
            });
        }

        // ── Brass cover border ─────────────────────────────────────────
        let border = (10.0 * scale).max(8.0);
        instances.push(GpuInstance {
            rect: [book_x, book_y, book_w, book_h],
            color: color::ANTIQUE,
        });
        // Inner brass highlight rim — one shade lighter than the outer
        // border for a beveled look.
        let bevel = 2.0 * scale;
        instances.push(GpuInstance {
            rect: [
                book_x + bevel,
                book_y + bevel,
                book_w - bevel * 2.0,
                book_h - bevel * 2.0,
            ],
            color: color::BRASS,
        });

        // Page area (inside the brass border). The two pages share this
        // rect with a small gutter dividing them.
        let pages_x = book_x + border;
        let pages_y = book_y + border;
        let pages_w = book_w - border * 2.0;
        let pages_h = book_h - border * 2.0;

        // Parchment fill for the full pages area.
        instances.push(GpuInstance {
            rect: [pages_x, pages_y, pages_w, pages_h],
            color: color::PARCHMENT,
        });

        // ── Center gutter shadow ───────────────────────────────────────
        // A vertical strip of progressively darker quads down the spine
        // simulates the inner curl where the two pages meet. Drawn as
        // five thin slabs with falloff so the renderer (no gradients)
        // still gets a soft seam.
        let gutter_w = (16.0 * scale).max(12.0);
        let gutter_x = pages_x + (pages_w - gutter_w) * 0.5;
        for i in 0..5 {
            let t = (i as f32 - 2.0).abs() / 2.0; // 1 at edges, 0 in center
            let slab_w = gutter_w / 5.0;
            let alpha = 0.55 * (1.0 - t * 0.7);
            instances.push(GpuInstance {
                rect: [gutter_x + i as f32 * slab_w, pages_y, slab_w, pages_h],
                color: color::alpha([0.0, 0.0, 0.0, 1.0], alpha),
            });
        }

        // Per-page rects.
        let page_pad_x = (18.0 * scale).max(14.0);
        let page_pad_y = (18.0 * scale).max(14.0);
        let half_w = (pages_w - gutter_w) * 0.5;
        let left_page_x = pages_x + page_pad_x;
        let right_page_x = pages_x + half_w + gutter_w + page_pad_x;
        let page_inner_w = half_w - page_pad_x * 2.0;
        let page_top = pages_y + page_pad_y;
        let page_bot = pages_y + pages_h - page_pad_y;

        // ── Title + control hint at the head of the LEFT page ─────────
        // Pinned floors are intentionally large — accessibility-first,
        // not auto-shrinking — so the journal is readable at any window
        // size without leaning on the user's display zoom. The title
        // sits left-aligned on the left page only; the right page's
        // top edge is left clear so its first entry sits high on the
        // page (the two columns intentionally don't align at the top).
        let title_font = typography::size(typography::TITLE, window_h).max(34.0) * 1.25;
        let title_h = title_font * 1.4;
        let title_y = page_top;
        text_labels.push(TextLabel {
            rect: [left_page_x, title_y, page_inner_w, title_h],
            text: "Yaku Journal".into(),
            color: color::OBSIDIAN,
            align: TextAlign::Left,
            font_px: Some(title_font),
            ..Default::default()
        });
        let hint_font = typography::size(typography::BODY, window_h).max(16.0) * 1.25;
        let hint_h = hint_font * 1.4;
        let hint_y = title_y + title_h;
        text_labels.push(TextLabel {
            rect: [left_page_x, hint_y, page_inner_w, hint_h],
            text: "Esc / H / ? to close".into(),
            color: color::darken(color::ANTIQUE, 0.3),
            align: TextAlign::Left,
            font_px: Some(hint_font),
            ..Default::default()
        });

        // Build all 12 yaku entries up front from live RunState. Each
        // entry is `(name, stats, shape)`; the stats string sits on the
        // same row as the name (right-aligned) so each entry only
        // consumes two visual lines, letting all 6 yaku fit on each
        // page without scrolling.
        let entries: Vec<(String, String, &'static str)> = YakuKind::all()
            .iter()
            .map(|&yk| {
                let lvl = run.yaku_levels.level_of(yk);
                let mult = yk.mult_bonus_at(lvl);
                let chip = yk.chip_bonus_at(lvl);
                let plays = run.yaku_times_played.get(&yk).copied().unwrap_or(0);
                let stats = format!(
                    "Lv {}   +{} m / +{} c   {}×",
                    lvl, mult, chip, plays,
                );
                (yk.name().to_string(), stats, yaku_shape_text(yk))
            })
            .collect();
        let mid = entries.len().div_ceil(2);
        let (left_entries, right_entries) = entries.split_at(mid);

        let close_reserve = (88.0 * scale).max(72.0);
        let body_top = hint_y + hint_h + (14.0 * scale);
        let body_bot = page_bot - close_reserve;
        let chapter_font = typography::size(typography::HEADING, window_h).max(26.0) * 1.25;
        let chapter_h = chapter_font * 1.5;
        let entries_top = body_top + chapter_h + (12.0 * scale);

        // No scrolling: every entry must fit in `body_bot - entries_top`.
        // Compute the row step from available space divided by row count
        // so the entries grow to fill the page no matter the window size.
        // This is what makes the journal use the full screen real estate
        // instead of leaving the bottom half of each page blank.
        let rows_per_page = mid; // 6 yaku per page
        let avail_h = (body_bot - entries_top).max(120.0);
        let row_step = avail_h / rows_per_page as f32;
        // Bigger inner pad than before — split between an upper margin
        // *inside* each row (so entries 2..6 don't slam into the previous
        // row's rule line) and a small bottom gap. Without this, entry 1
        // gets the chapter heading's padding for free but every other
        // entry sits flush against the rule above it.
        let row_inner_pad = (24.0 * scale).max(18.0);
        let entry_top_pad = row_inner_pad * 0.55;
        let entry_h = (row_step - row_inner_pad).max(40.0);

        // Floors are still pinned for accessibility, but the row sizes
        // also let the fonts breathe upward when the window is large —
        // we cap each font at a fraction of the row height so a 4K
        // window doesn't end up with 60-px text.
        let name_font = (typography::size(typography::HEADING, window_h)
            .max(22.0)
            .min(entry_h * 0.42))
            * 1.25;
        let body_font = (typography::size(typography::BODY, window_h)
            .max(18.0)
            .min(entry_h * 0.34))
            * 1.25;
        let stats_font = name_font * 0.78;
        // Stop the unused-scroll machinery from claiming any input.
        self.max_scroll_steps.set(0);
        self.scroll_steps.set(0);

        // Chapter headings — Roman numerals to play up the ledger feel.
        text_labels.push(TextLabel {
            rect: [left_page_x, body_top, page_inner_w, chapter_h],
            text: "Yaku   I – VI".into(),
            color: color::OBSIDIAN,
            align: TextAlign::Center,
            font_px: Some(chapter_font),
            ..Default::default()
        });
        text_labels.push(TextLabel {
            rect: [right_page_x, body_top, page_inner_w, chapter_h],
            text: "Yaku   VII – XII".into(),
            color: color::OBSIDIAN,
            align: TextAlign::Center,
            font_px: Some(chapter_font),
            ..Default::default()
        });
        // Hairline rule under each chapter heading.
        let rule_h = (1.5 * scale).max(1.0);
        instances.push(GpuInstance {
            rect: [
                left_page_x,
                body_top + chapter_h + (2.0 * scale),
                page_inner_w,
                rule_h,
            ],
            color: color::ANTIQUE,
        });
        instances.push(GpuInstance {
            rect: [
                right_page_x,
                body_top + chapter_h + (2.0 * scale),
                page_inner_w,
                rule_h,
            ],
            color: color::ANTIQUE,
        });

        let push_entry = |text_labels: &mut Vec<TextLabel>,
                          instances: &mut Vec<GpuInstance>,
                          name: &str,
                          stats: &str,
                          shape: &str,
                          x: f32,
                          y: f32,
                          w: f32| {
            // Two visual rows per entry. Row 1 is the name on the left
            // and the stats string on the right at the same baseline,
            // so the eye can sweep across "Tanyao …………… Lv 1 +2 m / +30 c
            // 0×". Row 2 is the construction hint underneath. The whole
            // entry is shifted down by `entry_top_pad` inside its row so
            // there's visible breathing room between the previous row's
            // rule line and this entry's name.
            let y = y + entry_top_pad;
            let name_h = name_font * 1.45;
            text_labels.push(TextLabel {
                rect: [x, y, w, name_h],
                text: name.into(),
                color: color::OBSIDIAN,
                align: TextAlign::Left,
                font_px: Some(name_font),
                ..Default::default()
            });
            // Stats — same row as the name, right-aligned. Smaller than
            // the name so the name stays the dominant element.
            text_labels.push(TextLabel {
                rect: [x, y, w, name_h],
                text: stats.into(),
                color: color::OBSIDIAN,
                align: TextAlign::Right,
                font_px: Some(stats_font),
                ..Default::default()
            });
            let shape_h = body_font * 1.5;
            text_labels.push(TextLabel {
                rect: [x, y + name_h, w, shape_h],
                text: shape.into(),
                color: color::OBSIDIAN,
                align: TextAlign::Left,
                font_px: Some(body_font),
                ..Default::default()
            });
            // Hairline rule along the bottom of the row, like ruled
            // ledger paper.
            instances.push(GpuInstance {
                rect: [
                    x,
                    y + name_h + shape_h + (4.0 * scale),
                    w,
                    (1.0 * scale).max(1.0),
                ],
                color: color::alpha(color::ANTIQUE, 0.45),
            });
        };

        for (i, (name, stats, shape)) in left_entries.iter().enumerate() {
            let y = entries_top + i as f32 * row_step;
            push_entry(
                text_labels, instances, name, stats, shape, left_page_x, y, page_inner_w,
            );
        }
        for (i, (name, stats, shape)) in right_entries.iter().enumerate() {
            let y = entries_top + i as f32 * row_step;
            push_entry(
                text_labels, instances, name, stats, shape, right_page_x, y, page_inner_w,
            );
        }

        // ── Close "wax seal" button at the bottom-center of the spread.
        // Brass ring + parchment-tinted disc + label, sitting in the
        // close_reserve gap below the entries.
        let seal_d = (72.0 * scale).max(56.0);
        let seal_x = book_x + (book_w - seal_d) * 0.5;
        let seal_y = book_y + book_h - seal_d - (6.0 * scale);
        instances.push(GpuInstance {
            rect: [seal_x - (2.0 * scale), seal_y - (2.0 * scale), seal_d + (4.0 * scale), seal_d + (4.0 * scale)],
            color: color::ANTIQUE,
        });
        instances.push(GpuInstance {
            rect: [seal_x, seal_y, seal_d, seal_d],
            color: color::BRASS,
        });
        instances.push(GpuInstance {
            rect: [
                seal_x + (4.0 * scale),
                seal_y + (4.0 * scale),
                seal_d - (8.0 * scale),
                seal_d - (8.0 * scale),
            ],
            color: color::darken(color::BRASS, 0.15),
        });
        let close_font = typography::size(typography::BODY, window_h).max(16.0) * 1.25;
        text_labels.push(TextLabel {
            rect: [seal_x, seal_y, seal_d, seal_d],
            text: "Close".into(),
            color: color::PARCHMENT,
            align: TextAlign::Center,
            font_px: Some(close_font),
            ..Default::default()
        });
        buttons.push(ButtonDef::scene(
            (seal_x, seal_y, seal_d, seal_d),
            JOURNAL_CLOSE_ID,
        ));
    }

    /// Canonical-frame variant of [`Self::draw`] for scenes that have
    /// migrated off the dual-vec `SceneDrawOutput` model. Mirrors
    /// `GlossaryOverlay::draw_into_frame`.
    pub fn draw_into_frame(
        &self,
        frame: &mut UiFrame,
        window_w: f32,
        window_h: f32,
        run: &RunState,
    ) {
        if !self.open {
            return;
        }
        let mut quads: Vec<GpuInstance> = Vec::new();
        let mut text: Vec<TextLabel> = Vec::new();
        let mut btns: Vec<ButtonDef> = Vec::new();
        self.draw(window_w, window_h, run, &mut quads, &mut text, &mut btns);
        frame.quads(quads);
        frame.texts(text);
        frame.buttons.extend(btns);
    }
}
