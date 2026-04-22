//! Modal overlay system for toasting important game events.

use std::time::Instant;

use rand::RngExt;

use crate::core::relic::RelicId;
use crate::core::relic::relic_visual;
use crate::render::draw_cmd::{Object3d, Object3dKind};
use crate::render::table_transform::rot_rx_ry_rz_deg;
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::scenes::ButtonDef;
use crate::ui::input::UiAction;
use crate::ui::widget::wrap_text;

/// Assembled render data emitted by `ModalQueue::draw`: instanced quads,
/// text labels, clickable button rects, and 3D relic meshes.
pub type ModalDrawOutput = (
    Vec<GpuInstance>,
    Vec<TextLabel>,
    Vec<ButtonDef>,
    Vec<Object3d>,
);

/// Mutable buffers that a paginated modal's `draw_paginated` pushes
/// into: 2D instanced quads, text labels, and the 3D relic objects that
/// render above each page.
struct ModalDrawSink<'a> {
    instances: &'a mut Vec<GpuInstance>,
    labels: &'a mut Vec<TextLabel>,
    relic_objects: &'a mut Vec<Object3d>,
}

/// A single page in a paginated unlock carousel.
pub struct UnlockPage {
    pub category: String,
    pub name: String,
    pub description: String,
    pub relic_id: Option<RelicId>,
    pub accent_color: [f32; 4],
}

/// Color theme for a modal.
#[derive(Clone, Copy)]
pub enum ModalTheme {
    /// Gold/warm — level up, round win.
    Success,
    /// Cool indigo — informational (e.g. update available).
    Info,
}

impl ModalTheme {
    fn bg_color(&self) -> [f32; 4] {
        match self {
            ModalTheme::Success => [0.12, 0.14, 0.08, 0.95],
            ModalTheme::Info => [0.08, 0.10, 0.18, 0.95],
        }
    }

    fn border_color(&self) -> [f32; 4] {
        match self {
            ModalTheme::Success => [0.85, 0.75, 0.2, 0.9],
            ModalTheme::Info => [0.4, 0.5, 0.85, 0.9],
        }
    }

    fn title_color(&self) -> [f32; 4] {
        match self {
            ModalTheme::Success => [1.0, 0.92, 0.4, 1.0],
            ModalTheme::Info => [0.7, 0.8, 1.0, 1.0],
        }
    }

    fn body_color(&self) -> [f32; 4] {
        match self {
            ModalTheme::Success => [0.9, 0.88, 0.7, 1.0],
            ModalTheme::Info => [0.85, 0.88, 0.95, 1.0],
        }
    }
}

/// A single firework rocket: launches upward, then bursts into sparks.
#[derive(Clone)]
struct Rocket {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    color: [f32; 3],
    fuse: f32, // time until burst (seconds)
    trail_timer: f32,
}

/// A spark from a firework burst.
#[derive(Clone)]
struct Spark {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    color: [f32; 3],
    life: f32,
    size: f32,
}

/// Firework particle system: rockets launch, burst into radial sparks.
pub struct Fireworks {
    rockets: Vec<Rocket>,
    sparks: Vec<Spark>,
    /// Accumulated trail sparks (tiny particles behind rockets).
    trails: Vec<Spark>,
}

impl Fireworks {
    pub fn new() -> Self {
        Self {
            rockets: Vec::new(),
            sparks: Vec::new(),
            trails: Vec::new(),
        }
    }

    /// Launch a salvo of firework rockets from the bottom of the given rect area.
    pub fn launch(&mut self, center_x: f32, base_y: f32, spread: f32, count: usize) {
        let mut rng = rand::rng();
        let colors: [[f32; 3]; 6] = [
            [1.0, 0.3, 0.2],  // red
            [0.2, 0.9, 0.4],  // green
            [0.3, 0.5, 1.0],  // blue
            [1.0, 0.85, 0.2], // gold
            [0.9, 0.3, 0.9],  // magenta
            [0.2, 0.9, 0.9],  // cyan
        ];
        for _ in 0..count {
            let color = colors[rng.random_range(0..colors.len())];
            let x = center_x + (rng.random::<f32>() - 0.5) * spread;
            let vx = (rng.random::<f32>() - 0.5) * 60.0;
            let vy = -(180.0 + rng.random::<f32>() * 120.0); // upward
            let fuse = 0.5 + rng.random::<f32>() * 0.6;
            self.rockets.push(Rocket {
                x,
                y: base_y,
                vx,
                vy,
                color,
                fuse,
                trail_timer: 0.0,
            });
        }
    }

