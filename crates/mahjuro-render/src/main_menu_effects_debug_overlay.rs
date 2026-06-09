//! Multi-tab debug panel for main-menu hub effects (moon, rain, moths).

use glam::Vec3;
use sdl3::keyboard::Scancode;

use crate::debug_overlay_ui::{self, DebugPointerState, DebugRowVisual};
use crate::draw_cmd::CameraParams;
use crate::main_menu_effects_tuning::MainMenuEffectsTuning;
use crate::main_menu_moon_tuning::{
    MOON_DEBUG_ROW_META, MOON_DEBUG_SLIDER_COUNT, MainMenuMoonPhaseDebug, moon_row_is_hue,
    moon_row_is_phase, moon_row_is_saturation,
};
use crate::main_menu_moth_tuning::{MOTH_DEBUG_ROW_META, MOTH_DEBUG_SLIDER_COUNT};
use crate::particles::RainSpawnVolume;
use crate::rain_field::main_menu_rain_spawn_volume;
use crate::rain_tuning::{
    RAIN_DEBUG_ROW_META, RAIN_DEBUG_SLIDER_COUNT, RainTuning, rain_color_swatch_rgb,
    rain_hue_wheel_preview_linear, rain_row_is_hue, rain_row_is_saturation,
};
use crate::theme::{ButtonVariant, color, metrics, typography};
use crate::wgpu_renderer::moon_phase_short_name;
use crate::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use mahjuro_types::UiAction;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainMenuEffectsTab {
    Moon = 0,
    Rain = 1,
    Moths = 2,
}

const TAB_COUNT: usize = 3;
const TAB_LABELS: [&str; TAB_COUNT] = ["Moon", "Rain", "Moths"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActionRow {
    MoonPhaseLive,
    MoonPhaseNew,
    MoonPhaseFirstQuarter,
    MoonPhaseFull,
    MoonPhaseLastQuarter,
    PrideRainbow,
    ShowRainHit,
    ShowRainDepth,
    HideUi,
    Save,
    Reset,
    Close,
}

fn tab_slider_meta(tab: MainMenuEffectsTab) -> &'static [(&'static str, f32, f32, f32)] {
    match tab {
        MainMenuEffectsTab::Moon => MOON_DEBUG_ROW_META,
        MainMenuEffectsTab::Rain => RAIN_DEBUG_ROW_META,
        MainMenuEffectsTab::Moths => MOTH_DEBUG_ROW_META,
    }
}

fn tab_slider_count(tab: MainMenuEffectsTab) -> usize {
    match tab {
        MainMenuEffectsTab::Moon => MOON_DEBUG_SLIDER_COUNT,
        MainMenuEffectsTab::Rain => RAIN_DEBUG_SLIDER_COUNT,
        MainMenuEffectsTab::Moths => MOTH_DEBUG_SLIDER_COUNT,
    }
}

const FOOTER_ACTION_ROWS: [ActionRow; 3] = [ActionRow::Save, ActionRow::Reset, ActionRow::Close];

/// Scrollable action buttons (Save / Reset / Close are pinned in the panel footer).
fn tab_mid_action_rows(tab: MainMenuEffectsTab) -> &'static [ActionRow] {
    match tab {
        MainMenuEffectsTab::Moon => &[
            ActionRow::MoonPhaseLive,
            ActionRow::MoonPhaseNew,
            ActionRow::MoonPhaseFirstQuarter,
            ActionRow::MoonPhaseFull,
            ActionRow::MoonPhaseLastQuarter,
            ActionRow::PrideRainbow,
        ],
        MainMenuEffectsTab::Rain => &[
            ActionRow::ShowRainHit,
            ActionRow::ShowRainDepth,
            ActionRow::HideUi,
        ],
        MainMenuEffectsTab::Moths => &[],
    }
}

fn tab_scroll_item_count(tab: MainMenuEffectsTab) -> usize {
    tab_slider_count(tab) + tab_mid_action_rows(tab).len()
}

fn tab_row_count(tab: MainMenuEffectsTab) -> usize {
    tab_scroll_item_count(tab) + FOOTER_ACTION_ROWS.len()
}

fn tab_footer_row_index(tab: MainMenuEffectsTab, footer_idx: usize) -> usize {
    tab_scroll_item_count(tab) + footer_idx
}

fn row_is_hue(tab: MainMenuEffectsTab, row: usize) -> bool {
    match tab {
        MainMenuEffectsTab::Moon => moon_row_is_hue(row),
        MainMenuEffectsTab::Rain => rain_row_is_hue(row),
        MainMenuEffectsTab::Moths => false,
    }
}

fn row_is_saturation(tab: MainMenuEffectsTab, row: usize) -> bool {
    match tab {
        MainMenuEffectsTab::Moon => moon_row_is_saturation(row),
        MainMenuEffectsTab::Rain => rain_row_is_saturation(row),
        MainMenuEffectsTab::Moths => false,
    }
}

fn row_color_swatch(
    tuning: &MainMenuEffectsTuning,
    tab: MainMenuEffectsTab,
    row: usize,
) -> Option<[f32; 3]> {
    match tab {
        MainMenuEffectsTab::Moon => tuning.moon.color_swatch_rgb(row),
        MainMenuEffectsTab::Rain => rain_color_swatch_rgb(&tuning.rain, row),
        MainMenuEffectsTab::Moths => None,
    }
}

const SPAWN_FIELD_COLS: usize = 26;
const SPAWN_FIELD_ROWS: usize = 18;
/// Upper bound when computing how many rows fit on screen.
const MAX_VISIBLE_ROWS: usize = 40;

fn rain_view_forward(cam: &CameraParams) -> Vec3 {
    let eye = Vec3::from_array(cam.eye);
    let target = Vec3::from_array(cam.target);
    (target - eye).normalize_or_zero()
}

fn rain_view_right(cam: &CameraParams) -> Vec3 {
    let forward = rain_view_forward(cam);
    let up = Vec3::from_array(cam.up);
    forward.cross(up).normalize_or_zero()
}

fn push_quad(instances: &mut Vec<GpuInstance>, x: f32, y: f32, w: f32, h: f32, rgba: [f32; 4]) {
    instances.push(GpuInstance {
        rect: [x, y, w, h],
        color: rgba,
        user: 0,
    });
}

/// Target reticle plus a horizontal-FOV wedge in the lateral / view-depth plane.
fn push_camera_marker(
    instances: &mut Vec<GpuInstance>,
    cx: f32,
    cy: f32,
    arm: f32,
    scale: f32,
    fovy_deg: f32,
    aspect: f32,
    reach: f32,
    draw_fill: bool,
) {
    let half_w = reach * (fovy_deg.to_radians() * 0.5).tan() * aspect;
    let top_y = cy - reach;
    let edge_t = (1.8 * scale).max(1.2);
    let fill_color = color::alpha(color::CHAMPAGNE, 0.10);
    let edge_color = color::alpha(color::CHAMPAGNE, 0.72);

    if draw_fill {
        const SLICES: usize = 6;
        for i in 0..SLICES {
            let t0 = i as f32 / SLICES as f32;
            let t1 = (i + 1) as f32 / SLICES as f32;
            let y_near = cy - reach * t0;
            let y_far = cy - reach * t1;
            let w = half_w * (t0 + t1);
            push_quad(
                instances,
                cx - w,
                y_far,
                w * 2.0,
                y_near - y_far,
                fill_color,
            );
        }
    }

    const EDGE_STEPS: usize = 10;
    for (ex, ey) in [(cx - half_w, top_y), (cx + half_w, top_y)] {
        for i in 0..EDGE_STEPS {
            let t1 = (i + 1) as f32 / EDGE_STEPS as f32;
            let px = cx + (ex - cx) * t1;
            let py = cy + (ey - cy) * t1;
            push_quad(
                instances,
                px - edge_t * 0.5,
                py - edge_t * 0.5,
                edge_t,
                edge_t,
                edge_color,
            );
        }
    }
    push_quad(
        instances,
        cx - half_w,
        top_y - edge_t * 0.5,
        half_w * 2.0,
        edge_t,
        edge_color,
    );

    let ring = (arm * 1.35).max(7.0 * scale);
    let bar_t = (2.4 * scale).max(1.8);
    let bar_l = (arm * 0.72).max(4.0 * scale);
    push_quad(
        instances,
        cx - ring * 0.5,
        cy - bar_t * 0.5,
        ring,
        bar_t,
        color::alpha(color::CHAMPAGNE, 0.35),
    );
    push_quad(
        instances,
        cx - bar_t * 0.5,
        cy - ring * 0.5,
        bar_t,
        ring,
        color::alpha(color::CHAMPAGNE, 0.35),
    );
    push_quad(
        instances,
        cx - bar_l * 0.5,
        cy - bar_t * 0.5,
        bar_l,
        bar_t,
        color::PARCHMENT,
    );
    push_quad(
        instances,
        cx - bar_t * 0.5,
        cy - bar_l * 0.5,
        bar_t,
        bar_l,
        color::PARCHMENT,
    );
    let dot = (3.2 * scale).max(2.0);
    push_quad(
        instances,
        cx - dot * 0.5,
        cy - dot * 0.5,
        dot,
        dot,
        color::CHAMPAGNE,
    );
}

