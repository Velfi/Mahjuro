//! Modal overlay system for toasting important game events.

use std::time::{Duration, Instant};

/// Quiet window before a held cancel/back key starts auto-advancing pages.
/// A quick tap (<this) advances exactly one page; longer than this and the
/// modal starts skimming.
const SKIM_INITIAL_DELAY: Duration = Duration::from_millis(250);
/// Interval between subsequent auto-advances while the cancel/back key is
/// still held — fast enough that "skim through all updates" feels snappy,
/// slow enough that each unlock card is still visible in passing.
const SKIM_REPEAT_INTERVAL: Duration = Duration::from_millis(120);

use crate::core::relic::RelicId;
use crate::core::relic::relic_visual;
use crate::render::draw_cmd::{Object3d, Object3dKind};
use crate::render::table_transform::euler_xyz_rad_from_deg;
use crate::render::wgpu_renderer::{GpuInstance, GradientQuadInstance, TextLabel};
use crate::scenes::ButtonDef;
use crate::ui::input::UiAction;
use crate::ui::widget::wrap_text;

/// Assembled render data emitted by `ModalQueue::draw`: instanced quads,
/// text labels, clickable button rects, 3D relic meshes, and radial-
/// gradient quads (used for the spotlight halo + lantern-mote glow,
/// which need shader-side circular falloff that flat axis-aligned
/// `GpuInstance` quads can't fake).
pub type ModalDrawOutput = (
    Vec<GpuInstance>,
    Vec<TextLabel>,
    Vec<ButtonDef>,
    Vec<Object3d>,
    Vec<GradientQuadInstance>,
);

/// Mutable buffers that a paginated modal's `draw_paginated` pushes
/// into: 2D instanced quads, text labels, the 3D relic objects that
/// render above each page, and radial-gradient sprites for the halo +
/// mote glow.
struct ModalDrawSink<'a> {
    instances: &'a mut Vec<GpuInstance>,
    labels: &'a mut Vec<TextLabel>,
    relic_objects: &'a mut Vec<Object3d>,
    gradient_quads: &'a mut Vec<GradientQuadInstance>,
}

/// A single page in a paginated unlock carousel.
#[derive(Clone)]
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
    /// Dark walnut panel — informational (e.g. update available).
    Info,
}

impl ModalTheme {
    fn bg_color(&self) -> [f32; 4] {
        use crate::render::theme::color;
        match self {
            // Success = a win at the table → deep walnut background.
            ModalTheme::Success => color::alpha(color::WALNUT_DEEP, 0.95),
            ModalTheme::Info => color::alpha(color::WALNUT_DEEP, 0.95),
        }
    }

    fn border_color(&self) -> [f32; 4] {
        use crate::render::theme::color;
        match self {
            ModalTheme::Success => color::alpha(color::GOLD, 0.9),
            ModalTheme::Info => color::alpha(color::BRASS, 0.9),
        }
    }

    fn title_color(&self) -> [f32; 4] {
        use crate::render::theme::color;
        match self {
            ModalTheme::Success => color::CHAMPAGNE,
            ModalTheme::Info => color::CHAMPAGNE,
        }
    }

    fn body_color(&self) -> [f32; 4] {
        use crate::render::theme::color;
        match self {
            // Bright celebratory text on dark walnut.
            ModalTheme::Success => color::PARCHMENT,
            ModalTheme::Info => color::PARCHMENT,
        }
    }
}

/// One drifting lantern mote: a slow-rising warm spark with subtle horizontal
/// sway, a fade-in lead, a long bright body, and a fade-out tail. The
/// per-mote `phase` makes the swarm asynchronous — replaces the noisy
/// multicolor firework burst with the parlor-lantern aesthetic the rest of
/// the game uses.
#[derive(Clone)]
struct Mote {
    x: f32,
    y: f32,
    vy: f32,
    sway_amp: f32,
    sway_freq: f32,
    color: [f32; 3],
    /// Total lifetime (seconds). Counts down each tick.
    life: f32,
    /// Total intended lifetime — needed to ratio the brightness envelope
    /// against `life`.
    life_total: f32,
    size: f32,
    /// Random phase offset so motes don't all sway in lockstep.
    phase: f32,
}

