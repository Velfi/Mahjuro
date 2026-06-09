//! Debug scene: tune floating relic-flavor gradient shadows against sample copy.
//!
//! Entered from Debug → Labs → Text Shadow Lab…

use crate::core::relic::RelicFlavorSpan;
use crate::render::decal::{DecalFonts, load_ui_font, load_ui_font_italic};
use crate::render::draw_cmd::UiFrame;
use crate::render::text_shadow_lab::{
    FloatingFlavorShadowTuning, TuningField, layout_floating_flavor_caption_at_band_top_for_spans,
};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::controller_hints::{HintStyle, back_footer_row, push_screen_footer_hint};
use crate::ui::input::UiAction;
use crate::ui::inspect_plaque::estimated_flavor_line_count;

use super::{
    BackgroundId, ButtonDef, DrawCtx, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx,
};

const CLICK_BACK: u32 = 0xE011;
const CLICK_RESET: u32 = 0xE012;
const CLICK_PREV_SAMPLE: u32 = 0xE013;
const CLICK_NEXT_SAMPLE: u32 = 0xE014;
const CLICK_PREV_FIELD: u32 = 0xE015;
const CLICK_NEXT_FIELD: u32 = 0xE016;
const CLICK_NUDGE_DOWN: u32 = 0xE017;
const CLICK_NUDGE_UP: u32 = 0xE018;

struct SampleSpec {
    label: &'static str,
    flavor: &'static [RelicFlavorSpan],
}

const SAMPLES: &[SampleSpec] = &[
    SampleSpec {
        label: "Green Luck (short)",
        flavor: &[RelicFlavorSpan {
            text: "The luck of the Irish!",
            bold: false,
            italic: false,
        }],
    },
    SampleSpec {
        label: "Kindling (italic)",
        flavor: &[RelicFlavorSpan {
            text: "Every hand thrown feeds the fire. By the end, it roars.",
            bold: false,
            italic: true,
        }],
    },
    SampleSpec {
        label: "Nest Egg (two lines)",
        flavor: &[
            RelicFlavorSpan {
                text: "The storeroom remembers",
                bold: false,
                italic: false,
            },
            RelicFlavorSpan {
                text: "\n",
                bold: false,
                italic: false,
            },
            RelicFlavorSpan {
                text: "its losers displayed and priced.",
                bold: false,
                italic: false,
            },
        ],
    },
    SampleSpec {
        label: "Dragon Rage (long)",
        flavor: &[RelicFlavorSpan {
            text: "The house has crossed the Dragon; His rage will carry you to a most deserved victory.",
            bold: false,
            italic: false,
        }],
    },
];

pub struct TextShadowLabScene {
    has_suspended: bool,
    sample_idx: usize,
    field: TuningField,
    tuning: FloatingFlavorShadowTuning,
}

impl TextShadowLabScene {
    pub fn new(has_suspended: bool) -> Self {
        Self {
            has_suspended,
            sample_idx: 0,
            field: TuningField::PadTopBody,
            tuning: FloatingFlavorShadowTuning::DEFAULT,
        }
    }

    fn sample(&self) -> &'static SampleSpec {
        &SAMPLES[self.sample_idx % SAMPLES.len()]
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        if self.has_suspended {
            *overlay_request = Some(super::OverlayRequest::Pop);
            None
        } else {
            Some(SceneIntent::MainMenu)
        }
    }

    fn nudge(&mut self, delta: f32) {
        self.field.nudge(&mut self.tuning, delta);
    }
}

