//! Odometer-style floating score reel.
//!
//! Displays the current score as a row of 7 digit columns (expandable if the
//! score overflows). Each column shows a single digit glyph; when the digit
//! changes the column spins upward through 0–9 like an odometer reel, with
//! only the columns whose digit actually changed animating. Higher-order
//! columns that carry-over spin with a brief cascade delay so the roll reads
//! right-to-left like a mechanical counter.
//!
//! The reel is positioned in 3D using the same `(pixel_x, pixel_y, lift)`
//! convention as every other placement in the renderer. The caller passes the
//! plaque's pixel anchor and lift each frame; the reel tiles its columns
//! horizontally around that center.
//!
//! ## Slot budget
//!
//! Each column renders three glyph meshes stacked vertically (prev, current,
//! next) so that the spin transition clips naturally. The column count starts
//! at the target score's digit width (set via `reset_for_target`) and only
//! grows if the player's score overflows; it is reset back down at round
//! start. The caller is responsible for ensuring `MAX_EXTRUDED_GLYPH_SLOTS`
//! is large enough for the worst-case column count.

use std::time::Instant;

use crate::render::draw_cmd::{GlyphMaterial, Object3d, Object3dKind};
use crate::render::world_space::{LayoutAnchorPx, PlacementAnchor};

// ── Tuning constants ──────────────────────────────────────────────────────

/// World-unit width of one digit column (controls spacing between digits).
const COLUMN_WIDTH: f32 = 95.0;

/// World-unit height of one digit slot (the vertical travel distance for one
/// reel step). Should match the visual cap-height of the glyph at `DIGIT_SCALE`.
const SLOT_HEIGHT: f32 = 165.0;

/// Uniform scale applied to every digit glyph. The glyph meshes are
/// normalised to ~1.0 height in font space; this scales them to world units.
const DIGIT_SCALE: f32 = 142.0;

/// Duration of one digit spin in seconds.
const SPIN_DURATION: f32 = 0.22;

/// Per-column cascade delay (seconds). Column 0 is the ones place; each
/// higher-order column starts its spin this many seconds later.
const CASCADE_DELAY: f32 = 0.04;

/// Spring overshoot: the reel momentarily shows `(digit + 1) % 10` at the
/// apex before settling back. Expressed as a fraction of `SLOT_HEIGHT`.
const OVERSHOOT: f32 = 0.18;

/// Colour of all digit glyphs (warm gold matching the existing score style).
const DIGIT_COLOR: [f32; 4] = [1.00, 0.91, 0.66, 1.0];

/// Emissive boost while a column is actively spinning.
const EMISSIVE_SPIN: f32 = 0.9;
/// Emissive level when idle.
const EMISSIVE_IDLE: f32 = 0.25;

/// Pitch (radians) passed as `rotation_x` through to the extruded-glyph
/// pipeline. The renderer applies `-π/2 + rotation_x` to the mesh, which
/// already makes the glyphs stand upright facing the camera (that's how
/// score popups read). Pass 0 — or a small value for a forward lean — to
/// keep the reel digits on the same upright plane.
const PITCH: f32 = 0.0;

// ── Internal state ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct ReelColumn {
    /// The digit currently displayed (0–9).
    current: u8,
    /// The digit being rolled away from (source of the spin).
    prev: u8,
    /// When this column's spin started. `None` = idle.
    spin_start: Option<Instant>,
}

impl ReelColumn {
    fn new(digit: u8) -> Self {
        Self {
            current: digit,
            prev: digit,
            spin_start: None,
        }
    }

    /// Trigger a spin to `new_digit`. No-op if already at that value.
    fn spin_to(&mut self, new_digit: u8, start: Instant) {
        if new_digit == self.current {
            return;
        }
        self.prev = self.current;
        self.current = new_digit;
        self.spin_start = Some(start);
    }

