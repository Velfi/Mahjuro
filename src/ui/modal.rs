//! Modal overlay system for toasting important game events.

use std::time::Instant;

use rand::RngExt;

use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::scenes::ButtonDef;
use crate::ui::input::UiAction;

/// Color theme for a modal.
#[derive(Clone, Copy)]
pub enum ModalTheme {
    /// Gold/warm — level up, round win.
    Success,
}

impl ModalTheme {
    fn bg_color(&self) -> [f32; 4] {
        match self {
            ModalTheme::Success => [0.12, 0.14, 0.08, 0.95],
        }
    }

    fn border_color(&self) -> [f32; 4] {
        match self {
            ModalTheme::Success => [0.85, 0.75, 0.2, 0.9],
        }
    }

    fn title_color(&self) -> [f32; 4] {
        match self {
            ModalTheme::Success => [1.0, 0.92, 0.4, 1.0],
        }
    }

    fn body_color(&self) -> [f32; 4] {
        match self {
            ModalTheme::Success => [0.9, 0.88, 0.7, 1.0],
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
        }
    }

    /// Create a modal with firework particles.
    pub fn with_fireworks(mut self, center_x: f32, base_y: f32, spread: f32, count: usize) -> Self {
        let mut fw = Fireworks::new();
        fw.launch(center_x, base_y, spread, count);
        self.fireworks = Some(fw);
        self
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
        if let Some(modal) = self.queue.first_mut() {
            if let Some(ref mut fw) = modal.fireworks {
                fw.update(dt);
            }
        }
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

    /// Generate GPU instances, text labels, and a dismiss button for the active modal.
    /// Returns `None` if no modal is active.
    pub fn draw(
        &self,
        window_w: f32,
        window_h: f32,
    ) -> Option<(Vec<GpuInstance>, Vec<TextLabel>, Vec<ButtonDef>)> {
        let modal = self.queue.first()?;
        let alpha = modal.opacity();
        let scale = (window_w.min(window_h)) / 600.0;

        let mut instances = Vec::new();
        let mut labels = Vec::new();
        let mut buttons = Vec::new();

        // Dim overlay behind the modal — Midnight Gold deep indigo, not pure black.
        let [or_, og, ob, _] = crate::render::theme::color::OBSIDIAN;
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: [or_, og, ob, 0.65 * alpha],
        });

        // Modal card dimensions.
        let card_w = (360.0 * scale).min(window_w * 0.8);
        let title_h = (48.0 * scale).max(28.0);
        let body_h = (36.0 * scale).max(20.0);
        let dismiss_h = (28.0 * scale).max(18.0);
        let padding = (20.0 * scale).max(10.0);
        let card_h =
            padding + title_h + padding * 0.5 + body_h + padding * 0.75 + dismiss_h + padding;

        let card_x = (window_w - card_w) * 0.5;
        let card_y = (window_h - card_h) * 0.5;

        // Border (slightly larger card behind).
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

        // Title text — use a proportioned rect for readability.
        let title_y = card_y + padding;
        let [tr, tg, tb, ta] = modal.theme.title_color();
        labels.push(TextLabel {
            rect: [card_x + padding, title_y, card_w - padding * 2.0, title_h],
            text: modal.title.clone(),
            color: [tr, tg, tb, ta * alpha],
        });

        // Body text.
        let body_y = title_y + title_h + padding * 0.5;
        let [dr, dg, db, da] = modal.theme.body_color();
        labels.push(TextLabel {
            rect: [card_x + padding, body_y, card_w - padding * 2.0, body_h],
            text: modal.body.clone(),
            color: [dr, dg, db, da * alpha],
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
        });

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

        Some((instances, labels, buttons))
    }
}
