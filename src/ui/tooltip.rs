//! Wikipedia-style nested tooltip system.
//!
//! Hovering a glossary term instantly opens a tooltip.  The tooltip stays open
//! while the cursor remains inside the tooltip rect **or** the word region that
//! spawned it (mirroring CSS parent-containment).  Glossary terms inside the
//! tooltip are themselves hoverable, creating an unbounded nesting chain.

use crate::core::relic::all_relic_defs;
use crate::render::decal::{load_ui_font, measure_label_advances};
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::color as themec;
use crate::render::wgpu_renderer::{GpuInstance, RelicIcon, TextLabel};
use crate::ui::glossary;

// ── Data types ───────────────────────────────────────────────────────────

/// A hoverable region on screen — either a glossary term inside a text label
/// or a UI element like a relic icon.  Carries its own title+description so
/// the tooltip system doesn't need to know where the data came from.
struct HoverRegion {
    title: &'static str,
    description: &'static str,
    rect: [f32; 4],
}

/// One tooltip in the nesting chain.
struct TooltipEntry {
    title: &'static str,
    lines: Vec<String>,
    /// Screen rect of the whole tooltip box.
    rect: [f32; 4],
    /// The hover region that spawned this tooltip (part of its containment zone).
    anchor_rect: [f32; 4],
    /// Hoverable glossary terms inside the description.
    word_regions: Vec<HoverRegion>,
    padding: f32,
    line_height: f32,
}

impl TooltipEntry {
    /// The tooltip stays open while the cursor is inside its rect OR on the
    /// anchor word that spawned it — the same semantics as a CSS child element
    /// being hovered counting as a hover on the parent.
    fn contains(&self, cursor: (f32, f32)) -> bool {
        hit(cursor, self.rect) || hit(cursor, self.anchor_rect)
    }
}

/// Manages the tooltip chain and produces render data each frame.
pub struct TooltipState {
    chain: Vec<TooltipEntry>,
}

// ── Public API ───────────────────────────────────────────────────────────

impl TooltipState {
    pub fn new() -> Self {
        Self { chain: Vec::new() }
    }

    pub fn is_active(&self) -> bool {
        !self.chain.is_empty()
    }

    pub fn clear(&mut self) {
        self.chain.clear();
    }

    /// Run every frame.  Pushes one tooltip's worth of draw cmds at a time
    /// into `frame.cmds`, in chain order — parent first, then child — so
    /// child tooltips fully occlude their parents (DOM-like z-ordering).
    /// `frame` should already contain everything the tooltips need to render
    /// on top of (scene, modals, etc.).
    pub fn update_and_draw_into(
        &mut self,
        frame: &mut UiFrame,
        cursor: (f32, f32),
        base_labels: &[TextLabel],
        button_rects: &[(f32, f32, f32, f32)],
        relic_icons: &[RelicIcon],
        window_w: f32,
        window_h: f32,
    ) {
        let font = match load_ui_font() {
            Some(f) => f,
            None => {
                self.clear();
                return;
            }
        };

        let scale = (window_w.min(window_h) / 600.0).max(0.5);

        // Filter out button labels.
        let non_button_labels: Vec<&TextLabel> = base_labels
            .iter()
            .filter(|l| {
                !button_rects
                    .iter()
                    .any(|&(bx, by, bw, bh)| rects_overlap(l.rect, [bx, by, bw, bh]))
            })
            .collect();

        let mut base_regions = regions_for_label_refs(&font, &non_button_labels, &[]);
        // Relic icons are first-class hover regions: hovering an icon shows
        // its name + description like any glossary term.
        base_regions.extend(relic_hover_regions(relic_icons));

        // ── Step 1: find deepest tooltip whose containment zone has cursor ─
        // Containment = tooltip rect ∪ anchor word rect.
        let cursor_depth: i32 = self
            .chain
            .iter()
            .enumerate()
            .rev()
            .find(|(_, e)| e.contains(cursor))
            .map(|(i, _)| i as i32)
            .unwrap_or(-1);

        // ── Step 2: trim the chain ───────────────────────────────────────
        if cursor_depth < 0 {
            self.chain.clear();
        } else {
            self.chain.truncate((cursor_depth + 1) as usize);
        }

        // ── Step 3: check for a hovered region at the active depth ───────
        let hovered: Option<(&'static str, &'static str, [f32; 4])> = if self.chain.is_empty() {
            hovered_region(cursor, &base_regions)
        } else {
            let tip = &self.chain[self.chain.len() - 1];
            // Only scan tooltip word regions when cursor is inside the
            // tooltip rect itself (not just on its anchor word).
            if hit(cursor, tip.rect) {
                hovered_region(cursor, &tip.word_regions)
            } else {
                None
            }
        };

        // ── Step 4: open / keep / replace child tooltip ──────────────────
        if let Some((title, description, anchor_rect)) = hovered {
            let already_open = self.chain.last().map(|t| t.title == title).unwrap_or(false);
            if !already_open {
                let exclude: Vec<&str> = self.chain.iter().map(|e| e.title).collect();
                let entry = build_tooltip(
                    &font,
                    title,
                    description,
                    anchor_rect,
                    scale,
                    window_w,
                    window_h,
                    &exclude,
                );
                self.chain.push(entry);
            }
        }

        // ── Step 5: push each tooltip's quads then text into the frame ──
        // Earlier-pushed tooltips render under later ones (parent → child),
        // matching DOM z-order.
        for entry in &self.chain {
            draw_tooltip_into(entry, frame);
        }
    }
}