/// Block-style arrow; `dir` is diagram-plane direction (x = lateral, y = −view depth).
fn push_wind_arrow_icon(
    instances: &mut Vec<GpuInstance>,
    tail_x: f32,
    tail_y: f32,
    dir_x: f32,
    dir_y: f32,
    len: f32,
    scale: f32,
    rgba: [f32; 4],
) {
    let dir_len = (dir_x * dir_x + dir_y * dir_y).sqrt();
    if dir_len <= 1e-4 {
        return;
    }
    let dx = dir_x / dir_len;
    let dy = dir_y / dir_len;
    let px = -dy;
    let py = dx;
    let shaft_len = len * 0.68;
    let head_len = len * 0.32;
    let head_w = (5.5 * scale).max(3.5);
    let shaft_w = (2.4 * scale).max(1.6);
    const SHAFT_STEPS: usize = 8;
    for i in 0..SHAFT_STEPS {
        let t = i as f32 / (SHAFT_STEPS - 1) as f32;
        if t > 0.92 {
            continue;
        }
        let cx = tail_x + dx * shaft_len * t;
        let cy = tail_y + dy * shaft_len * t;
        push_quad(
            instances,
            cx - shaft_w * 0.5,
            cy - shaft_w * 0.5,
            shaft_w,
            shaft_w,
            rgba,
        );
    }
    let tip_x = tail_x + dx * len;
    let tip_y = tail_y + dy * len;
    let head_back_x = tip_x - dx * head_len;
    let head_back_y = tip_y - dy * head_len;
    push_quad(
        instances,
        tip_x - head_w * 0.5,
        tip_y - head_w * 0.5,
        head_w,
        head_w,
        rgba,
    );
    push_quad(
        instances,
        head_back_x + px * head_w * 0.55 - head_w * 0.35,
        head_back_y + py * head_w * 0.55 - head_w * 0.35,
        head_w * 0.7,
        head_w * 0.7,
        rgba,
    );
    push_quad(
        instances,
        head_back_x - px * head_w * 0.55 - head_w * 0.35,
        head_back_y - py * head_w * 0.55 - head_w * 0.35,
        head_w * 0.7,
        head_w * 0.7,
        rgba,
    );
}

fn push_legend_swatch_row(
    instances: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
    x: f32,
    y: f32,
    row_h: f32,
    swatch_w: f32,
    label_w: f32,
    scale: f32,
    window_h: f32,
    caption: &str,
    draw_swatch: impl FnOnce(&mut Vec<GpuInstance>, f32, f32, f32),
) {
    let font = typography::tier_at_most((10.0 * scale).max(8.0), window_h);
    let cy = y + row_h * 0.5;
    draw_swatch(instances, x + swatch_w * 0.5, cy, scale);
    labels.push(TextLabel {
        rect: [
            x + swatch_w + 4.0 * scale,
            y,
            label_w - swatch_w - 4.0 * scale,
            row_h,
        ],
        text: caption.into(),
        font_px: Some(font),
        color: color::alpha(color::PARCHMENT, 0.82),
        align: TextAlign::Left,
        ..Default::default()
    });
}