    /// Advance simulation by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        let mut rng = rand::rng();
        let mut new_sparks = Vec::new();

        // Update rockets.
        for r in &mut self.rockets {
            r.x += r.vx * dt;
            r.y += r.vy * dt;
            r.vy += 40.0 * dt; // slight gravity on rockets
            r.fuse -= dt;

            // Trail particles.
            r.trail_timer -= dt;
            if r.trail_timer <= 0.0 {
                r.trail_timer = 0.02;
                self.trails.push(Spark {
                    x: r.x,
                    y: r.y,
                    vx: (rng.random::<f32>() - 0.5) * 10.0,
                    vy: 15.0 + rng.random::<f32>() * 10.0,
                    color: [0.9, 0.8, 0.5],
                    life: 0.3,
                    size: 2.0,
                });
            }
        }

        // Burst rockets whose fuse expired.
        let mut i = 0;
        while i < self.rockets.len() {
            if self.rockets[i].fuse <= 0.0 {
                let r = self.rockets.remove(i);
                let spark_count = 25 + rng.random_range(0..15);
                for _ in 0..spark_count {
                    let angle = rng.random::<f32>() * std::f32::consts::TAU;
                    let speed = 40.0 + rng.random::<f32>() * 100.0;
                    new_sparks.push(Spark {
                        x: r.x,
                        y: r.y,
                        vx: angle.cos() * speed,
                        vy: angle.sin() * speed,
                        color: r.color,
                        life: 0.6 + rng.random::<f32>() * 0.5,
                        size: 2.0 + rng.random::<f32>() * 3.0,
                    });
                }
            } else {
                i += 1;
            }
        }
        self.sparks.extend(new_sparks);

        // Update sparks.
        for s in &mut self.sparks {
            s.x += s.vx * dt;
            s.y += s.vy * dt;
            s.vy += 60.0 * dt; // gravity
            s.vx *= 0.98; // air resistance
            s.life -= dt;
        }
        self.sparks.retain(|s| s.life > 0.0);

        // Update trails.
        for t in &mut self.trails {
            t.x += t.vx * dt;
            t.y += t.vy * dt;
            t.life -= dt;
        }
        self.trails.retain(|t| t.life > 0.0);
    }

    pub fn is_active(&self) -> bool {
        !self.rockets.is_empty() || !self.sparks.is_empty() || !self.trails.is_empty()
    }

    /// Generate GPU instances for all firework particles.
    pub fn instances(&self) -> Vec<([f32; 4], [f32; 4])> {
        let mut out = Vec::new();

        // Rocket heads (bright).
        for r in &self.rockets {
            let s = 4.0;
            out.push((
                [r.x - s * 0.5, r.y - s * 0.5, s, s],
                [r.color[0], r.color[1], r.color[2], 1.0],
            ));
        }

        // Trails (dim, small).
        for t in &self.trails {
            let alpha = (t.life / 0.3).max(0.0) * 0.6;
            out.push((
                [t.x - t.size * 0.5, t.y - t.size * 0.5, t.size, t.size],
                [t.color[0], t.color[1], t.color[2], alpha],
            ));
        }

        // Burst sparks.
        for s in &self.sparks {
            let alpha = (s.life / 1.1).max(0.0);
            out.push((
                [s.x - s.size * 0.5, s.y - s.size * 0.5, s.size, s.size],
                [s.color[0], s.color[1], s.color[2], alpha],
            ));
        }

        out
    }
}

/// A single modal to display.
pub struct Modal {
    pub title: String,
    pub body: String,
    pub theme: ModalTheme,
    /// When the modal was shown (for fade-in animation).
    shown_at: Instant,
    /// How long the fade-in takes in seconds.
    fade_in_secs: f32,
    /// Optional firework celebration.
    pub fireworks: Option<Fireworks>,
    /// Paginated unlock pages (empty for normal modals).
    pub pages: Vec<UnlockPage>,
    /// Current page index when pages are present.
    pub current_page: usize,
}

