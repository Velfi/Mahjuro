//! Profile selection screen — pick one of three profile slots.

use crate::audio::SfxId;
use crate::game::event_bus::GameEvent;
use crate::persistence;
use crate::render::theme::{color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
use crate::ui::controller_hints::{HintKey, HintRow, HintStyle, push_inline_hint_rows};
use crate::ui::input::UiAction;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use crate::render::draw_cmd::UiFrame;

use super::{
    DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx, archive_career,
    scene_collection_archive,
};

const PROFILE_COUNT: usize = 3;
/// Max stat lines in a populated column (scores, collection counts, saved run).
const MAX_BODY_LINES: usize = 9;

/// Shared profile-column geometry and typography — one source for hit-test and draw.
#[derive(Clone, Copy, Debug)]
struct ProfileCardLayout {
    scale: f32,
    title_h: f32,
    title_y: f32,
    card_w: f32,
    card_h: f32,
    card_gap: f32,
    start_x: f32,
    card_y: f32,
    pad_x: f32,
    pad_y: f32,
    header_font: f32,
    body_font: f32,
    header_line_h: f32,
    body_line_h: f32,
}

impl ProfileCardLayout {
    fn card_content_h(pad_y: f32, header_line_h: f32, body_line_h: f32, body_lines: usize) -> f32 {
        let body_gap = pad_y * 0.3;
        let header_gap = pad_y * 0.5;
        pad_y
            + header_line_h
            + header_gap
            + body_lines as f32 * body_line_h
            + body_lines.saturating_sub(1) as f32 * body_gap
            + pad_y
    }

    fn compute(w: f32, h: f32) -> Self {
        let scale = (w.min(h)) / 600.0;
        let hint_style = HintStyle::profile_footer(h);
        let footer_reserve = hint_style.line_h + h * 0.02;
        let title_font = typography::size(typography::H20, h);
        let title_h = title_font * 1.35;
        let title_y = h * 0.06;
        let margin_x = w * 0.04;
        let card_gap = (20.0 * scale).max(12.0);
        let band_w = w - margin_x * 2.0;
        let card_w =
            ((band_w - card_gap * (PROFILE_COUNT - 1) as f32) / PROFILE_COUNT as f32).max(80.0);
        let pad_x = 14.0 * scale;
        let pad_y = 12.0 * scale;

        let avail_h = h - title_y - title_h - footer_reserve - h * 0.03;
        let card_h = avail_h.max(100.0);
        let card_y = title_y + title_h + h * 0.02;
        let start_x = margin_x;

        let mut header_font = typography::size(typography::H28, h);
        let mut body_font = typography::size(typography::H36, h);
        let mut header_line_h = header_font * 1.35;
        let mut body_line_h = body_font * 1.35;
        let mut min_content_h =
            Self::card_content_h(pad_y, header_line_h, body_line_h, MAX_BODY_LINES);

        let body_floor = typography::size(typography::H45, h);
        while min_content_h > card_h && body_font > body_floor + 0.5 {
            body_font = typography::tier_at_most(body_font - 1.0, h);
            body_line_h = body_font * 1.35;
            min_content_h = Self::card_content_h(pad_y, header_line_h, body_line_h, MAX_BODY_LINES);
        }

        let header_floor = typography::size(typography::H36, h);
        while min_content_h > card_h && header_font > header_floor + 0.5 {
            header_font = typography::tier_at_most(header_font - 1.0, h);
            header_line_h = header_font * 1.35;
            min_content_h = Self::card_content_h(pad_y, header_line_h, body_line_h, MAX_BODY_LINES);
        }

        Self {
            scale,
            title_h,
            title_y,
            card_w,
            card_h,
            card_gap,
            start_x,
            card_y,
            pad_x,
            pad_y,
            header_font,
            body_font,
            header_line_h,
            body_line_h,
        }
    }

    fn card_rects(self) -> Vec<[f32; 4]> {
        (0..PROFILE_COUNT)
            .map(|i| {
                let card_x = self.start_x + i as f32 * (self.card_w + self.card_gap);
                [card_x, self.card_y, self.card_w, self.card_h]
            })
            .collect()
    }
}

fn profile_stat_lines(summary: &persistence::ProfileSummary) -> Vec<(String, [f32; 4])> {
    let mut lines = Vec::new();
    lines.push((
        format!(
            "Depth {}",
            crate::core::progression::meta_depth_roman(summary.level)
        ),
        color::STONE,
    ));
    lines.push((
        format!(
            "{} run{} completed",
            summary.runs_completed,
            if summary.runs_completed == 1 { "" } else { "s" }
        ),
        color::STONE,
    ));
    lines.push((
        format!(
            "{} victor{}",
            summary.victories,
            if summary.victories == 1 { "y" } else { "ies" }
        ),
        color::STONE,
    ));
    lines.push((
        format!("Best: {}", archive_career::format_score(summary.high_score)),
        color::STONE,
    ));
    if summary.second_high_score > 0 {
        lines.push((
            format!(
                "2nd: {}",
                archive_career::format_score(summary.second_high_score)
            ),
            color::STONE,
        ));
    }
    if summary.third_high_score > 0 {
        lines.push((
            format!(
                "3rd: {}",
                archive_career::format_score(summary.third_high_score)
            ),
            color::STONE,
        ));
    }
    lines.push((
        format!(
            "{} relic{} unlocked",
            summary.relics_unlocked,
            if summary.relics_unlocked == 1 {
                ""
            } else {
                "s"
            }
        ),
        color::STONE,
    ));
    lines.push((
        format!("{} yaku discovered", summary.yaku_discovered,),
        color::STONE,
    ));
    if summary.has_saved_run {
        lines.push(("Saved game in progress".into(), color::JADE));
    }
    lines
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PickProfile(usize);

/// Confirmation sub-state for profile deletion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfirmDelete {
    /// No delete pending.
    None,
    /// Waiting for confirmation to delete profile at this index.
    Pending(usize),
}

pub struct ProfileSelectScene {
    tree: TreeState,
    confirm_delete: ConfirmDelete,
}

impl ProfileSelectScene {
    pub fn from_archive_switch_save() -> Self {
        let settings = persistence::load_settings();
        let mut tree = TreeState::new();
        tree.set_focus(FocusId(
            settings.active_profile.min(PROFILE_COUNT - 1) as u32,
        ));
        Self {
            tree,
            confirm_delete: ConfirmDelete::None,
        }
    }

    fn pop_return_scene(&self) -> Scene {
        scene_collection_archive()
    }

    fn cursor(&self) -> usize {
        self.tree
            .focused()
            .map(|f| f.0 as usize)
            .unwrap_or(0)
            .min(PROFILE_COUNT - 1)
    }

    /// Single source of truth for profile card layout — used by both
    /// `update()` (hit-test) and `draw()` (rendering + button registration).
    fn card_rects(window_w: f32, window_h: f32) -> Vec<[f32; 4]> {
        ProfileCardLayout::compute(window_w, window_h).card_rects()
    }

    fn flat_items(window_w: f32, window_h: f32) -> Vec<FlatItem<PickProfile>> {
        Self::card_rects(window_w, window_h)
            .into_iter()
            .enumerate()
            .map(|(i, rect)| FlatItem::new(FocusId(i as u32), rect, PickProfile(i)))
            .collect()
    }
}

impl SceneBehavior for ProfileSelectScene {
    fn face_button_bindings(
        &self,
        _ctx: crate::ui::input::FaceBindingCtx,
    ) -> crate::ui::input::FaceButtonBindings {
        if self.confirm_delete != ConfirmDelete::None {
            return crate::ui::input::FaceButtonBindings::default();
        }
        crate::ui::input::FaceButtonBindings {
            west_press: Some(UiAction::Delete),
            ..Default::default()
        }
    }

    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        // ── Confirmation dialog sub-state ──────────────────────────────
        if let ConfirmDelete::Pending(del_idx) = self.confirm_delete {
            for a in ctx.actions {
                match a {
                    UiAction::Confirm => {
                        *ctx.delete_profile = Some(del_idx);
                        self.confirm_delete = ConfirmDelete::None;
                        return None;
                    }
                    UiAction::Cancel | UiAction::Pause | UiAction::Delete => {
                        self.confirm_delete = ConfirmDelete::None;
                        return None;
                    }
                    _ => {}
                }
            }
            return None;
        }

        // ── Normal profile selection ───────────────────────────────────
        let items = Self::flat_items(ctx.layout.window_w, ctx.layout.window_h);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }

        for a in ctx.actions {
            if matches!(a, UiAction::Cancel | UiAction::Pause) {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                return Some(self.pop_return_scene());
            }
            if matches!(a, UiAction::Delete) {
                let idx = self.cursor();
                if persistence::profile_exists(idx) {
                    self.confirm_delete = ConfirmDelete::Pending(idx);
                    return None;
                }
            }
        }

        if let Some(PickProfile(idx)) = action {
            ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
            *ctx.switch_profile = Some(idx);
            return Some(self.pop_return_scene());
        }
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let layout = ProfileCardLayout::compute(w, h);
        let scale = layout.scale;

        let mut frame = UiFrame::new();
        let mut buttons = Vec::new();

        frame.quad(GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::WALNUT_INK,
            user: 0,
        });

        let showing_dialog = self.confirm_delete != ConfirmDelete::None;

        // Title.
        if !showing_dialog {
            let title = "Switch save";
            frame.text(TextLabel {
                rect: [0.0, layout.title_y, w, layout.title_h],
                text: title.into(),
                color: color::CHAMPAGNE,
                font_px: Some(typography::size(typography::H20, h)),
                ..Default::default()
            });
        }

        // Profile cards — single source of truth via card_rects().
        let summaries = persistence::all_profile_summaries();
        let card_rects = layout.card_rects();
        let cursor = self.cursor();

        for (i, summary) in summaries.iter().enumerate() {
            let [card_x, card_y, card_w, card_h] = card_rects[i];
            let is_focused = i == cursor;
            let is_active = i == ctx.active_profile;

            // Card background.
            if !showing_dialog {
                let bg_color = if is_focused {
                    color::WALNUT_SOFT
                } else {
                    color::WALNUT_RAISED
                };
                frame.quad(GpuInstance {
                    rect: [card_x, card_y, card_w, card_h],
                    color: bg_color,
                    user: 0,
                });

                // Active indicator stripe on top edge.
                if is_active {
                    let stripe_h = 4.0 * scale;
                    frame.quad(GpuInstance {
                        rect: [card_x, card_y, card_w, stripe_h],
                        color: color::JADE,
                        user: 0,
                    });
                }

                // Selection highlight border.
                if is_focused {
                    let border = 2.0 * scale;
                    frame.quad(GpuInstance {
                        rect: [card_x, card_y, card_w, border],
                        color: color::GOLD,
                        user: 0,
                    });
                    frame.quad(GpuInstance {
                        rect: [card_x, card_y + card_h - border, card_w, border],
                        color: color::GOLD,
                        user: 0,
                    });
                    frame.quad(GpuInstance {
                        rect: [card_x, card_y, border, card_h],
                        color: color::GOLD,
                        user: 0,
                    });
                    frame.quad(GpuInstance {
                        rect: [card_x + card_w - border, card_y, border, card_h],
                        color: color::GOLD,
                        user: 0,
                    });
                }
            }

            // Skip card text when the delete dialog is open — text is
            // rendered in a separate overlay pass so quads can't occlude it.
            if showing_dialog {
                continue;
            }

            let pad_x = layout.pad_x;
            let pad_y = layout.pad_y;
            let header_font = layout.header_font;
            let body_font = layout.body_font;
            let header_line_h = layout.header_line_h;
            let body_line_h = layout.body_line_h;

            // Profile header line.
            let header_text = if is_active {
                format!("Profile {}\n(active)", i + 1)
            } else {
                format!("Profile {}", i + 1)
            };
            let header_top = card_y + pad_y + if is_active { 4.0 * scale } else { 0.0 };
            let header_rect = [
                card_x + pad_x,
                header_top,
                card_w - pad_x * 2.0,
                header_line_h * if is_active { 2.0 } else { 1.0 },
            ];
            frame.text(TextLabel {
                rect: header_rect,
                text: header_text,
                color: if is_focused {
                    color::CHAMPAGNE
                } else {
                    color::PARCHMENT
                },
                font_px: Some(header_font),
                align: TextAlign::Center,
                ..Default::default()
            });

            if summary.exists {
                let stat_x = card_x + pad_x;
                let stat_w = card_w - pad_x * 2.0;
                let mut line_y =
                    header_top + header_line_h * if is_active { 2.0 } else { 1.0 } + pad_y * 0.5;

                for (text, stat_color) in profile_stat_lines(summary) {
                    frame.text(TextLabel {
                        rect: [stat_x, line_y, stat_w, body_line_h],
                        text,
                        color: stat_color,
                        font_px: Some(body_font),
                        align: TextAlign::Center,
                        ..Default::default()
                    });
                    line_y += body_line_h + pad_y * 0.3;
                }
            } else {
                let empty_y = header_top + header_line_h + pad_y;
                frame.text(TextLabel {
                    rect: [
                        card_x + pad_x,
                        empty_y,
                        card_w - pad_x * 2.0,
                        body_line_h * 2.0,
                    ],
                    text: "Empty slot\nStart a new adventure".into(),
                    color: color::UMBER,
                    font_px: Some(body_font),
                    align: TextAlign::Center,
                    ..Default::default()
                });
            }
        }

        // Single hit-target list shared with update() — no layout drift.
        let items = Self::flat_items(w, h);
        self.tree.register_flat_buttons(&items, &mut buttons);

        // ── Confirmation overlay ───────────────────────────────────────
        if let ConfirmDelete::Pending(del_idx) = self.confirm_delete {
            // Fully opaque overlay so card text underneath is completely hidden.
            frame.quad(GpuInstance {
                rect: [0.0, 0.0, w, h],
                color: color::WALNUT_INK,
                user: 0,
            });

            let hint_style = HintStyle::profile_footer(h);
            let dialog_pad_x = (18.0 * scale).max(14.0);
            let dialog_pad_y = (16.0 * scale).max(12.0);
            let section_gap = (10.0 * scale).max(8.0);

            let dialog_title_font = typography::size(typography::H24, h);
            let dialog_body_font = typography::size(typography::H36, h);
            let title_line_h = dialog_title_font * 1.35;
            let body_line_h = dialog_body_font * 1.35;
            let btn_h = hint_style.line_h;

            let content_h = dialog_pad_y
                + title_line_h
                + section_gap
                + body_line_h
                + section_gap
                + btn_h
                + dialog_pad_y;
            let dialog_w = ((360.0 * scale).max(280.0)).min(w * 0.85);
            let dialog_h = content_h;
            let dialog_x = (w - dialog_w) * 0.5;
            let dialog_y = (h - dialog_h) * 0.5;
            let inner_x = dialog_x + dialog_pad_x;
            let inner_w = (dialog_w - dialog_pad_x * 2.0).max(1.0);

            // Border (full rectangle).
            let b = 2.0 * scale;
            frame.quad(GpuInstance {
                rect: [
                    dialog_x - b,
                    dialog_y - b,
                    dialog_w + b * 2.0,
                    dialog_h + b * 2.0,
                ],
                color: color::RUBY,
                user: 0,
            });
            // Dialog background.
            frame.quad(GpuInstance {
                rect: [dialog_x, dialog_y, dialog_w, dialog_h],
                color: color::WALNUT_INK,
                user: 0,
            });

            let mut content_y = dialog_y + dialog_pad_y;
            frame.text(TextLabel {
                rect: [inner_x, content_y, inner_w, title_line_h],
                text: format!("Delete Profile {}?", del_idx + 1),
                color: color::CHAMPAGNE,
                font_px: Some(dialog_title_font),
                align: TextAlign::Center,
                ..Default::default()
            });

            content_y += title_line_h + section_gap;
            frame.text(TextLabel {
                rect: [inner_x, content_y, inner_w, body_line_h],
                text: "All progress will be lost.".into(),
                color: color::STONE,
                font_px: Some(dialog_body_font),
                align: TextAlign::Center,
                ..Default::default()
            });

            content_y += body_line_h + section_gap;
            let dialog_footer = HintRow::new()
                .bind(
                    "confirm",
                    vec![HintKey::for_input(
                        ctx.input_mode,
                        UiAction::Confirm,
                        "keyboard_enter",
                    )],
                )
                .sep()
                .bind(
                    "cancel",
                    vec![HintKey::for_input(
                        ctx.input_mode,
                        UiAction::Cancel,
                        "keyboard_escape",
                    )],
                )
                .into_segments();
            let dialog_rect = [inner_x, content_y, inner_w, btn_h];
            push_inline_hint_rows(
                &mut frame,
                &ctx,
                &[dialog_rect],
                &[dialog_footer],
                hint_style,
            );
        }

        // Hint icons at bottom.
        if self.confirm_delete == ConfirmDelete::None {
            let hint_style = HintStyle::profile_footer(h);
            let hint_h = hint_style.line_h;
            let hint_y = h - hint_h - h * 0.02;
            let bottom_footer = HintRow::new()
                .bind("browse", vec![HintKey::dpad_horizontal()])
                .sep()
                .bind(
                    "select",
                    vec![HintKey::for_input(
                        ctx.input_mode,
                        UiAction::Confirm,
                        "keyboard_enter",
                    )],
                )
                .sep()
                .bind(
                    "delete",
                    vec![HintKey::for_input(
                        ctx.input_mode,
                        UiAction::Delete,
                        "keyboard_x",
                    )],
                )
                .sep()
                .bind(
                    "back",
                    vec![HintKey::for_input(
                        ctx.input_mode,
                        UiAction::Cancel,
                        "keyboard_escape",
                    )],
                )
                .into_segments();
            let bottom_rect = [0.0, hint_y, w, hint_h];
            push_inline_hint_rows(
                &mut frame,
                &ctx,
                &[bottom_rect],
                &[bottom_footer],
                hint_style,
            );
        }

        frame.buttons = buttons;
        frame.window_title = "Mahjuro — Select Profile".into();
        frame
    }
}