// ── Geometry helpers ─────────────────────────────────────────────────────

fn hit(p: (f32, f32), r: [f32; 4]) -> bool {
    p.0 >= r[0] && p.0 <= r[0] + r[2] && p.1 >= r[1] && p.1 <= r[1] + r[3]
}

fn hovered_region(
    cursor: (f32, f32),
    regions: &[HoverRegion],
) -> Option<(&'static str, &'static str, [f32; 4])> {
    regions
        .iter()
        .find(|r| hit(cursor, r.rect))
        .map(|r| (r.title, r.description, r.rect))
}

/// Build a hoverable region for each visible relic icon.  The title and
/// description come from the relic's static def.
fn relic_hover_regions(icons: &[RelicIcon]) -> Vec<HoverRegion> {
    let defs = all_relic_defs();
    icons
        .iter()
        .filter_map(|icon| {
            defs.iter()
                .find(|d| d.id == icon.relic_id)
                .map(|d| HoverRegion {
                    title: d.name,
                    description: d.description,
                    rect: icon.rect,
                })
        })
        .collect()
}

fn rects_overlap(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[0] + b[2] && a[0] + a[2] > b[0] && a[1] < b[1] + b[3] && a[1] + a[3] > b[1]
}

// ── Word-region computation ──────────────────────────────────────────────

fn regions_for_label_refs(
    font: &fontdue::Font,
    labels: &[&TextLabel],
    exclude: &[&str],
) -> Vec<HoverRegion> {
    let owned: Vec<TextLabel> = labels
        .iter()
        .map(|l| TextLabel {
            rect: l.rect,
            text: l.text.clone(),
            color: l.color,
            ..Default::default()
        })
        .collect();
    regions_for_labels(font, &owned, exclude)
}

fn regions_for_labels(
    font: &fontdue::Font,
    labels: &[TextLabel],
    exclude: &[&str],
) -> Vec<HoverRegion> {
    let mut out = Vec::new();
    for label in labels {
        let matches = glossary::find_terms_in_text(&label.text);
        if matches.is_empty() {
            continue;
        }
        let [lx, ly, lw, lh] = label.rect;
        let w = lw as u32;
        let h = lh as u32;
        if w == 0 || h == 0 {
            continue;
        }
        let (_fp, start_x, advances) = measure_label_advances(font, &label.text, w, h);

        let mut cum = Vec::with_capacity(advances.len() + 1);
        cum.push(0.0f32);
        for &a in &advances {
            cum.push(cum.last().unwrap() + a);
        }

        for m in &matches {
            if exclude.iter().any(|&e| e == m.entry.term) {
                continue;
            }
            if m.char_end >= cum.len() {
                continue;
            }
            let x0 = lx + start_x + cum[m.char_start];
            let x1 = lx + start_x + cum[m.char_end];
            out.push(HoverRegion {
                title: m.entry.term,
                description: m.entry.description,
                rect: [x0, ly, x1 - x0, lh],
            });
        }
    }
    out
}

// ── Word wrapping ────────────────────────────────────────────────────────