impl Modal {
    pub fn new(title: impl Into<String>, body: impl Into<String>, theme: ModalTheme) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            theme,
            shown_at: Instant::now(),
            fade_in_secs: 0.25,
            fireworks: None,
            pages: Vec::new(),
            current_page: 0,
        }
    }

    /// Create a modal with firework particles.
    pub fn with_fireworks(mut self, center_x: f32, base_y: f32, spread: f32, count: usize) -> Self {
        let mut fw = Fireworks::new();
        fw.launch(center_x, base_y, spread, count);
        self.fireworks = Some(fw);
        self
    }

    /// Attach paginated unlock pages to this modal.
    pub fn with_pages(mut self, pages: Vec<UnlockPage>) -> Self {
        self.pages = pages;
        self.current_page = 0;
        self
    }

    pub fn has_pages(&self) -> bool {
        !self.pages.is_empty()
    }

    /// Current opacity based on fade-in progress (0.0 to 1.0).
    fn opacity(&self) -> f32 {
        let elapsed = self.shown_at.elapsed().as_secs_f32();
        (elapsed / self.fade_in_secs).min(1.0)
    }
}

/// A queue of modals. Only the front modal is visible; dismissing it reveals the next.
pub struct ModalQueue {
    queue: Vec<Modal>,
    last_update: Instant,
}

impl Default for ModalQueue {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            last_update: Instant::now(),
        }
    }
}

impl ModalQueue {
    pub fn push(&mut self, modal: Modal) {
        self.queue.push(modal);
    }

    /// Whether a modal is currently blocking input.
    pub fn is_active(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Whether the modal has active animations that need redraws.
    pub fn needs_redraw(&self) -> bool {
        if let Some(modal) = self.queue.first() {
            if modal.opacity() < 1.0 {
                return true;
            }
            if let Some(ref fw) = modal.fireworks {
                return fw.is_active();
            }
        }
        false
    }

    /// Tick firework particles on the active modal.
    pub fn update(&mut self) {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_update)
            .as_secs_f32();
        self.last_update = now;
        if let Some(modal) = self.queue.first_mut()
            && let Some(ref mut fw) = modal.fireworks
        {
            fw.update(dt);
        }
    }

    /// Navigate pages on the active modal by `delta` (-1 = left, +1 = right).
    pub fn navigate(&mut self, delta: i32) {
        if let Some(modal) = self.queue.first_mut()
            && modal.has_pages()
        {
            let new = (modal.current_page as i32 + delta)
                .max(0)
                .min(modal.pages.len() as i32 - 1) as usize;
            if new != modal.current_page {
                modal.current_page = new;
                modal.shown_at = Instant::now(); // restart fade-in
            }
        }
    }

    /// Advance to the next page, or dismiss if on the last page (or no pages).
    /// Returns `true` if there was something to advance/dismiss.
    pub fn advance_page(&mut self) -> bool {
        if let Some(modal) = self.queue.first_mut()
            && modal.has_pages()
            && modal.current_page + 1 < modal.pages.len()
        {
            modal.current_page += 1;
            modal.shown_at = Instant::now();
            return true;
        }
        self.dismiss()
    }

    /// Dismiss the current modal. Returns `true` if there was one to dismiss.
    pub fn dismiss(&mut self) -> bool {
        if self.queue.is_empty() {
            false
        } else {
            self.queue.remove(0);
            // Reset fade-in timer on the next modal.
            if let Some(next) = self.queue.first_mut() {
                next.shown_at = Instant::now();
            }
            true
        }
    }