/// Lantern-mote celebration field — slow warm sparks rise behind the
/// hero relic, sway gently, and fade. Replaces the prior multi-color
/// firework system; the API is unchanged so existing callers
/// (`Modal::with_fireworks`, etc.) keep working.
pub struct Fireworks {
    motes: Vec<Mote>,
    /// Spawn budget remaining (motes left to emit). We stagger spawning
    /// over the first ~2 seconds so the swarm rises in waves rather than
    /// appearing all at once.
    pending_spawn: usize,
    spawn_timer: f32,
    /// Geometry of the spawn band (set by `launch`). New motes pick a
    /// random x within this band; their initial y is `base_y`.
    spawn_center_x: f32,
    spawn_base_y: f32,
    spawn_spread: f32,
}

impl Default for Fireworks {
    fn default() -> Self {
        Self::new()
    }
}

impl Fireworks {
    pub fn new() -> Self {
        Self {
            motes: Vec::new(),
            pending_spawn: 0,
            spawn_timer: 0.0,
            spawn_center_x: 0.0,
            spawn_base_y: 0.0,
            spawn_spread: 0.0,
        }
    }

    /// Schedule a swarm of `count` lantern motes to rise from a band
    /// centered at `(center_x, base_y)` with horizontal `spread`. The
    /// motes spawn over ~2s so the field reads as a continuous rise
    /// rather than a single pop.
    pub fn launch(&mut self, center_x: f32, base_y: f32, spread: f32, count: usize) {
        // Scale up: the new system is gentler per-mote, so to fill the
        // hero stage we want a denser cloud than the old fireworks call
        // would request. ~3x the requested count, then capped.
        let scaled = (count * 3).min(160);
        self.pending_spawn = self.pending_spawn.saturating_add(scaled);
        self.spawn_center_x = center_x;
        self.spawn_base_y = base_y;
        self.spawn_spread = spread.max(1.0);
    }

    /// Spawn one mote with randomized lifetime, color, and sway.
    /// Per-mote size is derived from `spawn_spread` so the field
    /// reads at the same visual density across resolutions — at 3840px
    /// wide a 6px spark is a single dot, but scaling with the spread
    /// keeps the motes legible at TV distance.
    fn spawn_one(&mut self, rng: &mut impl rand::RngExt) {
        use crate::render::theme::color;
        // Warm palette: champagne / amber / soft gold. Subtle hue
        // variance so the swarm doesn't read as a flat monochrome dust.
        // Anchored on the CHAMPAGNE and PARCHMENT tokens; the other two
        // entries are deliberate brighter/darker variants that round out
        // the swarm.
        let palette: [[f32; 3]; 4] = [
            [1.00, 0.86, 0.55], // champagne shoulder, brighter than the token
            [
                color::CHAMPAGNE[0],
                color::CHAMPAGNE[1],
                color::CHAMPAGNE[2],
            ],
            [1.00, 0.72, 0.38], // amber shoulder, warmer than the AMBER token
            [
                color::PARCHMENT[0],
                color::PARCHMENT[1],
                color::PARCHMENT[2],
            ],
        ];
        let color = palette[rng.random_range(0..palette.len())];
        let x = self.spawn_center_x + (rng.random::<f32>() - 0.5) * self.spawn_spread;
        // Velocity and sway amplitude scale with viewport so the
        // motes climb proportional to screen height regardless of
        // resolution. `spawn_spread` is roughly the window width,
        // so 0.018 * spread per second ≈ 1.8% of the screen / sec.
        let speed_unit = self.spawn_spread.max(1.0) * 0.001;
        let vy = -(speed_unit * (24.0 + rng.random::<f32>() * 30.0));
        let sway_amp = speed_unit * (8.0 + rng.random::<f32>() * 18.0);
        let sway_freq = 0.6 + rng.random::<f32>() * 0.8;
        let life_total = 3.0 + rng.random::<f32>() * 2.0;
        // Mote size proportional to spread — 0.0025 * spread is
        // ~6px at 2560-wide and ~10px at 3840-wide, which both
        // read crisply.
        let size_unit = self.spawn_spread * 0.0025;
        let size = size_unit + rng.random::<f32>() * size_unit;
        let phase = rng.random::<f32>() * std::f32::consts::TAU;
        self.motes.push(Mote {
            x,
            y: self.spawn_base_y,
            vy,
            sway_amp,
            sway_freq,
            color,
            life: life_total,
            life_total,
            size,
            phase,
        });
    }