    /// Normalised spin progress in [0, 1]. Returns 0 when idle.
    fn progress(&self, now: Instant) -> f32 {
        match self.spin_start {
            None => 1.0,
            Some(t) => {
                let elapsed = now.saturating_duration_since(t).as_secs_f32();
                (elapsed / SPIN_DURATION).clamp(0.0, 1.0)
            }
        }
    }

    fn is_spinning(&self, now: Instant) -> bool {
        self.spin_start
            .map(|t| now.saturating_duration_since(t).as_secs_f32() < SPIN_DURATION)
            .unwrap_or(false)
    }
}

// ── Public API ────────────────────────────────────────────────────────────

pub struct ScoreReel {
    columns: Vec<ReelColumn>,
    /// The score value the reel currently represents.
    displayed: u64,
    /// Minimum column count for the current round — equals the target score's
    /// digit width. The reel never shrinks below this mid-round; it grows
    /// beyond it only if the player's score overflows.
    min_columns: usize,
}

impl ScoreReel {
    pub fn new() -> Self {
        // Default to a single zero until a round sets its target.
        Self {
            columns: vec![ReelColumn::new(0)],
            displayed: 0,
            min_columns: 1,
        }
    }

    /// Reset the reel for a new round: shrink back to the target score's
    /// digit width and zero every column. Call at round start so the reel
    /// only shows as many zeros as the target requires; extra columns added
    /// mid-round (when the player overshot) are dropped.
    pub fn reset_for_target(&mut self, target: u64) {
        let n = digits_for(target).max(1);
        self.min_columns = n;
        self.columns = (0..n).map(|_| ReelColumn::new(0)).collect();
        self.displayed = 0;
    }

    /// Drive the reel to show `score`. Changed digit columns spin with a
    /// right-to-left cascade delay so carries look mechanical.
    ///
    /// `now` is the current frame timestamp (used to stagger spin starts).
    pub fn set_score(&mut self, score: u64, now: Instant) {
        // Grow columns if the score overflows the current width. We never
        // shrink here — columns added after overshoot persist until the next
        // `reset_for_target` call.
        let needed = digits_for(score).max(self.min_columns);
        if needed > self.columns.len() {
            let extra = needed - self.columns.len();
            for _ in 0..extra {
                self.columns.push(ReelColumn::new(0));
            }
        }

        if score == self.displayed {
            return;
        }

        // Decompose both old and new score into per-column digits (ones first).
        let new_digits = decompose(score, self.columns.len());
        let _old_digits = decompose(self.displayed, self.columns.len());

        // Spin only the columns whose digit changed. Higher-order columns that
        // carry get a small additional delay so the roll cascades right-to-left.
        let mut carry_delay = 0.0_f32;
        for (column, &new_d) in self.columns.iter_mut().zip(new_digits.iter()) {
            if new_d != column.current {
                let spin_at = now + std::time::Duration::from_secs_f32(carry_delay);
                column.spin_to(new_d, spin_at);
                carry_delay += CASCADE_DELAY;
            }
        }

        self.displayed = score;
    }

    /// Instantly snap all columns to `score` with no spin animation.
    /// Use this on scene init or round start to avoid a spurious roll from 0.
    /// No-op if already displaying `score`. Column count floor is the current
    /// `min_columns` (set via `reset_for_target`) so snapping to a mid-round
    /// score keeps any overflow columns already shown.
    pub fn snap(&mut self, score: u64) {
        if score == self.displayed && !self.columns.is_empty() {
            return;
        }
        let n = digits_for(score).max(self.min_columns).max(1);
        let digits = decompose(score, n);
        self.columns = digits.iter().map(|&d| ReelColumn::new(d)).collect();
        self.displayed = score;
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.columns.iter().any(|c| c.is_spinning(now))
    }