    /// Generate GPU instances, text labels, 3D relic placements, and buttons
    /// for the active modal.
    /// Returns `None` if no modal is active.
    pub fn draw(&self, window_w: f32, window_h: f32, ui_scale: f32) -> Option<ModalDrawOutput> {
        let modal = self.queue.first()?;
        let alpha = modal.opacity();
        let scale = (window_w.min(window_h)) / 600.0 * ui_scale;

        let mut instances = Vec::new();
        let mut labels = Vec::new();
        let mut buttons = Vec::new();
        let mut relic_objects: Vec<Object3d> = Vec::new();

        // Dim overlay behind the modal — Midnight Gold deep indigo, not pure black.
        let [or_, og, ob, _] = crate::render::theme::color::OBSIDIAN;
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: [or_, og, ob, 0.65 * alpha],
        });

        if modal.has_pages() {
            self.draw_paginated(
                modal,
                alpha,
                scale,
                (window_w, window_h),
                ModalDrawSink {
                    instances: &mut instances,
                    labels: &mut labels,
                    relic_objects: &mut relic_objects,
                },
            );
        } else {
            self.draw_simple(
                modal,
                alpha,
                scale,
                (window_w, window_h),
                &mut instances,
                &mut labels,
            );
        }

        // Firework particles (rendered on top of dim overlay but mixed with card).
        if let Some(ref fw) = modal.fireworks {
            for (rect, color) in fw.instances() {
                instances.push(GpuInstance { rect, color });
            }
        }

        // Full-screen dismiss button so clicking anywhere also works.
        buttons.push(ButtonDef::ui(
            (0.0, 0.0, window_w, window_h),
            UiAction::Confirm,
        ));

        Some((instances, labels, buttons, relic_objects))
    }

    /// Draw a simple title+body modal (original behavior).
    fn draw_simple(
        &self,
        modal: &Modal,
        alpha: f32,
        scale: f32,
        window: (f32, f32),
        instances: &mut Vec<GpuInstance>,
        labels: &mut Vec<TextLabel>,
    ) {
        let (window_w, window_h) = window;
        let card_w = (360.0 * scale).min(window_w * 0.8);
        let title_h = (48.0 * scale).max(28.0);
        let dismiss_h = (28.0 * scale).max(18.0);
        let padding = (20.0 * scale).max(10.0);
        let body_font = (18.0 * scale).max(14.0);
        let body_inner_w = card_w - padding * 2.0;
        let wrapped = wrap_text(&modal.body, body_inner_w, body_font);
        let body_line_step = body_font * 1.4;
        let body_lines = wrapped.len().max(1) as f32;
        let chrome_h = padding + title_h + padding * 0.5 + padding * 0.75 + dismiss_h + padding;
        let max_body_h = window_h * 0.85 - chrome_h;
        let body_h = (body_lines * body_line_step)
            .max(20.0)
            .min(max_body_h.max(20.0));
        let body_text = wrapped.join("\n");
        let card_h = chrome_h + body_h;

        let card_x = (window_w - card_w) * 0.5;
        let card_y = ((window_h - card_h) * 0.5).max(8.0);

        // Border.
        let border = 3.0 * scale;
        let [br, bg, bb, ba] = modal.theme.border_color();
        instances.push(GpuInstance {
            rect: [
                card_x - border,
                card_y - border,
                card_w + border * 2.0,
                card_h + border * 2.0,
            ],
            color: [br, bg, bb, ba * alpha],
        });

        // Card background.
        let [cr, cg, cb, ca] = modal.theme.bg_color();
        instances.push(GpuInstance {
            rect: [card_x, card_y, card_w, card_h],
            color: [cr, cg, cb, ca * alpha],
        });

        // Title.
        let title_y = card_y + padding;
        let [tr, tg, tb, ta] = modal.theme.title_color();
        labels.push(TextLabel {
            rect: [card_x + padding, title_y, card_w - padding * 2.0, title_h],
            text: modal.title.clone(),
            color: [tr, tg, tb, ta * alpha],
            font_px: Some(title_h * 0.65),
            ..Default::default()
        });

        // Body.
        let body_y = title_y + title_h + padding * 0.5;
        let [dr, dg, db, da] = modal.theme.body_color();
        labels.push(TextLabel {
            rect: [card_x + padding, body_y, body_inner_w, body_h],
            text: body_text,
            color: [dr, dg, db, da * alpha],
            font_px: Some(body_font),
            ..Default::default()
        });

        // Dismiss hint.
        let dismiss_y = body_y + body_h + padding * 0.75;
        labels.push(TextLabel {
            rect: [
                card_x + padding,
                dismiss_y,
                card_w - padding * 2.0,
                dismiss_h,
            ],
            text: "Press Enter to continue".into(),
            color: {
                let [r, g, b, a] = crate::render::theme::color::SLATE;
                [r, g, b, a * 0.8 * alpha]
            },
            font_px: Some((14.0 * scale).max(12.0)),
            ..Default::default()
        });
    }

    /// Draw a paginated unlock carousel page.
    fn draw_paginated(
        &self,
        modal: &Modal,
        alpha: f32,
        scale: f32,
        window: (f32, f32),
        out: ModalDrawSink<'_>,
    ) {
        let ModalDrawSink {
            instances,
            labels,
            relic_objects,
        } = out;
        let (window_w, window_h) = window;
        let page = &modal.pages[modal.current_page];
        let padding = (20.0 * scale).max(10.0);
        let card_w = (400.0 * scale).min(window_w * 0.85);

        // Layout heights.
        let category_h = (30.0 * scale).max(18.0);
        let icon_h = if page.relic_id.is_some() {
            (64.0 * scale).max(40.0)
        } else {
            0.0
        };
        let icon_gap = if page.relic_id.is_some() {
            padding * 0.5
        } else {
            0.0
        };
        let name_h = (44.0 * scale).max(26.0);
        let desc_lines = page.description.lines().count().max(1) as f32;
        let desc_h = (desc_lines * 22.0 * scale).max(20.0).min(window_h * 0.3);
        let nav_h = (24.0 * scale).max(16.0);

        let card_h = padding
            + category_h
            + padding * 0.5
            + icon_h
            + icon_gap
            + name_h
            + padding * 0.5
            + desc_h
            + padding * 0.75
            + nav_h
            + padding;
        let card_x = (window_w - card_w) * 0.5;
        let card_y = ((window_h - card_h) * 0.5).max(8.0);

        // Border with page accent color.
        let border = 3.0 * scale;
        let [ar, ag, ab, aa] = page.accent_color;
        instances.push(GpuInstance {
            rect: [
                card_x - border,
                card_y - border,
                card_w + border * 2.0,
                card_h + border * 2.0,
            ],
            color: [ar, ag, ab, aa * alpha],
        });

        // Card background.
        let [cr, cg, cb, ca] = modal.theme.bg_color();
        instances.push(GpuInstance {
            rect: [card_x, card_y, card_w, card_h],
            color: [cr, cg, cb, ca * alpha],
        });

        let mut y = card_y + padding;
        let content_w = card_w - padding * 2.0;

        // Category label (e.g. "New Relic").
        labels.push(TextLabel {
            rect: [card_x + padding, y, content_w, category_h],
            text: page.category.clone(),
            color: {
                let [r, g, b, a] = crate::render::theme::color::SLATE;
                [r, g, b, a * alpha]
            },
            ..Default::default()
        });
        y += category_h + padding * 0.5;

        // Relic icon (if applicable).
        if let Some(relic_id) = page.relic_id {
            let icon_size = icon_h.min(content_w * 0.4);
            let icon_x = card_x + (card_w - icon_size) * 0.5;
            let visual = relic_visual(relic_id);
            let spin_deg = (Instant::now()
                .saturating_duration_since(modal.shown_at)
                .as_secs_f32()
                * visual.ui_spin_rate_deg)
                % 360.0;
            let face_size = icon_size * 0.80;
            let thick = face_size * 0.12 * visual.thickness_scale;
            relic_objects.push(Object3d {
                pos: [
                    icon_x + icon_size * 0.5,
                    y + icon_size * 0.5,
                    icon_size * 0.38,
                ],
                extents: [face_size, thick, face_size],
                rotation: rot_rx_ry_rz_deg(90.0 + visual.ui_tilt_x_deg, spin_deg, 0.0),
                color: page.accent_color,
                kind: Object3dKind::Relic {
                    relic_id,
                    glow: alpha * 0.35,
                    silhouette: false,
                    pick_id: None,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
            y += icon_h + icon_gap;
        }

        // Item name — large, gold.
        let [tr, tg, tb, ta] = modal.theme.title_color();
        labels.push(TextLabel {
            rect: [card_x + padding, y, content_w, name_h],
            text: page.name.clone(),
            color: [tr, tg, tb, ta * alpha],
            ..Default::default()
        });
        y += name_h + padding * 0.5;

        // Description.
        let [dr, dg, db, da] = modal.theme.body_color();
        labels.push(TextLabel {
            rect: [card_x + padding, y, content_w, desc_h],
            text: page.description.clone(),
            color: [dr, dg, db, da * alpha],
            ..Default::default()
        });
        y += desc_h + padding * 0.75;

        // Navigation hint + page indicator.
        let total = modal.pages.len();
        let current = modal.current_page + 1;
        let nav_text = if total > 1 {
            format!(
                "\u{25c0}  {} / {}  \u{25b6}  \u{2022}  Enter to continue",
                current, total
            )
        } else {
            "Enter to continue".into()
        };
        labels.push(TextLabel {
            rect: [card_x + padding, y, content_w, nav_h],
            text: nav_text,
            color: {
                let [r, g, b, a] = crate::render::theme::color::SLATE;
                [r, g, b, a * 0.8 * alpha]
            },
            ..Default::default()
        });
    }
}