impl SceneBehavior for TextShadowLabScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        for &cid in ctx.button_clicks {
            match cid {
                CLICK_BACK => return self.go_back(ctx.overlay_request),
                CLICK_RESET => self.tuning = FloatingFlavorShadowTuning::DEFAULT,
                CLICK_PREV_SAMPLE => {
                    self.sample_idx = (self.sample_idx + SAMPLES.len() - 1) % SAMPLES.len();
                }
                CLICK_NEXT_SAMPLE => {
                    self.sample_idx = (self.sample_idx + 1) % SAMPLES.len();
                }
                CLICK_PREV_FIELD => self.field = self.field.prev(),
                CLICK_NEXT_FIELD => self.field = self.field.next(),
                CLICK_NUDGE_DOWN => self.nudge(-0.01),
                CLICK_NUDGE_UP => self.nudge(0.01),
                _ => {}
            }
        }
        for a in ctx.actions {
            match a {
                UiAction::Cancel | UiAction::Pause => {
                    return self.go_back(ctx.overlay_request);
                }
                UiAction::FocusNext | UiAction::FocusDown => self.field = self.field.next(),
                UiAction::FocusPrev | UiAction::FocusUp => self.field = self.field.prev(),
                UiAction::Confirm => {
                    self.sample_idx = (self.sample_idx + 1) % SAMPLES.len();
                }
                UiAction::NorthFacePress => {
                    self.tuning = FloatingFlavorShadowTuning::DEFAULT;
                }
                UiAction::WestFacePress => {
                    self.sample_idx = (self.sample_idx + SAMPLES.len() - 1) % SAMPLES.len();
                }
                _ => {}
            }
        }
        if ctx.scroll_lines.abs() > f32::EPSILON {
            self.nudge(ctx.scroll_lines * 0.01);
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let sample = self.sample();

        let title_font = typography::size(typography::H20, h);
        let body_font = typography::size(typography::H36, h);
        let field_font = typography::size(typography::H42, h);
        let btn_font = typography::size(typography::H36, h);
        let margin = (w * 0.04).max(16.0);
        let gap = (8.0 * scale).max(6.0);

        let title_h = title_font * 1.45;
        let body_line_h = body_font * 1.28;
        let field_h = field_font * 1.28;
        let metrics_line_h = body_font * 1.28;
        let btn_h = (40.0 * scale).max(30.0);
        let footer_h = HintStyle::standard(w, h).line_h + h * 0.028;
        let toolbar_y = h - footer_h - btn_h - gap;
        let header_h =
            margin + title_h + gap + body_line_h * 3.0 + gap + field_h + gap + metrics_line_h + gap;
        let stage_top = header_h;
        let stage_h = (toolbar_y - gap - stage_top).max(body_font * 4.0);
        let stage_left = w * 0.12;
        let stage_w = w * 0.76;

        let body_px = typography::size(typography::H32, h);
        let min_font_px = typography::readable_floor_px(h);
        let layout = if let Some(font) = load_ui_font() {
            let fonts = DecalFonts {
                regular: font,
                italic: load_ui_font_italic(),
                emoji: None,
            };
            layout_floating_flavor_caption_at_band_top_for_spans(
                w,
                stage_top,
                stage_h,
                &fonts,
                sample.flavor,
                body_px,
                min_font_px,
                &self.tuning,
            )
        } else {
            let line_step = typography::size(typography::H32, h) * 1.4;
            let band_w = (w - 2.0 * w * self.tuning.margin_x_frac).min(self.tuning.band_max_w);
            let content_lines = estimated_flavor_line_count(sample.flavor, band_w, body_px, 8);
            let preview_band_h = (line_step * content_lines as f32 + body_px * 0.5)
                .min(stage_h * 0.45)
                .max(body_px * 2.0);
            let band_top = stage_top + stage_h - preview_band_h;
            crate::render::text_shadow_lab::layout_floating_flavor_caption_at_band_top(
                w,
                band_top,
                stage_h,
                body_px,
                line_step,
                content_lines,
                &self.tuning,
            )
        };

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        frame.window_title = "Mahjuro — Text Shadow Lab".into();

        frame.quads([
            GpuInstance {
                rect: [0.0, 0.0, w, h],
                color: color::alpha(color::WALNUT_INK, 0.92),
                user: 0,
            },
            GpuInstance {
                rect: [stage_left, stage_top, stage_w, stage_h],
                color: color::alpha([0.28, 0.16, 0.10, 1.0], 0.88),
                user: 0,
            },
        ]);

        frame.quads([
            GpuInstance {
                rect: layout.text_rect(),
                color: color::alpha([0.25, 0.85, 0.45, 1.0], 0.18),
                user: 0,
            },
            GpuInstance {
                rect: layout.content_rect(),
                color: color::alpha([0.35, 0.70, 0.85, 1.0], 0.22),
                user: 0,
            },
            GpuInstance {
                rect: [
                    layout.shadow_rect[0],
                    layout.shadow_rect[1],
                    layout.shadow_rect[2],
                    2.0,
                ],
                color: color::alpha([0.90, 0.35, 0.75, 1.0], 0.85),
                user: 0,
            },
            GpuInstance {
                rect: [
                    layout.shadow_rect[0],
                    layout.shadow_rect[1] + layout.shadow_rect[3] - 2.0,
                    layout.shadow_rect[2],
                    2.0,
                ],
                color: color::alpha([0.90, 0.35, 0.75, 1.0], 0.85),
                user: 0,
            },
        ]);

        frame.gradient_quads([layout.gradient_quad(&self.tuning)]);

        frame.text(TextLabel {
            rect: layout.text_rect(),
            text: String::new(),
            color: color::CHAMPAGNE,
            font_px: Some(body_px),
            align: TextAlign::Center,
            scroll_offset: 0.0,
            flavor_spans: Some(sample.flavor),
            bold: false,
            italic: false,
            underline: false,
            text_effect: crate::render::text_effect::TextEffectId::Flat,
            rotation_quarters: 0,
            baseline_shift_px: 0.0,
            clip_rect: None,
            block_vertical_align: Default::default(),
            mono: false,
        });

        let mut header_y = margin;
        frame.text(TextLabel {
            rect: [margin, header_y, w - margin * 2.0, title_h],
            text: "Text Shadow Lab".into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Left,
            font_px: Some(title_font),
            ..Default::default()
        });
        header_y += title_h + gap;

        for line in [
            format!("Sample: {} (←/→ or Confirm to cycle)", sample.label),
            "Green = text band · cyan = copy block · magenta = shadow bounds.".into(),
            "↑/↓ field · scroll ±0.01 · North = reset defaults.".into(),
        ] {
            frame.text(TextLabel {
                rect: [margin, header_y, w - margin * 2.0, body_line_h],
                text: line,
                color: color::PARCHMENT,
                align: TextAlign::Left,
                font_px: Some(body_font),
                ..Default::default()
            });
            header_y += body_line_h;
        }
        header_y += gap;
        frame.text(TextLabel {
            rect: [margin, header_y, w - margin * 2.0, field_h],
            text: format!(
                "Field: {} = {:.3}",
                self.field.label(),
                self.field.value(&self.tuning),
            ),
            color: color::GOLD,
            align: TextAlign::Left,
            font_px: Some(field_font),
            ..Default::default()
        });
        header_y += field_h + gap * 0.5;
        if let Some(font) = load_ui_font() {
            let fonts = DecalFonts {
                regular: font,
                italic: load_ui_font_italic(),
                emoji: None,
            };
            let band_w = (w - 2.0 * w * self.tuning.margin_x_frac).min(self.tuning.band_max_w);
            let metrics = crate::render::decal::measure_flavor_spans_layout(
                &fonts,
                sample.flavor,
                band_w.max(1.0) as u32,
                layout.band_h.max(1.0) as u32,
                body_px,
                min_font_px,
            );
            let heuristic = estimated_flavor_line_count(sample.flavor, band_w, body_px, 8);
            frame.text(TextLabel {
                rect: [margin, header_y, w - margin * 2.0, body_font * 1.28],
                text: format!(
                    "Raster: {} line(s) @ {:.1}px · heuristic guessed {heuristic}",
                    metrics.line_count, metrics.font_px,
                ),
                color: color::STONE,
                align: TextAlign::Left,
                font_px: Some(body_font),
                ..Default::default()
            });
        }

        let labels = [
            ("◀ sample", CLICK_PREV_SAMPLE),
            ("field ◀", CLICK_PREV_FIELD),
            ("−", CLICK_NUDGE_DOWN),
            ("+", CLICK_NUDGE_UP),
            ("field ▶", CLICK_NEXT_FIELD),
            ("sample ▶", CLICK_NEXT_SAMPLE),
            ("Reset", CLICK_RESET),
            ("Back", CLICK_BACK),
        ];
        let btn_w = ((w - margin * 2.0) - gap * (labels.len() as f32 - 1.0)) / labels.len() as f32;
        for (i, (label, cid)) in labels.iter().enumerate() {
            let x = margin + (btn_w + gap) * i as f32;
            frame.quad(GpuInstance {
                rect: [x, toolbar_y, btn_w, btn_h],
                color: color::WALNUT_INK,
                user: 0,
            });
            frame.text(TextLabel {
                rect: [x, toolbar_y, btn_w, btn_h],
                text: (*label).into(),
                color: color::CHAMPAGNE,
                align: TextAlign::Center,
                font_px: Some(btn_font),
                ..Default::default()
            });
            frame
                .buttons
                .push(ButtonDef::scene((x, toolbar_y, btn_w, btn_h), *cid));
        }

        push_screen_footer_hint(
            &mut frame,
            &ctx,
            back_footer_row(ctx.input_mode),
            HintStyle::standard(w, h),
        );
        frame
    }
}