fn wrap_text(font: &fontdue::Font, text: &str, font_px: f32, max_w: f32) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return vec![String::new()];
    }
    let space = font.metrics(' ', font_px).advance_width;

    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut lw = 0.0f32;

    for word in words {
        let ww: f32 = word
            .chars()
            .map(|c| font.metrics(c, font_px).advance_width)
            .sum();
        let need = if line.is_empty() { ww } else { space + ww };

        if lw + need > max_w && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
            lw = ww;
        } else {
            if !line.is_empty() {
                line.push(' ');
                lw += space;
            }
            line.push_str(word);
            lw += ww;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

// ── Tooltip construction ─────────────────────────────────────────────────

fn build_tooltip(
    font: &fontdue::Font,
    title: &'static str,
    description: &'static str,
    anchor: [f32; 4],
    scale: f32,
    win_w: f32,
    win_h: f32,
    exclude: &[&str],
) -> TooltipEntry {
    let padding = (10.0 * scale).max(6.0);
    let tooltip_w = (280.0 * scale).max(160.0);
    let line_h = (22.0 * scale).max(14.0);
    let content_w = tooltip_w - padding * 2.0;

    let font_px = (line_h * 0.55).max(8.0);
    let lines = wrap_text(font, description, font_px, content_w);

    let tooltip_h = padding * 2.0 + line_h + padding * 0.5 + lines.len() as f32 * line_h;

    // Position to the right of the anchor, vertically aligned.
    let mut tx = anchor[0] + anchor[2] + 4.0;
    let mut ty = anchor[1];

    if tx + tooltip_w > win_w {
        tx = (anchor[0] - tooltip_w - 4.0).max(4.0);
    }
    if ty + tooltip_h > win_h {
        ty = (win_h - tooltip_h - 4.0).max(4.0);
    }

    let rect = [tx, ty, tooltip_w, tooltip_h];

    let mut full_exclude: Vec<&str> = exclude.to_vec();
    full_exclude.push(title);

    let desc_y = ty + padding + line_h + padding * 0.5;
    let desc_labels: Vec<TextLabel> = lines
        .iter()
        .enumerate()
        .map(|(i, txt)| TextLabel {
            rect: [tx + padding, desc_y + i as f32 * line_h, content_w, line_h],
            text: txt.clone(),
            color: themec::PARCHMENT,
            ..Default::default()
        })
        .collect();

    let word_regions = regions_for_labels(font, &desc_labels, &full_exclude);

    TooltipEntry {
        title,
        lines,
        rect,
        anchor_rect: anchor,
        word_regions,
        padding,
        line_height: line_h,
    }
}

// ── Tooltip rendering ────────────────────────────────────────────────────

fn draw_tooltip_into(entry: &TooltipEntry, frame: &mut UiFrame) {
    let [tx, ty, tw, th] = entry.rect;
    let pad = entry.padding;
    let lh = entry.line_height;
    let border = 2.0;

    // Background — Midnight Gold deep panel.
    frame.quad(GpuInstance {
        rect: [tx, ty, tw, th],
        color: themec::alpha(themec::MIDNIGHT, 0.96),
    });

    // Gold border.
    let bc = themec::BRASS;
    frame.quad(GpuInstance {
        rect: [tx, ty, tw, border],
        color: bc,
    });
    frame.quad(GpuInstance {
        rect: [tx, ty + th - border, tw, border],
        color: bc,
    });
    frame.quad(GpuInstance {
        rect: [tx, ty, border, th],
        color: bc,
    });
    frame.quad(GpuInstance {
        rect: [tx + tw - border, ty, border, th],
        color: bc,
    });

    // Separator line (under the title text).
    let sep_y = ty + pad + lh + pad * 0.25;
    frame.quad(GpuInstance {
        rect: [tx + pad, sep_y, tw - pad * 2.0, 1.0],
        color: themec::alpha(themec::ANTIQUE, 0.7),
    });

    // Underlines for hoverable glossary terms (drawn under description text).
    for region in &entry.word_regions {
        frame.quad(GpuInstance {
            rect: [
                region.rect[0],
                region.rect[1] + region.rect[3] * 0.85,
                region.rect[2],
                1.5,
            ],
            color: themec::alpha(themec::CHAMPAGNE, 0.7),
        });
    }

    // Title.
    frame.text(TextLabel {
        rect: [tx + pad, ty + pad, tw - pad * 2.0, lh],
        text: entry.title.to_string(),
        color: themec::CHAMPAGNE,
        ..Default::default()
    });

    // Description lines.
    let desc_y = ty + pad + lh + pad * 0.5;
    for (i, line) in entry.lines.iter().enumerate() {
        frame.text(TextLabel {
            rect: [tx + pad, desc_y + i as f32 * lh, tw - pad * 2.0, lh],
            text: line.clone(),
            color: themec::PARCHMENT,
            ..Default::default()
        });
    }
}
