//! Full-screen credits roll loaded from `assets/data/credits.json`.

use crate::sfx_id::SfxId;
use crate::core::attribution::attribution_catalog;
use crate::core::credits::{CreditEntry, credits_catalog};
use crate::game::event_bus::GameEvent;
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::controller_hints::{
    HintStyle, back_scroll_footer_row, push_screen_footer_hint, screen_footer_reserve,
};
use crate::ui::input::{InputMode, UiAction};
use crate::ui::smooth_scroll::SmoothScroll;
use crate::ui::widget::{self, TextStyle};

use super::options::OptionsScene;
use super::{ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

const BACK_ID: u32 = 0xF310;

#[derive(Clone, Copy, Debug)]
struct Layout {
    scale: f32,
    title_y: f32,
    title_h: f32,
    subtitle_y: f32,
    subtitle_h: f32,
    content_x: f32,
    content_w: f32,
    content_start_y: f32,
    slot_h: f32,
    slot_gap: f32,
    visible_slots: usize,
    scroll_indicator_x: f32,
    scroll_indicator_w: f32,
    back_x: f32,
    back_y: f32,
    back_w: f32,
    back_h: f32,
}

fn compute_layout(w: f32, h: f32) -> Layout {
    let scale = (w.min(h) / 600.0).max(0.5);
    // Centered column — same cap as tutorial summary / options content density.
    let content_w = (560.0 * scale).min(w * 0.62).max(300.0);
    let content_x = (w - content_w) * 0.5;
    let scroll_indicator_w = (7.0 * scale).max(6.0);
    let scroll_indicator_x = content_x + content_w + (10.0 * scale).max(6.0);

    let title_h = (48.0 * scale).max(28.0);
    let title_y = h * 0.06;
    let subtitle_h = (28.0 * scale).max(18.0);
    let subtitle_y = title_y + title_h + (6.0 * scale);

    let content_start_y = subtitle_y + subtitle_h + h * 0.03;
    let slot_h = (40.0 * scale).max(26.0);
    let slot_gap = (10.0 * scale).max(5.0);

    let back_h = (42.0 * scale).max(28.0);
    let back_y = h - screen_footer_reserve(h) - back_h - (18.0 * scale);
    let back_w = content_w;
    let back_x = content_x;

    let content_end_y = back_y - (12.0 * scale);
    let slot_step = slot_h + slot_gap;
    let avail_h = (content_end_y - content_start_y).max(slot_step);
    let visible_slots = ((avail_h / slot_step).floor() as usize).max(1);

    Layout {
        scale,
        title_y,
        title_h,
        subtitle_y,
        subtitle_h,
        content_x,
        content_w,
        content_start_y,
        slot_h,
        slot_gap,
        visible_slots,
        scroll_indicator_x,
        scroll_indicator_w,
        back_x,
        back_y,
        back_w,
        back_h,
    }
}

enum CreditLine {
    SectionHeader(String),
    Entry(CreditEntry),
    Footer(String),
    BodyText {
        text: String,
        center: bool,
    },
}

fn push_wrapped_text_lines(lines: &mut Vec<CreditLine>, text: &str, center: bool) {
    if !text.is_empty() {
        lines.push(CreditLine::BodyText {
            text: text.to_owned(),
            center,
        });
    }
}

fn build_lines(
    catalog: &crate::core::credits::CreditsCatalog,
    attribution: &crate::core::attribution::AttributionCatalog,
) -> Vec<CreditLine> {
    let mut lines = Vec::new();
    for section in &catalog.sections {
        lines.push(CreditLine::SectionHeader(section.title.clone()));
        for entry in &section.entries {
            lines.push(CreditLine::Entry(entry.clone()));
        }
    }
    if !catalog.footer.is_empty() {
        lines.push(CreditLine::Footer(catalog.footer.clone()));
    }

    if attribution.sections.is_empty() && attribution.subtitle.is_empty() && attribution.footer.is_empty()
    {
        return lines;
    }

    lines.push(CreditLine::SectionHeader(attribution.title.clone()));
    push_wrapped_text_lines(&mut lines, &attribution.subtitle, true);
    for section in &attribution.sections {
        lines.push(CreditLine::SectionHeader(section.title.clone()));
        for entry in &section.entries {
            push_wrapped_text_lines(&mut lines, entry, false);
        }
    }
    push_wrapped_text_lines(&mut lines, &attribution.footer, true);
    lines
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreditsReturn {
    Options,
    Overlay,
}

pub struct CreditsScene {
    return_to: CreditsReturn,
    scroll: SmoothScroll,
    back_focused: bool,
    lines: Vec<CreditLine>,
}

impl CreditsScene {
    pub fn from_options() -> Self {
        Self::new(CreditsReturn::Options)
    }

    pub fn overlay() -> Self {
        Self::new(CreditsReturn::Overlay)
    }

    fn new(return_to: CreditsReturn) -> Self {
        Self {
            return_to,
            scroll: SmoothScroll::new(),
            back_focused: false,
            lines: build_lines(credits_catalog(), attribution_catalog()),
        }
    }

    fn sync_scroll(&self, layout: &Layout) {
        let max = self.lines.len().saturating_sub(layout.visible_slots) as u32;
        self.scroll.set_max(max);
    }

    fn go_back(&self, overlay_request: &mut Option<super::OverlayRequest>) -> SceneTransition {
        match self.return_to {
            CreditsReturn::Options => Some(Scene::Options(OptionsScene::new())),
            CreditsReturn::Overlay => {
                *overlay_request = Some(super::OverlayRequest::Pop);
                None
            }
        }
    }
}

impl SceneBehavior for CreditsScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let layout = compute_layout(ctx.layout.window_w, ctx.layout.window_h);
        self.sync_scroll(&layout);

        if ctx.scroll_lines.abs() > 0.001 {
            let wheel_over_content = {
                let (cx, cy) = ctx.cursor_pos;
                let content_end_y = layout.content_start_y
                    + layout.visible_slots as f32 * (layout.slot_h + layout.slot_gap);
                cx >= layout.content_x
                    && cx <= layout.content_x + layout.content_w
                    && cy >= layout.content_start_y
                    && cy <= content_end_y
            };
            if ctx.input_mode != InputMode::Cursor || wheel_over_content {
                self.scroll.scroll_by(-ctx.scroll_lines);
            }
        }

        if ctx.input_mode == InputMode::Cursor {
            let (cx, cy) = ctx.cursor_pos;
            let over_back = cx >= layout.back_x
                && cx <= layout.back_x + layout.back_w
                && cy >= layout.back_y
                && cy <= layout.back_y + layout.back_h;
            self.back_focused = over_back;
        }

        for &cid in ctx.button_clicks {
            if cid == BACK_ID {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                return self.go_back(ctx.overlay_request);
            }
        }

        for action in ctx.actions {
            match action {
                UiAction::FocusNext => {
                    if !self.back_focused && ctx.input_mode != InputMode::Cursor {
                        self.back_focused = true;
                    }
                }
                UiAction::FocusPrev => {
                    if self.back_focused && ctx.input_mode != InputMode::Cursor {
                        self.back_focused = false;
                    }
                }
                UiAction::Confirm | UiAction::CommitDiscard if self.back_focused => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    return self.go_back(ctx.overlay_request);
                }
                UiAction::Cancel | UiAction::Pause => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    return self.go_back(ctx.overlay_request);
                }
                _ => {}
            }
        }

        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let layout = compute_layout(w, h);
        self.sync_scroll(&layout);
        let smooth = self.scroll.tick();
        let catalog = credits_catalog();

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::WALNUT_INK,
            user: 0,
        }];
        let mut texts = Vec::new();
        let mut buttons = Vec::new();
        texts.push(TextLabel {
            rect: [0.0, layout.title_y, w, layout.title_h],
            text: catalog.title.clone(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(typography::size(typography::H16, h)),
            ..Default::default()
        });

        if !catalog.subtitle.is_empty() {
            texts.push(TextLabel {
                rect: [
                    layout.content_x,
                    layout.subtitle_y,
                    layout.content_w,
                    layout.subtitle_h,
                ],
                text: catalog.subtitle.clone(),
                color: color::UMBER,
                align: TextAlign::Center,
                font_px: Some(typography::size(typography::H28, h)),
                ..Default::default()
            });
        }

        let rule_y = layout.subtitle_y + layout.subtitle_h + (8.0 * layout.scale);
        instances.push(GpuInstance {
            rect: [w * 0.25, rule_y, w * 0.5, (1.0 * layout.scale).max(1.0)],
            color: color::BRASS,
            user: 0,
        });

        let scroll = smooth.floor() as usize;
        let slot_step = layout.slot_h + layout.slot_gap;
        let content_h = layout.visible_slots as f32 * slot_step;
        let content_clip = [
            layout.content_x,
            layout.content_start_y,
            layout.content_w,
            content_h,
        ];
        let max_scroll = self.lines.len().saturating_sub(layout.visible_slots) as f32;

        instances.push(GpuInstance {
            rect: [
                layout.content_x,
                layout.content_start_y,
                layout.content_w,
                content_h,
            ],
            color: color::WALNUT_DEEP,
            user: 0,
        });

        for (vi, line) in self.lines.get(scroll..).into_iter().flatten().enumerate() {
            if vi >= layout.visible_slots {
                break;
            }
            let row_y = layout.content_start_y + vi as f32 * slot_step;
            let row_rect = [layout.content_x, row_y, layout.content_w, layout.slot_h];
            let clip_rect = Some(content_clip);

            match line {
                CreditLine::SectionHeader(title) => {
                    let pad = 12.0 * layout.scale;
                    texts.push(TextLabel {
                        rect: [
                            row_rect[0] + pad,
                            row_y,
                            row_rect[2] - pad * 2.0,
                            layout.slot_h,
                        ],
                        text: title.clone(),
                        color: color::CHAMPAGNE,
                        align: TextAlign::Left,
                        font_px: Some(typography::size(typography::H20, h)),
                        clip_rect,
                        ..Default::default()
                    });
                }
                CreditLine::Entry(CreditEntry { name, role }) => {
                    let pad = 12.0 * layout.scale;
                    let label_w = row_rect[2] * 0.44;
                    let role_x = row_rect[0] + label_w + pad;
                    let role_w = row_rect[2] - label_w - pad * 2.0;
                    texts.push(TextLabel {
                        rect: [row_rect[0] + pad, row_y, label_w - pad, layout.slot_h],
                        text: name.clone(),
                        color: color::PARCHMENT,
                        align: TextAlign::Left,
                        font_px: Some(typography::size(typography::H36, h)),
                        clip_rect,
                        ..Default::default()
                    });
                    if !role.is_empty() {
                        texts.push(TextLabel {
                            rect: [role_x, row_y, role_w.max(0.0), layout.slot_h],
                            text: role.clone(),
                            color: color::STONE,
                            align: TextAlign::Right,
                            font_px: Some(typography::size(typography::H45, h)),
                            clip_rect,
                            ..Default::default()
                        });
                    }
                }
                CreditLine::Footer(text) => {
                    let font = typography::size(typography::H36, h);
                    let wrapped_h = widget::plain_text_block_height(
                        text,
                        layout.content_w,
                        font,
                        widget::PLAIN_TEXT_LINE_STEP_MUL,
                    );
                    widget::push_text_block(
                        &mut texts,
                        [
                            layout.content_x,
                            row_y,
                            layout.content_w,
                            wrapped_h.max(layout.slot_h),
                        ],
                        text,
                        TextStyle {
                            tier: typography::H36,
                            color: color::UMBER,
                            padding: 0.0,
                            align: TextAlign::Center,
                            ..Default::default()
                        },
                        h,
                    );
                }
                CreditLine::BodyText { text, center } => {
                    let font = typography::size(typography::H36, h);
                    let wrapped_h = widget::plain_text_block_height(
                        text,
                        layout.content_w,
                        font,
                        widget::PLAIN_TEXT_LINE_STEP_MUL,
                    );
                    widget::push_text_block(
                        &mut texts,
                        [
                            layout.content_x,
                            row_y,
                            layout.content_w,
                            wrapped_h.max(layout.slot_h),
                        ],
                        text,
                        TextStyle {
                            tier: typography::H36,
                            color: if *center {
                                color::UMBER
                            } else {
                                color::PARCHMENT
                            },
                            padding: 0.0,
                            align: if *center {
                                TextAlign::Center
                            } else {
                                TextAlign::Left
                            },
                            ..Default::default()
                        },
                        h,
                    );
                }
            }
        }

        if max_scroll > 0.0 {
            let indicator_y = layout.content_start_y;
            let indicator_h = content_h;
            instances.push(GpuInstance {
                rect: [
                    layout.scroll_indicator_x,
                    indicator_y,
                    layout.scroll_indicator_w,
                    indicator_h,
                ],
                color: color::WALNUT_RAISED,
                user: 0,
            });
            let thumb_h = (indicator_h * (layout.visible_slots as f32 / self.lines.len() as f32))
                .max(12.0 * layout.scale);
            let thumb_y = indicator_y + (indicator_h - thumb_h) * (smooth / max_scroll);
            instances.push(GpuInstance {
                rect: [
                    layout.scroll_indicator_x,
                    thumb_y,
                    layout.scroll_indicator_w,
                    thumb_h,
                ],
                color: color::WALNUT_BRIGHT,
                user: 0,
            });
        }

        let back_bg = if self.back_focused {
            color::WALNUT_BRIGHT
        } else {
            color::WALNUT_RAISED
        };
        instances.push(GpuInstance {
            rect: [layout.back_x, layout.back_y, layout.back_w, layout.back_h],
            color: back_bg,
            user: 0,
        });
        let back_text = if self.back_focused {
            color::CHAMPAGNE
        } else {
            color::STONE
        };
        texts.push(TextLabel {
            rect: [layout.back_x, layout.back_y, layout.back_w, layout.back_h],
            text: "Back".into(),
            color: back_text,
            align: TextAlign::Center,
            ..Default::default()
        });
        buttons.push(ButtonDef::scene(
            (layout.back_x, layout.back_y, layout.back_w, layout.back_h),
            BACK_ID,
        ));

        let mut frame = UiFrame::new();
        frame.quads(instances);
        frame.texts(texts);
        frame.buttons = buttons;
        if ctx.effect_layers.starfield {
            frame.starfield();
        }
        push_screen_footer_hint(
            &mut frame,
            &ctx,
            back_scroll_footer_row(ctx.input_mode),
            HintStyle::archive_footer(h),
        );
        frame.window_title = "Mahjuro — Credits".into();
        frame
    }
}