/// Camera-centric dot field: lateral (→) vs view depth (near ↓ far ↑), dot α = spawn weight.
fn draw_spawn_field_diagram(
    cam: &CameraParams,
    volume: RainSpawnVolume,
    tuning: &RainTuning,
    window_w: f32,
    window_h: f32,
    scale: f32,
    x0: f32,
    instances: &mut Vec<GpuInstance>,
    labels: &mut Vec<TextLabel>,
) {
    let margin = (10.0 * scale).max(6.0);
    let diagram_w = (320.0 * scale).min(window_w * 0.38);
    let legend_h = (74.0 * scale).max(52.0);
    let plot_area_h = (200.0 * scale).min(window_h * 0.34);
    let diagram_h = plot_area_h + legend_h;
    let y0 = margin;
    let border = 2.0;
    let pad = (6.0 * scale).max(4.0);
    let title_h = (14.0 * scale).max(10.0);
    let plot_x = x0 + pad;
    let plot_y = y0 + pad + title_h;
    let plot_w = diagram_w - pad * 2.0;
    let plot_h = plot_area_h - pad - title_h;

    instances.push(GpuInstance {
        rect: [
            x0 - border,
            y0 - border,
            diagram_w + border * 2.0,
            diagram_h + border * 2.0,
        ],
        color: color::alpha(color::WALNUT_SOFT, 0.85),
        user: 0,
    });
    instances.push(GpuInstance {
        rect: [x0, y0, diagram_w, diagram_h],
        color: color::alpha(color::WALNUT_INK, 0.94),
        user: 0,
    });

    let title_font = typography::tier_at_most(title_h * 0.92, window_h);
    labels.push(TextLabel {
        rect: [x0 + pad, y0 + pad * 0.5, diagram_w - pad * 2.0, title_h],
        text: "Spawn weight (camera view)".into(),
        font_px: Some(title_font),
        color: color::JADE,
        align: TextAlign::Left,
        ..Default::default()
    });

    instances.push(GpuInstance {
        rect: [plot_x, plot_y, plot_w, plot_h],
        color: color::alpha(color::WALNUT_DEEP, 0.92),
        user: 0,
    });

    // Plot border + corner axis tags.
    let frame_t = (1.5 * scale).max(1.0);
    push_quad(
        instances,
        plot_x,
        plot_y,
        plot_w,
        frame_t,
        color::alpha(color::STONE, 0.45),
    );
    push_quad(
        instances,
        plot_x,
        plot_y + plot_h - frame_t,
        plot_w,
        frame_t,
        color::alpha(color::STONE, 0.45),
    );
    push_quad(
        instances,
        plot_x,
        plot_y,
        frame_t,
        plot_h,
        color::alpha(color::STONE, 0.45),
    );
    push_quad(
        instances,
        plot_x + plot_w - frame_t,
        plot_y,
        frame_t,
        plot_h,
        color::alpha(color::STONE, 0.45),
    );

    let eye = Vec3::from_array(cam.eye);
    let forward = rain_view_forward(cam);
    let right = rain_view_right(cam);
    let aspect = window_w / window_h.max(1.0);
    let (d_min, d_max) = volume.frustum_depth_range(cam);
    let near_bias = tuning.field.spawn_near_bias;
    let z_span = (volume.max.z - volume.min.z).max(1e-3);
    let spawn_z = volume.max.z - z_span * 0.275;

    let cell_w = plot_w / SPAWN_FIELD_COLS as f32;
    let cell_h = plot_h / SPAWN_FIELD_ROWS as f32;

    for row in 0..SPAWN_FIELD_ROWS {
        for col in 0..SPAWN_FIELD_COLS {
            let lateral_t = if SPAWN_FIELD_COLS <= 1 {
                0.0
            } else {
                col as f32 / (SPAWN_FIELD_COLS - 1) as f32 * 2.0 - 1.0
            };
            let depth_t = if SPAWN_FIELD_ROWS <= 1 {
                0.0
            } else {
                row as f32 / (SPAWN_FIELD_ROWS - 1) as f32
            };
            let depth = d_min + depth_t * (d_max - d_min).max(1e-3);
            let lateral_half = RainSpawnVolume::frustum_lateral_half_at(cam, depth, aspect);
            let mut pos = eye + forward * depth + right * (lateral_t * lateral_half);
            pos.z = spawn_z;

            let in_volume = volume.contains(pos);
            let in_frustum = volume.in_view_frustum(cam, pos, aspect);
            let weight = if in_volume && in_frustum {
                volume.spawn_weight_at(cam, pos, aspect, near_bias)
            } else {
                0.0
            };

            let cx = plot_x + (col as f32 + 0.5) * cell_w;
            let cy = plot_y + plot_h - (row as f32 + 0.5) * cell_h;
            let dot = (cell_w.min(cell_h) * 0.38).max(1.5);
            let size = dot * (0.55 + 0.45 * weight);
            let alpha = if in_volume && in_frustum {
                0.12 + 0.88 * weight
            } else {
                0.04
            };
            instances.push(GpuInstance {
                rect: [cx - size * 0.5, cy - size * 0.5, size, size],
                color: color::alpha(color::JADE, alpha),
                user: 0,
            });
        }
    }

    // Camera reticle on the plot.
    let cam_depth_t = if (d_max - d_min).abs() > 1e-3 {
        ((0.0 - d_min) / (d_max - d_min)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let cam_x = plot_x + plot_w * 0.5;
    let cam_y = plot_y + plot_h - cam_depth_t * plot_h;
    let frustum_reach = (cam_y - plot_y).max(20.0 * scale);
    push_camera_marker(
        instances,
        cam_x,
        cam_y,
        (10.0 * scale).max(6.0),
        scale,
        cam.fovy_deg(),
        aspect,
        frustum_reach,
        true,
    );

    // Wind arrow on the plot (projected drift direction).
    let speed = tuning.speed_mul.max(0.0);
    let wind = Vec3::new(
        tuning.field.wind_x * speed,
        tuning.field.wind_y * speed,
        0.0,
    );
    let wind_len = wind.length();
    if wind_len > 1e-3 {
        let w_lat = wind.dot(right);
        let w_fwd = wind.dot(forward);
        let dir_len = (w_lat * w_lat + w_fwd * w_fwd).sqrt().max(1e-4);
        let arrow_len = (plot_w.min(plot_h) * 0.22)
            .min(56.0 * scale)
            .max(16.0 * scale);
        push_wind_arrow_icon(
            instances,
            plot_x + plot_w * 0.10,
            plot_y + plot_h * 0.86,
            w_lat / dir_len,
            -w_fwd / dir_len,
            arrow_len,
            scale,
            color::alpha(color::LAPIS, 0.95),
        );
    }

    let axis_font = typography::tier_at_most((11.0 * scale).max(8.0), window_h);
    let axis_h = (12.0 * scale).max(9.0);
    labels.push(TextLabel {
        rect: [plot_x, plot_y + plot_h + 1.0 * scale, plot_w * 0.34, axis_h],
        text: "← L".into(),
        font_px: Some(axis_font),
        color: color::alpha(color::PARCHMENT, 0.7),
        align: TextAlign::Left,
        ..Default::default()
    });
    labels.push(TextLabel {
        rect: [
            plot_x + plot_w * 0.33,
            plot_y + plot_h + 1.0 * scale,
            plot_w * 0.34,
            axis_h,
        ],
        text: "lateral".into(),
        font_px: Some(axis_font),
        color: color::alpha(color::PARCHMENT, 0.55),
        align: TextAlign::Center,
        ..Default::default()
    });
    labels.push(TextLabel {
        rect: [
            plot_x + plot_w * 0.66,
            plot_y + plot_h + 1.0 * scale,
            plot_w * 0.34,
            axis_h,
        ],
        text: "R →".into(),
        font_px: Some(axis_font),
        color: color::alpha(color::PARCHMENT, 0.7),
        align: TextAlign::Right,
        ..Default::default()
    });
    labels.push(TextLabel {
        rect: [
            plot_x + plot_w + 3.0 * scale,
            plot_y + plot_h - axis_h,
            (18.0 * scale).max(12.0),
            axis_h,
        ],
        text: "near".into(),
        font_px: Some(axis_font),
        color: color::alpha(color::PARCHMENT, 0.7),
        align: TextAlign::Left,
        ..Default::default()
    });
    labels.push(TextLabel {
        rect: [
            plot_x + plot_w + 3.0 * scale,
            plot_y,
            (18.0 * scale).max(12.0),
            axis_h,
        ],
        text: "far".into(),
        font_px: Some(axis_font),
        color: color::alpha(color::PARCHMENT, 0.7),
        align: TextAlign::Left,
        ..Default::default()
    });

    // Legend strip (icons match markers drawn on the plot).
    let legend_y0 = plot_y + plot_h + axis_h + (6.0 * scale).max(4.0);
    let legend_x = x0 + pad;
    let legend_w = diagram_w - pad * 2.0;
    let legend_row_h = (legend_h - (8.0 * scale).max(5.0)) / 3.0;
    let swatch_w = (28.0 * scale).max(20.0);
    push_quad(
        instances,
        legend_x,
        legend_y0 - 2.0 * scale,
        legend_w,
        legend_h - (4.0 * scale).max(2.0),
        color::alpha(color::WALNUT_DEEP, 0.88),
    );

    let row_y = |i: usize| legend_y0 + i as f32 * legend_row_h;
    push_legend_swatch_row(
        instances,
        labels,
        legend_x + 4.0 * scale,
        row_y(0),
        legend_row_h,
        swatch_w,
        legend_w * 0.52,
        scale,
        window_h,
        "Spawn chance (bright = more rain)",
        |inst, cx, cy, sc| {
            let n = 4;
            let gap = (4.0 * sc).max(2.5);
            let dot = (5.0 * sc).max(3.0);
            let total = n as f32 * dot + (n as f32 - 1.0) * gap;
            let x0 = cx - total * 0.5;
            for i in 0..n {
                let t = i as f32 / (n as f32 - 1.0);
                let a = 0.15 + 0.85 * t;
                push_quad(
                    inst,
                    x0 + i as f32 * (dot + gap),
                    cy - dot * 0.5,
                    dot,
                    dot,
                    color::alpha(color::JADE, a),
                );
            }
        },
    );
    push_legend_swatch_row(
        instances,
        labels,
        legend_x + legend_w * 0.52,
        row_y(0),
        legend_row_h,
        swatch_w,
        legend_w * 0.46,
        scale,
        window_h,
        "Outside spawn volume",
        |inst, cx, cy, sc| {
            let dot = (5.0 * sc).max(3.0);
            push_quad(
                inst,
                cx - dot * 0.5,
                cy - dot * 0.5,
                dot,
                dot,
                color::alpha(color::JADE, 0.06),
            );
        },
    );
    push_legend_swatch_row(
        instances,
        labels,
        legend_x + 4.0 * scale,
        row_y(1),
        legend_row_h,
        swatch_w,
        legend_w * 0.52,
        scale,
        window_h,
        "Camera + view frustum",
        |inst, cx, cy, sc| {
            push_camera_marker(
                inst,
                cx + (6.0 * sc).max(4.0),
                cy + (4.0 * sc).max(3.0),
                (6.0 * sc).max(4.0),
                sc,
                cam.fovy_deg(),
                aspect,
                (16.0 * sc).max(11.0),
                true,
            );
        },
    );
    if wind_len > 1e-3 {
        push_legend_swatch_row(
            instances,
            labels,
            legend_x + legend_w * 0.52,
            row_y(1),
            legend_row_h,
            swatch_w,
            legend_w * 0.46,
            scale,
            window_h,
            "Wind drift (world XY)",
            |inst, cx, cy, sc| {
                push_wind_arrow_icon(
                    inst,
                    cx - (10.0 * sc).max(7.0),
                    cy,
                    1.0,
                    0.0,
                    (18.0 * sc).max(12.0),
                    sc,
                    color::LAPIS,
                );
            },
        );
    }
    push_legend_swatch_row(
        instances,
        labels,
        legend_x + 4.0 * scale,
        row_y(2),
        legend_row_h,
        swatch_w,
        legend_w - 8.0 * scale,
        scale,
        window_h,
        "Brighter: near camera along view frustum",
        |inst, _cx, cy, sc| {
            let bar_w = (3.0 * sc).max(2.0);
            let bar_h = (12.0 * sc).max(8.0);
            let x = legend_x + swatch_w * 0.35;
            push_quad(
                inst,
                x,
                cy - bar_h * 0.5,
                bar_w,
                bar_h,
                color::alpha(color::STONE, 0.55),
            );
            push_quad(
                inst,
                x + bar_w + 2.0 * sc,
                cy + bar_h * 0.5 - (2.5 * sc).max(2.0),
                bar_w,
                (2.5 * sc).max(2.0),
                color::alpha(color::PARCHMENT, 0.75),
            );
            push_quad(
                inst,
                x + bar_w + 2.0 * sc,
                cy - bar_h * 0.5,
                bar_w,
                (2.5 * sc).max(2.0),
                color::alpha(color::PARCHMENT, 0.45),
            );
        },
    );
}

fn tab_panel_on_left(tab: MainMenuEffectsTab) -> bool {
    matches!(tab, MainMenuEffectsTab::Rain | MainMenuEffectsTab::Moths)
}

struct EffectsDebugLayout {
    panel_x: f32,
    panel_y: f32,
    panel_w: f32,
    tab_y: f32,
    tab_h: f32,
    rows_y0: f32,
    row_h: f32,
    row_gap: f32,
    label_w: f32,
    slider_w: f32,
    value_w: f32,
    scale: f32,
    scroll_row: usize,
    /// Rows shown in the scroll viewport (sliders + mid-tab actions).
    visible_rows: usize,
    scroll_item_count: usize,
    total_rows: usize,
}

impl EffectsDebugLayout {
    fn compute(window_w: f32, window_h: f32, scroll_row: usize, tab: MainMenuEffectsTab) -> Self {
        let scale = metrics::scene_scale(window_w, window_h);
        let row_h = (18.0 * scale).max(14.0);
        let row_gap = (2.0 * scale).max(1.0);
        let title_h = (22.0 * scale).max(14.0);
        let tab_h = (20.0 * scale).max(14.0);
        let pad = (8.0 * scale).max(5.0);
        let margin = (10.0 * scale).max(6.0);
        let mut panel_w = (340.0 * scale).min(window_w * 0.46);
        if tab == MainMenuEffectsTab::Rain {
            let diagram_w = (320.0 * scale).min(window_w * 0.38);
            let max_panel = window_w - diagram_w - margin * 3.0;
            panel_w = panel_w.min(max_panel.max(200.0 * scale));
        }
        let panel_x = if tab_panel_on_left(tab) {
            margin
        } else {
            window_w - panel_w - margin
        };
        let panel_y = margin;
        let tab_y = panel_y + pad + title_h + pad;
        let rows_y0 = tab_y + tab_h + pad;
        let label_w = panel_w * 0.42;
        let slider_w = panel_w * 0.32;
        let value_w = (panel_w - label_w - slider_w - 10.0 * scale).max(32.0);
        let hint_h = (13.0 * scale).max(9.0);
        let hints_block = pad + hint_h * 2.0 + pad;
        let per_row = row_h + row_gap;
        let scroll_item_count = tab_scroll_item_count(tab);
        let total_rows = tab_row_count(tab);
        let footer_block = FOOTER_ACTION_ROWS.len() as f32 * per_row + pad;
        let list_bottom = window_h - margin - hints_block - footer_block;
        let available = (list_bottom - rows_y0).max(row_h);
        let visible_rows = (available / per_row).floor().max(1.0) as usize;
        let visible_rows = visible_rows.clamp(1, MAX_VISIBLE_ROWS);
        Self {
            panel_x,
            panel_y,
            panel_w,
            tab_y,
            tab_h,
            rows_y0,
            row_h,
            row_gap,
            label_w,
            slider_w,
            value_w,
            scale,
            scroll_row,
            visible_rows,
            scroll_item_count,
            total_rows,
        }
    }

    /// Scroll-region height in rows (excludes pinned footer actions).
    fn viewport_rows(&self) -> usize {
        if self.scroll_item_count <= self.visible_rows {
            self.scroll_item_count.max(1)
        } else {
            self.visible_rows
        }
    }

    fn footer_y0(&self) -> f32 {
        let pad = (8.0 * self.scale).max(5.0);
        self.rows_y0 + self.viewport_rows() as f32 * (self.row_h + self.row_gap) + pad
    }

    fn is_footer_row(&self, row: usize) -> bool {
        row >= self.scroll_item_count
    }

    fn row_in_viewport(&self, row: usize) -> bool {
        if self.is_footer_row(row) {
            return true;
        }
        row >= self.scroll_row && row < self.scroll_row + self.visible_rows
    }

    fn tab_rect(&self, tab: MainMenuEffectsTab) -> (f32, f32, f32, f32) {
        let gap = (3.0 * self.scale).max(2.0);
        let tw = (self.panel_w - gap * (TAB_COUNT as f32 - 1.0) - 8.0) / TAB_COUNT as f32;
        let x = self.panel_x + 4.0 + (tw + gap) * tab as u32 as f32;
        (x, self.tab_y, tw, self.tab_h)
    }

    fn row_y(&self, row: usize) -> Option<f32> {
        if row >= self.total_rows {
            return None;
        }
        if self.is_footer_row(row) {
            let footer_idx = row - self.scroll_item_count;
            return Some(self.footer_y0() + footer_idx as f32 * (self.row_h + self.row_gap));
        }
        if !self.row_in_viewport(row) {
            return None;
        }
        let vis = row - self.scroll_row;
        Some(self.rows_y0 + vis as f32 * (self.row_h + self.row_gap))
    }

    fn slider_track(&self, row: usize) -> Option<(f32, f32, f32, f32)> {
        let row_y = self.row_y(row)?;
        let track_x = self.panel_x + self.label_w;
        let track_h = (4.0 * self.scale).max(3.0);
        let track_y = row_y + (self.row_h - track_h) * 0.5;
        Some((track_x, track_y, self.slider_w, track_h))
    }

    fn value_cell(&self, row: usize) -> Option<(f32, f32, f32, f32)> {
        let row_y = self.row_y(row)?;
        let x = self.panel_x + self.label_w + self.slider_w + 4.0 * self.scale;
        Some((x, row_y, self.value_w, self.row_h))
    }

    fn row_rect(&self, row: usize) -> Option<(f32, f32, f32, f32)> {
        let row_y = self.row_y(row)?;
        Some((self.panel_x + 4.0, row_y, self.panel_w - 8.0, self.row_h))
    }

    fn hit_row_rect(&self, row: usize) -> Option<(f32, f32, f32, f32)> {
        if row >= self.total_rows {
            return None;
        }
        self.row_rect(row)
    }
}

fn point_in_rect(mx: f32, my: f32, r: (f32, f32, f32, f32)) -> bool {
    mx >= r.0 && mx <= r.0 + r.2 && my >= r.1 && my <= r.1 + r.3
}

pub struct MainMenuEffectsDebugOverlay {
    pub tab: MainMenuEffectsTab,
    cursor: usize,
    pub tuning: MainMenuEffectsTuning,
    pub pride_rainbow_debug: bool,
    pub moon_phase_debug: MainMenuMoonPhaseDebug,
    pub show_rain_hit_colliders: bool,
    pub show_rain_depth: bool,
    pub hide_all_ui: bool,
    scroll_row: usize,
    editing: bool,
    edit_buffer: String,
    dragging_slider: Option<usize>,
    pointer: DebugPointerState,
}

pub enum MainMenuEffectsDebugResult {
    Stay,
    Close,
    Reset,
    Save,
}

/// Back-compat alias — prefer [`MainMenuEffectsDebugOverlay`].
pub type RainDebugOverlay = MainMenuEffectsDebugOverlay;
pub type RainDebugResult = MainMenuEffectsDebugResult;

impl MainMenuEffectsDebugOverlay {
    pub fn new(
        tuning: MainMenuEffectsTuning,
        pride_rainbow_debug: bool,
        moon_phase_debug: MainMenuMoonPhaseDebug,
    ) -> Self {
        Self {
            tab: MainMenuEffectsTab::Rain,
            cursor: 0,
            tuning,
            pride_rainbow_debug,
            moon_phase_debug,
            show_rain_hit_colliders: false,
            show_rain_depth: false,
            hide_all_ui: false,
            scroll_row: 0,
            editing: false,
            edit_buffer: String::new(),
            dragging_slider: None,
            pointer: DebugPointerState::default(),
        }
    }

    fn row_count(&self) -> usize {
        tab_row_count(self.tab)
    }

    fn ensure_scroll(&mut self, window_w: f32, window_h: f32) {
        let layout = EffectsDebugLayout::compute(window_w, window_h, self.scroll_row, self.tab);
        let vis = layout.visible_rows;
        let scroll_items = layout.scroll_item_count;
        if self.cursor >= self.row_count() {
            self.cursor = self.row_count().saturating_sub(1);
        }
        if layout.is_footer_row(self.cursor) {
            let max_scroll = scroll_items.saturating_sub(vis);
            self.scroll_row = self.scroll_row.min(max_scroll);
            return;
        }
        if self.cursor >= self.scroll_row + vis {
            self.scroll_row = self.cursor.saturating_sub(vis.saturating_sub(1));
        }
        if self.cursor < self.scroll_row {
            self.scroll_row = self.cursor;
        }
        let max_scroll = scroll_items.saturating_sub(vis);
        self.scroll_row = self.scroll_row.min(max_scroll);
    }

    fn row_value(&self, row: usize) -> f32 {
        match self.tab {
            MainMenuEffectsTab::Moon => {
                if moon_row_is_phase(row) {
                    self.moon_phase_debug.resolved_phase()
                } else {
                    self.tuning.moon.debug_row_value(row)
                }
            }
            MainMenuEffectsTab::Rain => self.tuning.rain.debug_row_value(row),
            MainMenuEffectsTab::Moths => self.tuning.moths.debug_row_value(row),
        }
    }

    fn set_row_value(&mut self, row: usize, v: f32) {
        match self.tab {
            MainMenuEffectsTab::Moon => {
                if moon_row_is_phase(row) {
                    let (_, lo, hi, _) = MOON_DEBUG_ROW_META[row];
                    self.moon_phase_debug.forced_phase = v.clamp(lo, hi);
                    self.moon_phase_debug.use_live_calendar = false;
                } else {
                    self.tuning.moon.set_debug_row_value(row, v);
                }
            }
            MainMenuEffectsTab::Rain => self.tuning.rain.set_debug_row_value(row, v),
            MainMenuEffectsTab::Moths => self.tuning.moths.set_debug_row_value(row, v),
        }
    }

    fn set_moon_phase_preset(&mut self, phase: f32) {
        self.moon_phase_debug.forced_phase = phase.clamp(0.0, 1.0);
        self.moon_phase_debug.use_live_calendar = false;
    }

    fn apply_slider_mx(&mut self, row: usize, mx: f32, layout: &EffectsDebugLayout) {
        let Some((tx, _, tw, _)) = layout.slider_track(row) else {
            return;
        };
        let (_, min, max, _) = tab_slider_meta(self.tab)[row];
        let t = ((mx - tx) / tw.max(1e-6)).clamp(0.0, 1.0);
        self.set_row_value(row, min + t * (max - min));
    }

    fn adjust_row(&mut self, dir: f32) {
        if self.editing || self.cursor >= tab_slider_count(self.tab) {
            return;
        }
        let row = self.cursor;
        let (_, _, _, step) = tab_slider_meta(self.tab)[row];
        self.set_row_value(row, self.row_value(row) + dir * step);
    }

    fn clear_edit(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
    }

    fn begin_editing(&mut self) {
        let row = self.cursor;
        let v = self.row_value(row);
        self.edit_buffer = if row_is_hue(self.tab, row) {
            format!("{}", (v * 360.0).round() as i32)
        } else if row_is_saturation(self.tab, row) {
            format!("{:.0}", v * 100.0)
        } else {
            let mut s = format!("{:.6}", v);
            while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
                s.pop();
            }
            s
        };
        self.editing = true;
    }

    fn commit_edit(&mut self) {
        let row = self
            .cursor
            .min(tab_slider_count(self.tab).saturating_sub(1));
        let t = self.edit_buffer.trim();
        let parsed = if row_is_hue(self.tab, row) {
            t.parse::<f32>()
                .ok()
                .map(|deg| (deg / 360.0).fract())
                .or_else(|| t.parse::<f32>().ok().map(|h| h.fract()))
        } else if row_is_saturation(self.tab, row) {
            t.trim_end_matches('%')
                .parse::<f32>()
                .ok()
                .map(|pct| (pct / 100.0).clamp(0.0, 1.0))
        } else {
            t.parse::<f32>().ok()
        };
        if let Some(v) = parsed {
            self.set_row_value(row, v);
        }
        self.clear_edit();
    }

    fn format_row_display(
        tab: MainMenuEffectsTab,
        row: usize,
        v: f32,
        moon_phase_debug: MainMenuMoonPhaseDebug,
    ) -> String {
        let (_, _, _, step) = tab_slider_meta(tab)[row];
        if row_is_hue(tab, row) {
            return format!("{}°", (v * 360.0).round() as i32);
        }
        if row_is_saturation(tab, row) {
            return format!("{:.0}%", v * 100.0);
        }
        if tab == MainMenuEffectsTab::Moon && moon_row_is_phase(row) {
            let name = moon_phase_short_name(v);
            if moon_phase_debug.use_live_calendar {
                return format!("{v:.2} — {name} (live)");
            }
            return format!("{v:.2} — {name} (forced)");
        }
        if step >= 0.01 {
            format!("{v:.2}")
        } else if step >= 0.001 {
            format!("{v:.3}")
        } else {
            format!("{v:.4}")
        }
    }

    fn draw_hue_slider_track(
        track_x: f32,
        track_y: f32,
        tw: f32,
        th: f32,
        instances: &mut Vec<GpuInstance>,
    ) {
        const SEGMENTS: usize = 12;
        let seg_w = tw / SEGMENTS as f32;
        for i in 0..SEGMENTS {
            let h = i as f32 / SEGMENTS as f32;
            let preview = rain_hue_wheel_preview_linear(h);
            instances.push(GpuInstance {
                rect: [track_x + seg_w * i as f32, track_y, seg_w + 0.5, th],
                color: [preview[0], preview[1], preview[2], 1.0],
                user: 0,
            });
        }
    }

    fn draw_action_row(
        &self,
        layout: &EffectsDebugLayout,
        row_y: f32,
        row: usize,
        label: &str,
        instances: &mut Vec<GpuInstance>,
        labels: &mut Vec<TextLabel>,
        row_font: f32,
        variant: ButtonVariant,
    ) {
        let visual = DebugRowVisual::for_row(row, self.cursor, &self.pointer);
        let (bg, tc) = debug_overlay_ui::row_surface_colors(visual, variant);
        instances.push(GpuInstance {
            rect: [
                layout.panel_x + 4.0,
                row_y,
                layout.panel_w - 8.0,
                layout.row_h,
            ],
            color: bg,
            user: 0,
        });
        labels.push(TextLabel {
            rect: [layout.panel_x, row_y, layout.panel_w, layout.row_h],
            text: label.into(),
            font_px: Some(row_font),
            color: tc,
            align: TextAlign::Center,
            ..Default::default()
        });
    }

    pub fn copy_to_clipboard(&self) {
        let text = self.tuning.to_rust_literal();
        #[cfg(feature = "clipboard")]
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                if let Err(e) = cb.set_text(&text) {
                    log::error!("Clipboard write failed: {e}");
                } else {
                    log::info!("Rain tuning snapshot copied");
                }
            }
            Err(e) => log::error!("Could not open clipboard: {e}"),
        }
        #[cfg(not(feature = "clipboard"))]
        log::info!("Rain tuning snapshot (clipboard unavailable in this build):\n{text}");
    }

    pub fn feed_key_event(&mut self, scancode: Option<Scancode>, ctrl: bool) -> bool {
        if self.hide_all_ui {
            if scancode.is_some() {
                self.hide_all_ui = false;
                return true;
            }
            return false;
        }
        let Some(code) = scancode else {
            return false;
        };
        if ctrl && matches!(code, Scancode::C) && !self.editing {
            self.copy_to_clipboard();
            return true;
        }
        if !self.editing {
            return false;
        }
        match code {
            Scancode::Backspace => {
                let _ = self.edit_buffer.pop();
                true
            }
            Scancode::Escape => {
                self.clear_edit();
                true
            }
            Scancode::Return | Scancode::KpEnter => {
                self.commit_edit();
                true
            }
            Scancode::Period | Scancode::KpPeriod => {
                if !self.edit_buffer.contains('.') {
                    self.edit_buffer.push('.');
                }
                true
            }
            Scancode::Minus | Scancode::KpMinus => {
                if self.edit_buffer.is_empty() {
                    self.edit_buffer.push('-');
                }
                true
            }
            _ => {
                if let Some(c) = scancode_to_digit_char(code) {
                    self.edit_buffer.push(c);
                    true
                } else {
                    false
                }
            }
        }
    }

    fn switch_tab(&mut self, tab: MainMenuEffectsTab, window_w: f32, window_h: f32) {
        if self.tab != tab {
            self.tab = tab;
            self.cursor = 0;
            self.scroll_row = 0;
            self.clear_edit();
            self.dragging_slider = None;
            self.ensure_scroll(window_w, window_h);
        }
    }

    fn action_at_row(&self, row: usize) -> Option<ActionRow> {
        let slider_count = tab_slider_count(self.tab);
        if row < slider_count {
            return None;
        }
        let mid = tab_mid_action_rows(self.tab);
        let mid_idx = row - slider_count;
        if mid_idx < mid.len() {
            return Some(mid[mid_idx]);
        }
        let footer_idx = mid_idx - mid.len();
        FOOTER_ACTION_ROWS.get(footer_idx).copied()
    }

    fn dispatch_action(&mut self, action: ActionRow) -> MainMenuEffectsDebugResult {
        match action {
            ActionRow::MoonPhaseLive => {
                self.moon_phase_debug.use_live_calendar = !self.moon_phase_debug.use_live_calendar;
                if !self.moon_phase_debug.use_live_calendar {
                    self.moon_phase_debug.sync_forced_from_calendar();
                }
                MainMenuEffectsDebugResult::Stay
            }
            ActionRow::MoonPhaseNew => {
                self.set_moon_phase_preset(0.0);
                MainMenuEffectsDebugResult::Stay
            }
            ActionRow::MoonPhaseFirstQuarter => {
                self.set_moon_phase_preset(0.25);
                MainMenuEffectsDebugResult::Stay
            }
            ActionRow::MoonPhaseFull => {
                self.set_moon_phase_preset(0.5);
                MainMenuEffectsDebugResult::Stay
            }
            ActionRow::MoonPhaseLastQuarter => {
                self.set_moon_phase_preset(0.75);
                MainMenuEffectsDebugResult::Stay
            }
            ActionRow::PrideRainbow => {
                self.pride_rainbow_debug = !self.pride_rainbow_debug;
                MainMenuEffectsDebugResult::Stay
            }
            ActionRow::ShowRainHit => {
                self.show_rain_hit_colliders = !self.show_rain_hit_colliders;
                MainMenuEffectsDebugResult::Stay
            }
            ActionRow::ShowRainDepth => {
                self.show_rain_depth = !self.show_rain_depth;
                MainMenuEffectsDebugResult::Stay
            }
            ActionRow::HideUi => {
                self.hide_all_ui = true;
                MainMenuEffectsDebugResult::Stay
            }
            ActionRow::Save => MainMenuEffectsDebugResult::Save,
            ActionRow::Reset => MainMenuEffectsDebugResult::Reset,
            ActionRow::Close => MainMenuEffectsDebugResult::Close,
        }
    }

    fn action_row_label(&self, action: ActionRow) -> (&'static str, ButtonVariant) {
        match action {
            ActionRow::MoonPhaseLive => (
                if self.moon_phase_debug.use_live_calendar {
                    "Use live calendar phase [ON]"
                } else {
                    "Use live calendar phase [OFF]"
                },
                ButtonVariant::Default,
            ),
            ActionRow::MoonPhaseNew => ("New moon (0.00)", ButtonVariant::Default),
            ActionRow::MoonPhaseFirstQuarter => ("First quarter (0.25)", ButtonVariant::Default),
            ActionRow::MoonPhaseFull => ("Full moon (0.50)", ButtonVariant::Default),
            ActionRow::MoonPhaseLastQuarter => ("Last quarter (0.75)", ButtonVariant::Default),
            ActionRow::PrideRainbow => (
                if self.pride_rainbow_debug {
                    "Pride rainbow [ON]"
                } else {
                    "Pride rainbow [OFF]"
                },
                ButtonVariant::Default,
            ),
            ActionRow::ShowRainHit => (
                if self.show_rain_hit_colliders {
                    "Show rain_hit colliders [ON]"
                } else {
                    "Show rain_hit colliders [OFF]"
                },
                ButtonVariant::Default,
            ),
            ActionRow::ShowRainDepth => (
                if self.show_rain_depth {
                    "Visualize rain depth [ON]"
                } else {
                    "Visualize rain depth [OFF]"
                },
                ButtonVariant::Default,
            ),
            ActionRow::HideUi => (
                if self.hide_all_ui {
                    "Hide all UI [ON] (press any key)"
                } else {
                    "Hide all UI [OFF]"
                },
                ButtonVariant::Default,
            ),
            ActionRow::Save => ("Save for main menu", ButtonVariant::Primary),
            ActionRow::Reset => ("Reset to defaults", ButtonVariant::Danger),
            ActionRow::Close => ("Close", ButtonVariant::Subtle),
        }
    }

    pub fn update(
        &mut self,
        actions: &[UiAction],
        mouse: Option<(f32, f32, bool, bool)>,
        window_w: f32,
        window_h: f32,
    ) -> MainMenuEffectsDebugResult {
        if self.hide_all_ui {
            self.dragging_slider = None;
            self.pointer.clear_hover();
            return MainMenuEffectsDebugResult::Stay;
        }
        self.ensure_scroll(window_w, window_h);
        let layout = EffectsDebugLayout::compute(window_w, window_h, self.scroll_row, self.tab);
        self.pointer.sync_held(mouse);
        self.pointer.clear_hover();
        let slider_count = tab_slider_count(self.tab);

        if let Some((mx, my, clicked, held)) = mouse {
            if clicked {
                for t in [
                    MainMenuEffectsTab::Moon,
                    MainMenuEffectsTab::Rain,
                    MainMenuEffectsTab::Moths,
                ] {
                    if point_in_rect(mx, my, layout.tab_rect(t)) {
                        self.switch_tab(t, window_w, window_h);
                        break;
                    }
                }
            }

            for i in 0..self.row_count() {
                if let Some(rect) = layout.hit_row_rect(i)
                    && point_in_rect(mx, my, rect)
                {
                    self.pointer.hover_row = Some(i);
                    if self.dragging_slider.is_none() {
                        self.cursor = i;
                    }
                    break;
                }
            }

            if let Some(di) = self.dragging_slider
                && held
                && di < slider_count
            {
                self.apply_slider_mx(di, mx, &layout);
            }
            if (clicked || held) && self.dragging_slider.is_none() {
                for i in self.scroll_row..(self.scroll_row + layout.visible_rows).min(slider_count)
                {
                    let Some(track) = layout.slider_track(i) else {
                        continue;
                    };
                    if point_in_rect(mx, my, track) {
                        self.cursor = i;
                        self.clear_edit();
                        self.apply_slider_mx(i, mx, &layout);
                        if held {
                            self.dragging_slider = Some(i);
                        }
                        break;
                    }
                }
            }
            if clicked && self.dragging_slider.is_none() {
                for i in self.scroll_row..(self.scroll_row + layout.visible_rows).min(slider_count)
                {
                    let Some(cell) = layout.value_cell(i) else {
                        continue;
                    };
                    if point_in_rect(mx, my, cell) {
                        self.cursor = i;
                        self.begin_editing();
                        break;
                    }
                }
                for row in slider_count..self.row_count() {
                    let Some(rect) = layout.hit_row_rect(row) else {
                        continue;
                    };
                    if point_in_rect(mx, my, rect) {
                        self.cursor = row;
                        self.clear_edit();
                        if let Some(action) = self.action_at_row(row) {
                            return self.dispatch_action(action);
                        }
                    }
                }
            }
        }

        if let Some((_, _, _, held)) = mouse {
            if !held {
                self.dragging_slider = None;
            }
        } else {
            self.dragging_slider = None;
        }

        for a in actions {
            match a {
                UiAction::FocusDown => {
                    self.cursor = (self.cursor + 1) % self.row_count();
                    self.clear_edit();
                    self.ensure_scroll(window_w, window_h);
                }
                UiAction::FocusUp => {
                    self.cursor = (self.cursor + self.row_count() - 1) % self.row_count();
                    self.clear_edit();
                    self.ensure_scroll(window_w, window_h);
                }
                UiAction::FocusNext | UiAction::FocusPrev => {
                    if !self.editing {
                        self.adjust_row(if matches!(a, UiAction::FocusPrev) {
                            -1.0
                        } else {
                            1.0
                        });
                    }
                }
                UiAction::Confirm | UiAction::CommitDiscard => {
                    if self.editing {
                        self.commit_edit();
                    } else if let Some(action) = self.action_at_row(self.cursor) {
                        return self.dispatch_action(action);
                    } else if self.cursor < slider_count {
                        self.begin_editing();
                    }
                }
                UiAction::Cancel | UiAction::Pause => {
                    if self.editing {
                        self.clear_edit();
                    } else {
                        return MainMenuEffectsDebugResult::Close;
                    }
                }
                _ => {}
            }
        }
        MainMenuEffectsDebugResult::Stay
    }

    pub fn draw(
        &self,
        window_w: f32,
        window_h: f32,
        cam: CameraParams,
        env_scale: f32,
    ) -> (Vec<GpuInstance>, Vec<TextLabel>) {
        let layout = EffectsDebugLayout::compute(window_w, window_h, self.scroll_row, self.tab);
        let mut instances = Vec::new();
        let mut labels = Vec::new();

        if self.tab == MainMenuEffectsTab::Rain {
            let volume = main_menu_rain_spawn_volume(env_scale, window_h, &self.tuning.rain);
            let margin = (10.0 * layout.scale).max(6.0);
            let diagram_w = (320.0 * layout.scale).min(window_w * 0.38);
            // Panel is on the left for Rain — keep the spawn diagram on the right.
            let diagram_x0 = window_w - diagram_w - margin;
            draw_spawn_field_diagram(
                &cam,
                volume,
                &self.tuning.rain,
                window_w,
                window_h,
                layout.scale,
                diagram_x0,
                &mut instances,
                &mut labels,
            );
        }
        let pad = (8.0 * layout.scale).max(5.0);
        let title_h = (22.0 * layout.scale).max(14.0);
        let hint_h = (13.0 * layout.scale).max(9.0);
        let scroll_vis_h = layout.viewport_rows() as f32 * (layout.row_h + layout.row_gap);
        let footer_vis_h = FOOTER_ACTION_ROWS.len() as f32 * (layout.row_h + layout.row_gap);
        let panel_h = pad
            + title_h
            + pad
            + layout.tab_h
            + pad
            + scroll_vis_h
            + pad
            + footer_vis_h
            + pad
            + hint_h * 2.0
            + pad * 2.0;
        let row_font = typography::tier_at_most(layout.row_h * 0.48, window_h);
        let tab_font = typography::tier_at_most(layout.tab_h * 0.52, window_h);

        let border = 2.0;
        instances.push(GpuInstance {
            rect: [
                layout.panel_x - border,
                layout.panel_y - border,
                layout.panel_w + border * 2.0,
                panel_h + border * 2.0,
            ],
            color: color::alpha(color::WALNUT_SOFT, 0.85),
            user: 0,
        });
        instances.push(GpuInstance {
            rect: [layout.panel_x, layout.panel_y, layout.panel_w, panel_h],
            color: color::alpha(color::WALNUT_INK, 0.97),
            user: 0,
        });

        labels.push(TextLabel {
            rect: [
                layout.panel_x,
                layout.panel_y + pad,
                layout.panel_w,
                title_h,
            ],
            text: "Main menu effects".into(),
            color: color::JADE,
            ..Default::default()
        });

        for t in [
            MainMenuEffectsTab::Moon,
            MainMenuEffectsTab::Rain,
            MainMenuEffectsTab::Moths,
        ] {
            let (tx, ty, tw, th) = layout.tab_rect(t);
            let active = self.tab == t;
            instances.push(GpuInstance {
                rect: [tx, ty, tw, th],
                color: if active {
                    color::alpha(color::JADE, 0.35)
                } else {
                    color::alpha(color::WALNUT_SOFT, 0.55)
                },
                user: 0,
            });
            labels.push(TextLabel {
                rect: [tx, ty, tw, th],
                text: TAB_LABELS[t as usize].into(),
                font_px: Some(tab_font),
                color: if active {
                    color::PARCHMENT
                } else {
                    color::alpha(color::PARCHMENT, 0.72)
                },
                align: TextAlign::Center,
                ..Default::default()
            });
        }

        let slider_count = tab_slider_count(self.tab);
        let rows: &[(&str, f32, f32, f32)] = match self.tab {
            MainMenuEffectsTab::Moon => MOON_DEBUG_ROW_META,
            MainMenuEffectsTab::Rain => RAIN_DEBUG_ROW_META,
            MainMenuEffectsTab::Moths => MOTH_DEBUG_ROW_META,
        };
        let last_visible = (self.scroll_row + layout.visible_rows).min(layout.scroll_item_count);
        for row in self.scroll_row..last_visible {
            if row < slider_count {
                let i = row;
                let Some(&(name, min, max, _)) = rows.get(i) else {
                    continue;
                };
                let visual = DebugRowVisual::for_row(i, self.cursor, &self.pointer);
                let v = self.row_value(i);

                let (bg, tc) = debug_overlay_ui::row_surface_colors(visual, ButtonVariant::Default);
                let Some((rx, ry, rw, rh)) = layout.row_rect(i) else {
                    continue;
                };
                instances.push(GpuInstance {
                    rect: [rx, ry, rw, rh],
                    color: bg,
                    user: 0,
                });

                let swatch = (14.0 * layout.scale).max(10.0);
                let mut label_x = layout.panel_x + 6.0 * layout.scale;
                let mut label_w = layout.label_w - 4.0 * layout.scale;
                if let Some(rgb) = row_color_swatch(&self.tuning, self.tab, i) {
                    let sw_y = ry + (layout.row_h - swatch) * 0.5;
                    instances.push(GpuInstance {
                        rect: [label_x - 1.0, sw_y - 1.0, swatch + 2.0, swatch + 2.0],
                        color: color::alpha(
                            color::PARCHMENT,
                            if visual.highlighted { 0.55 } else { 0.28 },
                        ),
                        user: 0,
                    });
                    instances.push(GpuInstance {
                        rect: [label_x, sw_y, swatch, swatch],
                        color: [rgb[0], rgb[1], rgb[2], 1.0],
                        user: 0,
                    });
                    label_x += swatch + 4.0 * layout.scale;
                    label_w -= swatch + 4.0 * layout.scale;
                }
                labels.push(TextLabel {
                    rect: [label_x, ry, label_w, layout.row_h],
                    text: name.to_string(),
                    font_px: Some(row_font),
                    color: tc,
                    align: TextAlign::Left,
                    ..Default::default()
                });

                let Some((track_x, track_y, tw, th)) = layout.slider_track(i) else {
                    continue;
                };
                let t = ((v - min) / (max - min).max(1e-8)).clamp(0.0, 1.0);
                let fill_w = tw * t;
                if row_is_hue(self.tab, i) {
                    Self::draw_hue_slider_track(track_x, track_y, tw, th, &mut instances);
                } else {
                    instances.push(GpuInstance {
                        rect: [track_x, track_y, tw, th],
                        color: color::WALNUT_INK,
                        user: 0,
                    });
                    let fill_color = row_color_swatch(&self.tuning, self.tab, i)
                        .map(|[r, g, b]| {
                            if visual.highlighted {
                                [r, g, b, 0.95]
                            } else {
                                [r * 0.85, g * 0.85, b * 0.85, 0.75]
                            }
                        })
                        .unwrap_or_else(|| {
                            if visual.highlighted {
                                color::JADE
                            } else {
                                color::alpha(color::JADE, 0.7)
                            }
                        });
                    instances.push(GpuInstance {
                        rect: [track_x, track_y, fill_w, th],
                        color: fill_color,
                        user: 0,
                    });
                }
                let knob_size = th * 2.5;
                let knob_x = track_x + fill_w - knob_size * 0.5;
                let knob_y = track_y + (th - knob_size) * 0.5;
                instances.push(GpuInstance {
                    rect: [knob_x, knob_y, knob_size, knob_size],
                    color: if visual.highlighted {
                        color::PARCHMENT
                    } else {
                        color::alpha(color::STONE, 0.95)
                    },
                    user: 0,
                });

                let Some((vx, vy, vw, vh)) = layout.value_cell(i) else {
                    continue;
                };
                let value_text = if self.editing && i == self.cursor {
                    format!(
                        "{}{}",
                        self.edit_buffer,
                        if visual.highlighted { "\u{258c}" } else { "" }
                    )
                } else {
                    Self::format_row_display(self.tab, i, v, self.moon_phase_debug)
                };
                instances.push(GpuInstance {
                    rect: [vx, vy + vh * 0.15, vw, vh * 0.7],
                    color: if self.editing && i == self.cursor {
                        color::alpha(color::WALNUT_INK, 0.95)
                    } else {
                        color::alpha(color::WALNUT_INK, 0.75)
                    },
                    user: 0,
                });
                labels.push(TextLabel {
                    rect: [vx + 2.0 * layout.scale, vy, vw - 4.0 * layout.scale, vh],
                    text: value_text,
                    font_px: Some(row_font),
                    color: [tc[0] * 0.92, tc[1] * 0.92, tc[2] * 0.92, 1.0],
                    align: TextAlign::Right,
                    ..Default::default()
                });
            } else if let Some(action) = self.action_at_row(row) {
                let Some(row_y) = layout.row_y(row) else {
                    continue;
                };
                let (label, variant) = self.action_row_label(action);
                self.draw_action_row(
                    &layout,
                    row_y,
                    row,
                    label,
                    &mut instances,
                    &mut labels,
                    row_font,
                    variant,
                );
            }
        }

        for (footer_idx, action) in FOOTER_ACTION_ROWS.iter().enumerate() {
            let row = tab_footer_row_index(self.tab, footer_idx);
            let Some(row_y) = layout.row_y(row) else {
                continue;
            };
            let (label, variant) = self.action_row_label(*action);
            self.draw_action_row(
                &layout,
                row_y,
                row,
                label,
                &mut instances,
                &mut labels,
                row_font,
                variant,
            );
        }

        let hint_y = layout.panel_y + panel_h - pad - hint_h * 2.0;
        let hint_font = typography::tier_at_most(hint_h * 0.85, window_h);
        let scroll_hint = if layout.scroll_item_count > layout.visible_rows {
            " · more rows: ↑↓".to_string()
        } else {
            String::new()
        };
        labels.push(TextLabel {
            rect: [layout.panel_x, hint_y, layout.panel_w, hint_h],
            text: format!(
                "Tabs: click · ↑↓ navigate · ←→ adjust · Enter edit/confirm{scroll_hint}"
            ),
            font_px: Some(hint_font),
            color: color::alpha(color::PARCHMENT, 0.55),
            align: TextAlign::Center,
            ..Default::default()
        });
        labels.push(TextLabel {
            rect: [layout.panel_x, hint_y + hint_h, layout.panel_w, hint_h],
            text: "Ctrl+C copy Rust literal".into(),
            font_px: Some(hint_font),
            color: color::alpha(color::PARCHMENT, 0.55),
            align: TextAlign::Center,
            ..Default::default()
        });

        (instances, labels)
    }
}

fn scancode_to_digit_char(code: Scancode) -> Option<char> {
    Some(match code {
        Scancode::_0 | Scancode::Kp0 => '0',
        Scancode::_1 | Scancode::Kp1 => '1',
        Scancode::_2 | Scancode::Kp2 => '2',
        Scancode::_3 | Scancode::Kp3 => '3',
        Scancode::_4 | Scancode::Kp4 => '4',
        Scancode::_5 | Scancode::Kp5 => '5',
        Scancode::_6 | Scancode::Kp6 => '6',
        Scancode::_7 | Scancode::Kp7 => '7',
        Scancode::_8 | Scancode::Kp8 => '8',
        Scancode::_9 | Scancode::Kp9 => '9',
        _ => return None,
    })
}
