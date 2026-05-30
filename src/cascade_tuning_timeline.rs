//! Interactive multi-track timelines for the cascade tuning debug overlay.

use crate::game::cascade::CascadeTuning;
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};

pub const TUNING_MIN_MS: u64 = 50;
pub const TUNING_MAX_MS: u64 = 5000;
pub const TUNING_SNAP_MS: u64 = 50;
/// Example step count shown on the score timeline (two beats at `step_hold_ms`).
pub const SCORE_SAMPLE_STEPS: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineDragTarget {
    Score(usize),
    Discard(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct CascadeTuningTimelineGeom {
    pub inner_x: f32,
    pub inner_w: f32,
    pub score_bar_y: f32,
    pub discard_bar_y: f32,
    pub bar_h: f32,
    pub handle_r: f32,
    pub diagram_top: f32,
    pub diagram_h: f32,
}

impl CascadeTuningTimelineGeom {
    pub fn compute(panel_x: f32, panel_w: f32, diagram_top: f32, scale: f32) -> Self {
        let diagram_h = (120.0 * scale).max(88.0);
        let diag_pad = 12.0 * scale;
        let label_col = (56.0 * scale).max(44.0);
        let inner_x = panel_x + diag_pad + label_col;
        let inner_w = panel_w - diag_pad * 2.0 - label_col;
        let bar_h = (14.0 * scale).max(10.0);
        let row_gap = (22.0 * scale).max(16.0);
        let score_bar_y = diagram_top + (18.0 * scale).max(12.0);
        let discard_bar_y = score_bar_y + bar_h + row_gap;
        let handle_r = (bar_h * 0.55).clamp(5.0, 9.0);
        Self {
            inner_x,
            inner_w,
            score_bar_y,
            discard_bar_y,
            bar_h,
            handle_r,
            diagram_top,
            diagram_h,
        }
    }

    pub fn diagram_rect(&self, panel_x: f32, panel_w: f32, diag_pad: f32) -> [f32; 4] {
        [
            panel_x + diag_pad,
            self.diagram_top,
            panel_w - diag_pad * 2.0,
            self.diagram_h,
        ]
    }

    pub fn score_boundaries_ms(tuning: &CascadeTuning) -> [u64; 4] {
        let steps = SCORE_SAMPLE_STEPS * tuning.step_hold_ms;
        [
            0,
            tuning.base_hold_ms,
            tuning.base_hold_ms + steps,
            tuning.base_hold_ms + steps + tuning.total_hold_ms,
        ]
    }

    pub fn discard_boundaries_ms(tuning: &CascadeTuning) -> [u64; 5] {
        let lift = tuning.discard_lift_ms;
        let flight = tuning.discard_flight_ms;
        let land = tuning.discard_landing_ms;
        let sink = tuning.discard_river_sink_ms;
        [0, lift, lift + flight, lift + flight + land, lift + flight + land + sink]
    }

    fn track_max(boundaries: &[u64]) -> u64 {
        (*boundaries.last().unwrap_or(&1)).max(TUNING_MIN_MS * boundaries.len() as u64)
    }

    pub fn ms_to_x(&self, ms: u64, max_ms: u64) -> f32 {
        let t = ms as f32 / max_ms.max(1) as f32;
        self.inner_x + self.inner_w * t.clamp(0.0, 1.0)
    }

    pub fn x_to_ms(&self, x: f32, max_ms: u64) -> u64 {
        let t = ((x - self.inner_x) / self.inner_w.max(1e-6)).clamp(0.0, 1.0);
        snap_ms((t * max_ms as f32).round() as u64)
    }

    pub fn hit_handle(&self, mx: f32, my: f32, tuning: &CascadeTuning) -> Option<TimelineDragTarget> {
        let hit = self.handle_r * 2.2;
        for handle in (1..=3).rev() {
            let max_ms = Self::track_max(&Self::score_boundaries_ms(tuning));
            let bx = self.ms_to_x(Self::score_boundaries_ms(tuning)[handle], max_ms);
            if (mx - bx).abs() <= hit && (my - self.score_bar_y - self.bar_h * 0.5).abs() <= hit {
                return Some(TimelineDragTarget::Score(handle));
            }
        }
        for handle in (1..=4).rev() {
            let max_ms = Self::track_max(&Self::discard_boundaries_ms(tuning));
            let bx = self.ms_to_x(Self::discard_boundaries_ms(tuning)[handle], max_ms);
            if (mx - bx).abs() <= hit && (my - self.discard_bar_y - self.bar_h * 0.5).abs() <= hit {
                return Some(TimelineDragTarget::Discard(handle));
            }
        }
        None
    }

    pub fn cursor_for_drag(target: TimelineDragTarget) -> usize {
        match target {
            TimelineDragTarget::Score(1) => 0,
            TimelineDragTarget::Score(2) => 1,
            TimelineDragTarget::Score(3) => 2,
            TimelineDragTarget::Discard(1) => 5,
            TimelineDragTarget::Discard(2) => 6,
            TimelineDragTarget::Discard(3) => 7,
            TimelineDragTarget::Discard(4) => 9,
            _ => 0,
        }
    }
}

pub fn snap_ms(ms: u64) -> u64 {
    let snapped = (ms / TUNING_SNAP_MS) * TUNING_SNAP_MS;
    snapped.clamp(TUNING_MIN_MS, TUNING_MAX_MS)
}

pub fn apply_timeline_drag(tuning: &mut CascadeTuning, target: TimelineDragTarget, mx: f32, geom: &CascadeTuningTimelineGeom) {
    match target {
        TimelineDragTarget::Score(handle) => {
            let mut b = CascadeTuningTimelineGeom::score_boundaries_ms(tuning);
            let max_ms = CascadeTuningTimelineGeom::track_max(&b);
            let new_ms = geom.x_to_ms(mx, max_ms);
            let min_gap = TUNING_MIN_MS;
            let lo = b[handle - 1] + min_gap;
            let hi = if handle + 1 < b.len() {
                b[handle + 1].saturating_sub(min_gap)
            } else {
                TUNING_MAX_MS * 4
            };
            b[handle] = new_ms.clamp(lo, hi);
            tuning.base_hold_ms = snap_ms(b[1]);
            tuning.step_hold_ms =
                snap_ms(((b[2].saturating_sub(b[1])) / SCORE_SAMPLE_STEPS).max(min_gap));
            tuning.total_hold_ms = snap_ms(b[3].saturating_sub(b[2]).max(min_gap));
        }
        TimelineDragTarget::Discard(handle) => {
            let mut b = CascadeTuningTimelineGeom::discard_boundaries_ms(tuning);
            let max_ms = CascadeTuningTimelineGeom::track_max(&b);
            let new_ms = geom.x_to_ms(mx, max_ms);
            let min_gap = TUNING_MIN_MS;
            let lo = b[handle - 1] + min_gap;
            let hi = if handle + 1 < b.len() {
                b[handle + 1].saturating_sub(min_gap)
            } else {
                TUNING_MAX_MS * 3
            };
            b[handle] = new_ms.clamp(lo, hi);
            tuning.discard_lift_ms = snap_ms(b[1]);
            tuning.discard_flight_ms = snap_ms(b[2].saturating_sub(b[1]).max(min_gap));
            tuning.discard_landing_ms = snap_ms(b[3].saturating_sub(b[2]).max(min_gap));
            tuning.discard_river_sink_ms = snap_ms(b[4].saturating_sub(b[3]).max(min_gap));
        }
    }
}

struct TrackDrawSpec {
    label: &'static str,
    bar_y: f32,
    boundaries: Vec<u64>,
    segment_colors: Vec<[f32; 4]>,
    segment_labels: Vec<&'static str>,
    event_labels: Vec<&'static str>,
    highlight_segments: Vec<usize>,
    dragging_handle: Option<usize>,
    show_tick_marks: bool,
    tick_ms: u64,
    stagger_ms: Option<u64>,
}

pub fn draw_timelines(
    geom: &CascadeTuningTimelineGeom,
    tuning: &CascadeTuning,
    panel_x: f32,
    panel_w: f32,
    scale: f32,
    focused_row: usize,
    dragging: Option<TimelineDragTarget>,
    window_h: f32,
    instances: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
) {
    let diag_pad = 12.0 * scale;
    let font_sm = typography::tier_at_most(11.0 * scale, window_h);

    instances.push(GpuInstance {
        rect: geom.diagram_rect(panel_x, panel_w, diag_pad),
        color: color::alpha(color::WALNUT_INK, 0.9),
        user: 0,
    });

    labels.push(TextLabel {
        rect: [
            panel_x + diag_pad,
            geom.diagram_top + 2.0,
            panel_w - diag_pad * 2.0,
            geom.bar_h,
        ],
        text: "Drag the handles — each dot is an event boundary".into(),
        color: color::alpha(color::STONE, 0.82),
        font_px: Some(font_sm),
        ..Default::default()
    });

    let score_drag = match dragging {
        Some(TimelineDragTarget::Score(h)) => Some(h),
        _ => None,
    };
    let score_highlight = match focused_row {
        0 => vec![0],
        1 => vec![1, 2],
        2 => vec![3],
        3 => vec![0, 1, 2, 3],
        _ => Vec::new(),
    };
    let score_b = CascadeTuningTimelineGeom::score_boundaries_ms(tuning);
    draw_track(
        geom,
        TrackDrawSpec {
            label: "Score",
            bar_y: geom.score_bar_y,
            boundaries: score_b.to_vec(),
            segment_colors: vec![
                color::alpha(color::WALNUT_BRIGHT, 0.92),
                color::alpha(color::JADE, 0.88),
                color::alpha(color::WALNUT_SOFT, 0.88),
                color::alpha(color::GOLD, 0.90),
            ],
            segment_labels: vec!["Base", "Step", "Step", "Total"],
            event_labels: vec!["Start", "Base", "Steps", "Done"],
            highlight_segments: score_highlight,
            dragging_handle: score_drag,
            show_tick_marks: focused_row == 3,
            tick_ms: tuning.tick_duration_ms,
            stagger_ms: None,
        },
        scale,
        window_h,
        panel_x,
        diag_pad,
        instances,
        labels,
    );

    let discard_drag = match dragging {
        Some(TimelineDragTarget::Discard(h)) => Some(h),
        _ => None,
    };
    let discard_highlight = match focused_row {
        5 => vec![0],
        6 => vec![1],
        7 => vec![2],
        8 => vec![2],
        9 => vec![3],
        _ => Vec::new(),
    };
    let discard_b = CascadeTuningTimelineGeom::discard_boundaries_ms(tuning);
    draw_track(
        geom,
        TrackDrawSpec {
            label: "Discard",
            bar_y: geom.discard_bar_y,
            boundaries: discard_b.to_vec(),
            segment_colors: vec![
                color::alpha(color::ANTIQUE, 0.85),
                color::alpha(color::JADE, 0.75),
                color::alpha(color::WALNUT_BRIGHT, 0.82),
                color::alpha(color::UMBER, 0.80),
            ],
            segment_labels: vec!["Lift", "Fly", "Land", "Sink"],
            event_labels: vec!["", "Lift", "Fly", "Land", "Sink"],
            highlight_segments: discard_highlight,
            dragging_handle: discard_drag,
            show_tick_marks: false,
            tick_ms: 0,
            stagger_ms: Some(tuning.discard_stagger_ms),
        },
        scale,
        window_h,
        panel_x,
        diag_pad,
        instances,
        labels,
    );

    labels.push(TextLabel {
        rect: [
            geom.inner_x,
            geom.discard_bar_y + geom.bar_h + 4.0 * scale,
            geom.inner_w,
            14.0 * scale,
        ],
        text: format!(
            "Score counter ticks every {}ms per beat",
            tuning.tick_duration_ms
        ),
        color: color::alpha(color::STONE, 0.65),
        font_px: Some(font_sm),
        align: TextAlign::Left,
        ..Default::default()
    });
}

fn draw_track(
    geom: &CascadeTuningTimelineGeom,
    spec: TrackDrawSpec,
    scale: f32,
    window_h: f32,
    panel_x: f32,
    diag_pad: f32,
    instances: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
) {
    let font_sm = typography::tier_at_most(10.0 * scale, window_h);
    let label_x = panel_x + diag_pad;
    let label_w = geom.inner_x - label_x - 4.0 * scale;

    labels.push(TextLabel {
        rect: [label_x, spec.bar_y, label_w, geom.bar_h],
        text: spec.label.into(),
        color: color::alpha(color::CHAMPAGNE, 0.85),
        font_px: Some(font_sm),
        align: TextAlign::Right,
        ..Default::default()
    });

    let max_ms = CascadeTuningTimelineGeom::track_max(&spec.boundaries);
    let seg_count = spec.boundaries.len().saturating_sub(1);
    let mut seg_x = geom.inner_x;

    for i in 0..seg_count {
        let ms = spec.boundaries[i + 1].saturating_sub(spec.boundaries[i]);
        let seg_w = geom.inner_w * (ms as f32 / max_ms as f32);
        if seg_w < 0.5 {
            continue;
        }
        let seg_color = spec.segment_colors.get(i).copied().unwrap_or(color::WALNUT_SOFT);
        let highlighted = spec.highlight_segments.contains(&i);
        let fill = if highlighted {
            color::lighten(seg_color, 0.12)
        } else {
            seg_color
        };
        instances.push(GpuInstance {
            rect: [seg_x, spec.bar_y, seg_w, geom.bar_h],
            color: fill,
            user: 0,
        });
        if highlighted {
            instances.push(GpuInstance {
                rect: [seg_x, spec.bar_y - 1.5, seg_w, 1.5],
                color: color::alpha(color::GOLD, 0.85),
                user: 0,
            });
        }
        if seg_w > 24.0
            && let Some(lbl) = spec.segment_labels.get(i) {
                labels.push(TextLabel {
                    rect: [seg_x, spec.bar_y, seg_w, geom.bar_h],
                    text: (*lbl).into(),
                    color: color::alpha(color::WALNUT_INK, 0.88),
                    font_px: Some(font_sm),
                    align: TextAlign::Center,
                    ..Default::default()
                });
            }
        if spec.show_tick_marks && ms > 0 {
            let tick_w = 1.0;
            let step_px = geom.inner_w * (spec.tick_ms as f32 / max_ms as f32);
            if step_px > 3.0 {
                let mut tx = seg_x + step_px;
                while tx < seg_x + seg_w - 1.0 {
                    instances.push(GpuInstance {
                        rect: [tx, spec.bar_y, tick_w, geom.bar_h],
                        color: color::alpha(color::PARCHMENT, 0.35),
                        user: 0,
                    });
                    tx += step_px;
                }
            }
        }
        seg_x += seg_w;
    }

    if let Some(stagger) = spec.stagger_ms.filter(|&s| s > 0 && spec.label == "Discard") {
        let land_x = geom.ms_to_x(spec.boundaries[3], max_ms);
        let fuzz_w = (geom.inner_w * (stagger as f32 / max_ms as f32)).clamp(4.0, 28.0);
        instances.push(GpuInstance {
            rect: [land_x - fuzz_w * 0.3, spec.bar_y + geom.bar_h * 0.15, fuzz_w, geom.bar_h * 0.7],
            color: color::alpha(color::PARCHMENT, 0.22),
            user: 0,
        });
        labels.push(TextLabel {
            rect: [land_x, spec.bar_y + geom.bar_h + 1.0, fuzz_w + 20.0, 10.0 * scale],
            text: format!("±{stagger}ms stagger"),
            color: color::alpha(color::STONE, 0.55),
            font_px: Some(font_sm * 0.9),
            ..Default::default()
        });
    }

    for (i, &ms) in spec.boundaries.iter().enumerate() {
        let hx = geom.ms_to_x(ms, max_ms);
        let is_dragging = spec.dragging_handle == Some(i);
        let handle_d = geom.handle_r * if is_dragging { 2.4 } else { 2.0 };
        let handle_color = if is_dragging {
            color::CHAMPAGNE
        } else if i > 0 && i <= seg_count {
            color::PARCHMENT
        } else {
            color::alpha(color::STONE, 0.7)
        };
        instances.push(GpuInstance {
            rect: [hx - handle_d * 0.5, spec.bar_y + geom.bar_h * 0.5 - handle_d * 0.5, handle_d, handle_d],
            color: handle_color,
            user: 0,
        });
        instances.push(GpuInstance {
            rect: [
                hx - handle_d * 0.35,
                spec.bar_y + geom.bar_h * 0.5 - handle_d * 0.35,
                handle_d * 0.7,
                handle_d * 0.7,
            ],
            color: if is_dragging {
                color::alpha(color::GOLD, 0.95)
            } else {
                color::alpha(color::WALNUT_INK, 0.55)
            },
            user: 0,
        });

        if let Some(ev) = spec.event_labels.get(i)
            && !ev.is_empty() {
                labels.push(TextLabel {
                    rect: [hx - 28.0, spec.bar_y - 12.0 * scale, 56.0, 11.0 * scale],
                    text: (*ev).into(),
                    color: color::alpha(color::STONE, 0.78),
                    font_px: Some(font_sm * 0.92),
                    align: TextAlign::Center,
                    ..Default::default()
                });
            }
        if i > 0 {
            labels.push(TextLabel {
                rect: [hx - 24.0, spec.bar_y + geom.bar_h + 2.0, 48.0, 10.0 * scale],
                text: format!("{ms}"),
                color: color::alpha(color::STONE, 0.62),
                font_px: Some(font_sm * 0.88),
                align: TextAlign::Center,
                ..Default::default()
            });
        }
    }

    if spec.label == "Score" && SCORE_SAMPLE_STEPS > 1 {
        let b = &spec.boundaries;
        if b.len() >= 3 {
            let step_ms = (b[2] - b[1]) / SCORE_SAMPLE_STEPS;
            for s in 1..SCORE_SAMPLE_STEPS {
                let ms = b[1] + step_ms * s;
                let mx = geom.ms_to_x(ms, max_ms);
                instances.push(GpuInstance {
                    rect: [mx - 0.5, spec.bar_y - 2.0, 1.0, geom.bar_h + 4.0],
                    color: color::alpha(color::PARCHMENT, 0.45),
                    user: 0,
                });
            }
        }
    }
}