    /// Build the per-frame `ExtrudedGlyphPlacement` list.
    ///
    /// `placement` — pixel-space anchor (matches the plaque center so the
    /// reel floats in front of it), world-Z lift (same as plaque lift, bump
    /// slightly forward), yaw inherited from the plaque's camera-facing
    /// rotation, and uniform scale.
    /// `target` — optional target score rendered as a "/ N" tail.
    pub fn placements(
        &self,
        now: Instant,
        placement: PlacementAnchor,
        target: Option<u64>,
    ) -> Vec<Object3d> {
        let PlacementAnchor {
            anchor:
                LayoutAnchorPx {
                    px: anchor_px,
                    py: anchor_py,
                    lift_z: lift,
                },
            rot_y,
            scale,
        } = placement;
        let col_w = COLUMN_WIDTH * scale;
        let slot_h = SLOT_HEIGHT * scale;
        let digit_scale = DIGIT_SCALE * scale;
        // The glyph's scale differs from the overall reel scale — bake it into
        // the per-glyph placement so `make_placement` only sees one scale.
        let digit_placement = PlacementAnchor {
            anchor: LayoutAnchorPx {
                px: anchor_px,
                py: anchor_py,
                lift_z: lift,
            },
            rot_y,
            scale: digit_scale,
        };
        let n = self.columns.len();
        // Static "score:" prefix rendered as dimmer glyphs to the left of
        // the digit columns so the reel reads as "score: N / target".
        let prefix: Vec<char> = "score:".chars().collect();
        // Target tail ("/ N") rendered as dimmer glyphs immediately to the
        // right of the digit columns so the reel reads as "score / target".
        // Each tail char occupies roughly one column-width so the spacing
        // matches the reel.
        let tail: Vec<char> = target
            .map(|t| format!("/{t}").chars().collect())
            .unwrap_or_default();
        let total_cols = prefix.len() + n + tail.len();
        // Total reel+tail width in world units; center it on the anchor.
        let total_w = total_cols as f32 * col_w;
        // Columns are ordered ones-first internally; display most-significant on left.
        let mut out = Vec::with_capacity(prefix.len() + n * 3 + tail.len());

        for col in 0..n {
            // Display index: prefix slots come first, then most-significant
            // digit, then the rest of the digits, then the tail.
            let display_idx = prefix.len() + (n - 1 - col);
            let col_center_x = anchor_px + (-total_w * 0.5 + (display_idx as f32 + 0.5) * col_w);

            let c = &self.columns[col];
            let t = c.progress(now);
            let spinning = c.is_spinning(now);

            // Spring easing: overshoot then settle.
            let y_offset = if t < 1.0 {
                let eased = spring_ease(t);
                eased * slot_h
            } else {
                0.0
            };

            let emissive = if spinning {
                EMISSIVE_SPIN
            } else {
                EMISSIVE_IDLE
            };
            let mut color = DIGIT_COLOR;

            // Fade leading zeros that are above the most-significant non-zero
            // column to a dimmer alpha so they read as placeholders. The
            // ones-place (col 0) always renders at full alpha so a score of
            // zero still shows a readable "0" rather than fading to nothing.
            let is_leading_zero = col > 0
                && c.current == 0
                && self.columns[col + 1..].iter().all(|cc| cc.current == 0);
            if is_leading_zero {
                color[3] = 0.25;
            }

            if spinning {
                // During spin: show prev digit scrolling out (upward) and
                // current digit rolling in from below. A third "next" ghost
                // (current+1 wrapped) peeks in from below for visual continuity.
                let prev_label = digit_label(c.prev);
                let cur_label = digit_label(c.current);

                // prev moves up (+Z) and out: starts at 0, exits at +SLOT_HEIGHT
                let prev_z = y_offset;
                // current rolls in from below (-Z): starts at -SLOT_HEIGHT, lands at 0
                let cur_z = y_offset - slot_h;

                out.push(make_placement(
                    prev_label,
                    col_center_x,
                    prev_z,
                    digit_placement,
                    color,
                    emissive,
                ));
                out.push(make_placement(
                    cur_label,
                    col_center_x,
                    cur_z,
                    digit_placement,
                    color,
                    emissive,
                ));
            } else {
                // Idle: just the current digit, centred.
                let cur_label = digit_label(c.current);
                out.push(make_placement(
                    cur_label,
                    col_center_x,
                    0.0,
                    digit_placement,
                    color,
                    emissive,
                ));
            }
        }

        // Prefix: "score:" rendered before the digit columns, dimmed so the
        // digits themselves remain the primary read.
        let dim_color = [DIGIT_COLOR[0], DIGIT_COLOR[1], DIGIT_COLOR[2], 0.55];
        for (i, ch) in prefix.iter().enumerate() {
            let col_center_x = anchor_px + (-total_w * 0.5 + (i as f32 + 0.5) * col_w);
            out.push(make_placement(
                &ch.to_string(),
                col_center_x,
                0.0,
                digit_placement,
                dim_color,
                EMISSIVE_IDLE,
            ));
        }

        // Tail: "/ N" target glyphs to the right of the digit columns,
        // dimmed so the reel reads as the primary number and the target
        // reads as a secondary reference.
        for (i, ch) in tail.iter().enumerate() {
            let display_idx = prefix.len() + n + i;
            let col_center_x = anchor_px + (-total_w * 0.5 + (display_idx as f32 + 0.5) * col_w);
            out.push(make_placement(
                &ch.to_string(),
                col_center_x,
                0.0,
                digit_placement,
                dim_color,
                EMISSIVE_IDLE,
            ));
        }

        out
    }
}