    pub fn update(&mut self, dt: f32) {
        let mut rng = rand::rng();

        // Stagger spawns: emit 1-2 motes per ~30ms while pending > 0.
        if self.pending_spawn > 0 {
            self.spawn_timer -= dt;
            while self.spawn_timer <= 0.0 && self.pending_spawn > 0 {
                self.spawn_one(&mut rng);
                self.pending_spawn -= 1;
                self.spawn_timer += 0.025;
            }
        }

        // Advance every mote.
        for m in &mut self.motes {
            m.life -= dt;
            m.y += m.vy * dt;
            // Subtle horizontal sway: position-driven so motes weave as
            // they rise rather than tracking a straight column.
            let t = (m.life_total - m.life) * m.sway_freq + m.phase;
            // Apply sway as a per-frame x increment so it accumulates
            // smoothly rather than snapping to a sin curve.
            m.x += m.sway_amp * t.sin() * dt;
        }
        self.motes.retain(|m| m.life > 0.0);
    }

    pub fn is_active(&self) -> bool {
        !self.motes.is_empty() || self.pending_spawn > 0
    }

    /// Generate GPU instances for the mote field. Each mote is one
    /// radial-gradient quad — the shader does the smoothstep falloff
    /// from center to edge, so a single quad reads as a soft circular
    /// glow at any resolution. (Earlier versions stacked 4-5
    /// concentric flat quads to fake a halo; that always left the
    /// brightest core looking like a sharp axis-aligned square.)
    /// The returned `rect` is sized as the mote's *full* halo
    /// diameter so the glow extends past the bright core; the shader
    /// fades it to zero at the rect's edge.
    pub fn instances(&self) -> Vec<([f32; 4], [f32; 4])> {
        let mut out = Vec::with_capacity(self.motes.len());
        for m in &self.motes {
            // Brightness envelope: ramp up over first 0.4s, hold,
            // fade out over last 1.0s. Keeps the swarm from popping
            // into existence and gives a "lantern-light" easing.
            let elapsed = m.life_total - m.life;
            let fade_in = (elapsed / 0.4).clamp(0.0, 1.0);
            let fade_out = (m.life / 1.0).clamp(0.0, 1.0);
            let env = fade_in * fade_out;

            // Halo diameter ~5x the per-mote `size` so the visible
            // bright dot reads at roughly the original size while
            // the radial fade extends well past it.
            let s = m.size * 5.0;
            out.push((
                [m.x - s * 0.5, m.y - s * 0.5, s, s],
                [m.color[0], m.color[1], m.color[2], 0.95 * env],
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

    /// Lantern-mote radial quads (same encoding as [`ModalQueue::draw`]).
    pub(crate) fn append_fireworks_gradient_quads(&self, out: &mut Vec<GradientQuadInstance>) {
        if let Some(ref fw) = self.fireworks {
            for (rect, color) in fw.instances() {
                out.push(GradientQuadInstance {
                    rect,
                    color,
                    feather: [1.0, 1.0, 0.0, 0.0],
                });
            }
        }
    }

    /// Current opacity based on fade-in progress (0.0 to 1.0).
    fn opacity(&self) -> f32 {
        self.card_fade_alpha()
    }

    #[inline]
    pub(crate) fn card_fade_alpha(&self) -> f32 {
        let elapsed = self.shown_at.elapsed().as_secs_f32();
        (elapsed / self.fade_in_secs).min(1.0)
    }

    pub(crate) fn advance_unlock_page(&mut self) -> bool {
        if !self.has_pages() {
            return false;
        }
        if self.current_page + 1 < self.pages.len() {
            self.current_page += 1;
            self.shown_at = Instant::now();
            true
        } else {
            false
        }
    }

    pub(crate) fn navigate_unlock_page(&mut self, delta: i32) {
        if !self.has_pages() {
            return;
        }
        let new = (self.current_page as i32 + delta)
            .max(0)
            .min(self.pages.len() as i32 - 1) as usize;
        if new != self.current_page {
            self.current_page = new;
            self.shown_at = Instant::now();
        }
    }
}

/// A queue of modals. Only the front modal is visible; dismissing it reveals the next.
pub struct ModalQueue {
    queue: Vec<Modal>,
    last_update: Instant,
    /// `Some(next_advance_at)` while the player is holding the cancel/back
    /// key over a paginated modal. The first auto-advance fires at
    /// `press_time + SKIM_INITIAL_DELAY`; subsequent advances fire every
    /// `SKIM_REPEAT_INTERVAL` after that. `None` means no skim in progress.
    skim_next_at: Option<Instant>,
}

impl Default for ModalQueue {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            last_update: Instant::now(),
            skim_next_at: None,
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

    /// Tick firework particles on the active modal, and auto-advance pages
    /// while the player is holding the cancel/back key over a paginated
    /// modal (level-up celebration skim).
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
        // Skim: while the cancel/back key is held over a paginated modal,
        // advance one page per `SKIM_REPEAT_INTERVAL`. The first scheduled
        // advance is `SKIM_INITIAL_DELAY` after the initial press (set by
        // `cancel_pressed`), so a quick tap never auto-skims.
        if let Some(next_at) = self.skim_next_at
            && now >= next_at
        {
            let still_paginated = self.queue.first().map(|m| m.has_pages()).unwrap_or(false);
            if still_paginated {
                self.advance_page();
                let paginated_after = self.queue.first().map(|m| m.has_pages()).unwrap_or(false);
                // Keep skimming as long as a paginated modal is still on
                // top (the next modal in queue might also be paginated and
                // continue the skim seamlessly).
                self.skim_next_at = if paginated_after {
                    Some(now + SKIM_REPEAT_INTERVAL)
                } else {
                    None
                };
            } else {
                self.skim_next_at = None;
            }
        }
    }

    /// Press edge for the cancel/back key (gamepad East, Backspace,
    /// Escape). On a paginated modal this advances one page immediately
    /// and arms the held-skim timer so further holding flips through the
    /// remaining unlock cards rapidly. On a non-paginated modal it falls
    /// back to the historical dismiss-on-cancel behavior.
    pub fn cancel_pressed(&mut self) -> bool {
        let paginated = self.queue.first().map(|m| m.has_pages()).unwrap_or(false);
        if paginated {
            let advanced = self.advance_page();
            self.skim_next_at = Some(Instant::now() + SKIM_INITIAL_DELAY);
            advanced
        } else {
            self.skim_next_at = None;
            self.dismiss()
        }
    }

    /// Release edge for the cancel/back key. Stops the held-skim timer.
    pub fn cancel_released(&mut self) {
        self.skim_next_at = None;
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
            // If skimming and the queue just drained, stop ticking.
            if self.queue.is_empty() {
                self.skim_next_at = None;
            }
            true
        }
    }

    /// Generate GPU instances, text labels, 3D relic placements, and buttons
    /// for the active modal.
    /// Returns `None` if no modal is active.
    pub fn draw(&self, window_w: f32, window_h: f32) -> Option<ModalDrawOutput> {
        let modal = self.queue.first()?;
        let alpha = modal.opacity();
        let scale = (window_w.min(window_h)) / 600.0;

        let mut instances = Vec::new();
        let mut labels = Vec::new();
        let mut buttons = Vec::new();
        let mut relic_objects: Vec<Object3d> = Vec::new();
        let mut gradient_quads: Vec<GradientQuadInstance> = Vec::new();

        // Dim overlay behind the modal — WALNUT_INK (near-black walnut base).
        let [or_, og, ob, _] = crate::render::theme::color::WALNUT_INK;
        instances.push(GpuInstance {
            rect: [0.0, 0.0, window_w, window_h],
            color: [or_, og, ob, 0.65 * alpha],
            user: 0,
        });

        if modal.has_pages() {
            let (pag_i, pag_l, pag_r, mut pag_g) =
                modal_paginated_unlock_layer_vecs(modal, alpha, window_w, window_h);
            modal.append_fireworks_gradient_quads(&mut pag_g);
            instances.extend(pag_i);
            labels.extend(pag_l);
            relic_objects.extend(pag_r);
            gradient_quads.extend(pag_g);
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

        // Paginated path already merged fireworks; simple modals may still need them.
        if !modal.has_pages() {
            modal.append_fireworks_gradient_quads(&mut gradient_quads);
        }

        // Full-screen dismiss button so clicking anywhere also works.
        buttons.push(ButtonDef::ui(
            (0.0, 0.0, window_w, window_h),
            UiAction::Confirm,
        ));

        Some((instances, labels, buttons, relic_objects, gradient_quads))
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
        use crate::render::theme::typography;
        let card_w = (360.0 * scale).min(window_w * 0.8);
        let title_px = typography::size(typography::H20, window_h);
        let title_h = title_px * 1.35;
        let padding = (20.0 * scale).max(10.0);
        let body_font = typography::size(typography::H36, window_h);
        let body_inner_w = card_w - padding * 2.0;
        let wrapped = wrap_text(&modal.body, body_inner_w, body_font);
        let body_line_step = crate::ui::colored_keywords::colored_row_line_step(body_font);
        let body_lines = wrapped.len().max(1) as f32;
        let chrome_h = padding + title_h + padding * 0.5 + padding;
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
            user: 0,
        });

        // Card background.
        let [cr, cg, cb, ca] = modal.theme.bg_color();
        instances.push(GpuInstance {
            rect: [card_x, card_y, card_w, card_h],
            color: [cr, cg, cb, ca * alpha],
            user: 0,
        });

        // Title.
        let title_y = card_y + padding;
        let [tr, tg, tb, ta] = modal.theme.title_color();
        labels.push(TextLabel {
            rect: [card_x + padding, title_y, card_w - padding * 2.0, title_h],
            text: modal.title.clone(),
            color: [tr, tg, tb, ta * alpha],
            font_px: Some(title_px),
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

    }
}

/// Paginated unlock layout (museum placard + optional hero relic mesh).
///
/// Used by [`ModalQueue::draw`] and [`modal_paginated_unlock_layer_vecs`]. Does not include the
/// modal queue's full-screen dimmer or dismiss hit target.
fn draw_modal_paginated_unlock(
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
        gradient_quads,
    } = out;
    let (window_w, window_h) = window;
    use crate::render::theme::typography;
    let page = &modal.pages[modal.current_page];

    // ── Vignette (cinematic letterbox) ───────────────────────────
    // Thin horizontal strips along the top and bottom of the
    // screen, accumulating to a smooth dark band that frames the
    // relic stage cinematically. We deliberately skip the left
    // and right edges: full-height side strips create a visible
    // vertical seam against the lit center column, while
    // top-and-bottom only reads as a clean letterbox at TV
    // distance. The relic-name + description column stays
    // unaffected.
    let [vr, vg, vb, _] = crate::render::theme::color::WALNUT_INK;
    let layers = 24;
    let strip_h = window_h * 0.30 / layers as f32;
    for i in 0..layers {
        // Per-strip alpha steady — stacking handles the gradient.
        let a = 0.040 * alpha;
        // Top band.
        instances.push(GpuInstance {
            rect: [0.0, i as f32 * strip_h, window_w, strip_h],
            color: [vr, vg, vb, a],
            user: 0,
        });
        // Bottom band.
        instances.push(GpuInstance {
            rect: [0.0, window_h - (i + 1) as f32 * strip_h, window_w, strip_h],
            color: [vr, vg, vb, a],
            user: 0,
        });
    }

    // ── Hero stage layout ────────────────────────────────────────
    // The relic occupies the upper-middle of the screen; type
    // stacks below it; a footer pins page indicator + dismiss hint
    // to the bottom edge. All sizes are derived from the smaller
    // window dimension so a 4K TV scales up cleanly.
    let base = window_w.min(window_h);
    let center_x = window_w * 0.5;

    // The hero relic disk is sized by the smaller window dimension
    // so it occupies a fixed fraction of the visible field on every
    // aspect ratio — about 28% of the smaller axis, which reads
    // clearly even on a couch-distance TV.
    let icon_size = base * 0.30;
    // Vertical centering biased slightly upward so the name +
    // description below the relic have room without pushing the
    // page indicator out of the lower vignette.
    let icon_center_y = window_h * 0.40;

    // ── Stage spotlight pool ─────────────────────────────────────
    // Single warm radial-gradient quad behind the relic. The
    // gradient_quad shader does the actual circular falloff in
    // the fragment stage (feather.y=1 selects radial mode), so
    // we get a clean soft disc instead of the stacked-rect
    // pseudo-halos earlier versions of this scene used.
    if page.relic_id.is_some() {
        let halo_size = icon_size * 2.6;
        gradient_quads.push(GradientQuadInstance {
            rect: [
                center_x - halo_size * 0.5,
                icon_center_y - halo_size * 0.5,
                halo_size,
                halo_size,
            ],
            color: [1.00, 0.78, 0.42, 0.32 * alpha],
            feather: [1.0, 1.0, 0.0, 0.0],
        });
    }

    // ── Hero relic ───────────────────────────────────────────────
    if let Some(relic_id) = page.relic_id {
        let visual = relic_visual(relic_id);
        let face_size = icon_size * 0.80;
        let thick = face_size * 0.06 * visual.thickness_scale;
        // Camera in main/draw.rs looks along +Y with up=+Z, so
        // screen-vertical comes from world Z (lift) — not pixel Y.
        // Park depth at the camera target plane and move the
        // icon's vertical screen position into the lift.
        //
        // The relic mesh is a thin disk with its broad face on the
        // local XZ plane (normal +Y for the textured front cap).
        // The +Y-looking camera sits at -Y, so we flip the disk
        // 180° so the front face points toward the camera. Mirrors
        // the approach used by the collection scene's relic plaques.
        //
        relic_objects.push(Object3d {
            pos: [center_x, window_h * 0.5, window_h * 0.5 - icon_center_y],
            extents: [face_size, thick, face_size],
            rotation: euler_xyz_rad_from_deg(180.0, 0.0, 0.0),
            color: page.accent_color,
            kind: Object3dKind::Relic {
                relic_id,
                // Modest glow — the 2D spotlight halo behind the
                // relic does most of the visual lift; pushing the
                // disk's own glow too high blooms the textured
                // face into a featureless white circle.
                glow: alpha * 0.20,
                silhouette: false,
                debuffed: false,
            },
            hover_target: 0.0,
            anim_id: 0,
        });
    }

    // ── Type column ──────────────────────────────────────────────
    // All sizes scale against `base` (smaller window dim) so the
    // hierarchy holds at any resolution.
    let category_font = typography::size(typography::H42, window_h);
    let name_font = typography::size(typography::H12, window_h);
    let desc_font = typography::size(typography::H36, window_h);
    let nav_font = typography::size(typography::H42, window_h);

    // Category placard ("New Relic") — sits above the
    // relic, slate tone, smaller. Reads as a museum label.
    let category_y = icon_center_y - icon_size * 0.55 - category_font * 1.6;
    labels.push(TextLabel {
        rect: [
            0.0,
            category_y,
            window_w,
            crate::ui::colored_keywords::colored_row_line_step(category_font),
        ],
        text: page.category.to_uppercase(),
        color: {
            let [r, g, b, _] = crate::render::theme::color::STONE;
            [r, g, b, 0.85 * alpha]
        },
        font_px: Some(category_font),
        ..Default::default()
    });

    // Name — the largest type on screen. Gold-warm, sits just below
    // the relic disk. We treat it as the page's hero label.
    let name_y = icon_center_y + icon_size * 0.55 + base * 0.02;
    let [tr, tg, tb, _ta] = crate::render::theme::color::CHAMPAGNE;
    labels.push(TextLabel {
        rect: [0.0, name_y, window_w, name_font * 1.3],
        text: page.name.clone(),
        color: [tr, tg, tb, alpha],
        font_px: Some(name_font),
        ..Default::default()
    });

    // Description — wraps under the name, generous leading.
    let desc_y = name_y + name_font * 1.5;
    let desc_w = window_w * 0.7;
    let desc_x = (window_w - desc_w) * 0.5;
    let desc_line_step = desc_font * 1.5;
    let wrapped_desc = wrap_text(&page.description, desc_w, desc_font);
    let desc_lines = wrapped_desc.len().max(1) as f32;
    let desc_h = desc_lines * desc_line_step;
    let [dr, dg, db, _da] = crate::render::theme::color::PARCHMENT;
    labels.push(TextLabel {
        rect: [desc_x, desc_y, desc_w, desc_h],
        text: wrapped_desc.join("\n"),
        color: [dr, dg, db, 0.92 * alpha],
        font_px: Some(desc_font),
        ..Default::default()
    });

    // ── Footer (page indicator when multi-page) ───────────────────
    let total = modal.pages.len();
    if total > 1 {
        let current = modal.current_page + 1;
        labels.push(TextLabel {
            rect: [0.0, window_h - nav_font * 3.0, window_w, nav_font * 1.5],
            text: format!("\u{25c0}  {} / {}  \u{25b6}", current, total),
            color: {
                let [r, g, b, _] = crate::render::theme::color::STONE;
                [r, g, b, 0.7 * alpha]
            },
            font_px: Some(nav_font),
            ..Default::default()
        });
    }

    let _ = scale; // kept on signature for parity with simple draw
}

/// Paginated unlock layers for full-screen showcase (no modal-queue dimmer).
pub(crate) fn modal_paginated_unlock_layer_vecs(
    modal: &Modal,
    alpha: f32,
    window_w: f32,
    window_h: f32,
) -> (
    Vec<GpuInstance>,
    Vec<TextLabel>,
    Vec<Object3d>,
    Vec<GradientQuadInstance>,
) {
    let scale = (window_w.min(window_h)) / 600.0;
    let mut instances = Vec::new();
    let mut labels = Vec::new();
    let mut relic_objects = Vec::new();
    let mut gradient_quads = Vec::new();
    draw_modal_paginated_unlock(
        modal,
        alpha,
        scale,
        (window_w, window_h),
        ModalDrawSink {
            instances: &mut instances,
            labels: &mut labels,
            relic_objects: &mut relic_objects,
            gradient_quads: &mut gradient_quads,
        },
    );
    (instances, labels, relic_objects, gradient_quads)
}