impl Default for ScoreReel {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Decompose `score` into per-column digits, ones first, padded to `n` columns.
fn decompose(score: u64, n: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(n);
    let mut s = score;
    for _ in 0..n {
        v.push((s % 10) as u8);
        s /= 10;
    }
    v
}

/// Number of decimal digits needed to represent `score` (minimum 1).
fn digits_for(score: u64) -> usize {
    if score == 0 {
        1
    } else {
        score.ilog10() as usize + 1
    }
}

fn digit_label(d: u8) -> &'static str {
    match d {
        0 => "0",
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        6 => "6",
        7 => "7",
        8 => "8",
        9 => "9",
        _ => "0",
    }
}

/// Spring easing with overshoot. `t` in [0, 1], output in [0, 1] with a brief
/// excursion above 1.0 near `t ≈ 0.7` to simulate mechanical momentum.
fn spring_ease(t: f32) -> f32 {
    // Critically-damped-ish spring approximation using a sine envelope.
    // Peaks slightly above 1.0 at t≈0.65, then settles to 1.0 at t=1.0.
    let base = 1.0 - (1.0 - t).powi(3); // smooth approach
    let overshoot = OVERSHOOT * (t * std::f32::consts::PI).sin() * (1.0 - t);
    (base + overshoot).min(1.0 + OVERSHOOT)
}

/// Build one `ExtrudedGlyphPlacement` for a single digit.
///
/// `z_offset` — vertical offset in world Z units (+Z = up). Used during spin
/// to move the two travelling glyphs above/below the column anchor.
#[inline]
fn make_placement(
    label: &str,
    col_px: f32,
    z_offset: f32,
    placement: PlacementAnchor,
    color: [f32; 4],
    emissive: f32,
) -> Object3d {
    let PlacementAnchor {
        anchor:
            LayoutAnchorPx {
                py: anchor_py,
                lift_z: lift,
                ..
            },
        rot_y,
        scale: digit_scale,
    } = placement;
    Object3d {
        pos: [col_px, anchor_py, lift + z_offset],
        extents: [1.0, 1.0, 1.0],
        rotation: [0.0, 0.0, 0.0],
        color,
        kind: Object3dKind::ExtrudedGlyph {
            scale: digit_scale,
            rotation_x: PITCH,
            rotation_y: rot_y,
            label: std::sync::Arc::from(label),
            emissive,
            material: GlyphMaterial::Plain,
        },
        hover_target: 0.0,
        anim_id: 0,
        arrange_name: None,
    }
}
