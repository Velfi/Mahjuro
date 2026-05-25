//! Floating Kenney prompts along the bottom edge (Discard / Draw / Cash in).

use super::focus::FocusTarget;
use crate::render::draw_cmd::UiFrame;
use crate::render::wgpu_renderer::TextLabel;
use crate::scenes::DrawCtx;
use crate::ui::controller_hints::{
    ColumnHintEntry, ColumnHintLayout, ColumnHintStyle, HintKey, push_column_hints,
};
use crate::ui::input::InputMode;
use crate::ui::kenney_prompt_paths::gameplay_keyboard_prompt_icons;

/// Whether to show the West / North (keyboard **Q** / **E**) gameplay legend for discard or play.
///
/// Hides when the action cannot run (`action_enabled` false). With a controller and
/// "X and Y quick action" off, also hides while focus is on inspect-only HUD (relics, yaku
/// tablets, pegs, etc.) so prompts match what those face buttons do from hand / action buttons.
pub fn gameplay_west_north_legend_active(
    input_mode: InputMode,
    xy_quick_action: bool,
    focus: Option<FocusTarget>,
    action_enabled: bool,
) -> bool {
    if !action_enabled {
        return false;
    }
    match input_mode {
        InputMode::Keyboard | InputMode::Cursor => true,
        InputMode::Controller => {
            if xy_quick_action {
                return true;
            }
            match focus {
                None => true,
                Some(
                    FocusTarget::Relic(_)
                    | FocusTarget::Peg(_)
                    | FocusTarget::Gold
                    | FocusTarget::YakuTablet(_)
                    | FocusTarget::Dora
                    | FocusTarget::Ordeal
                    | FocusTarget::RoundWind
                    | FocusTarget::Consumable(_),
                ) => false,
                Some(_) => true,
            }
        }
    }
}

/// Matches [`crate::scenes::gameplay::scene_behavior`] copy: discard → bowl, play → mirror, cash in → trigger.
const GAMEPLAY_ACTION_PROMPT_LABELS: [&str; 3] = ["Discard", "Play", "Cash in"];

pub struct GameplayActionPromptInput<'a> {
    pub discard_btn_rect: (f32, f32, f32, f32),
    pub play_btn_rect: (f32, f32, f32, f32),
    pub trigger_btn_rect: (f32, f32, f32, f32),
    pub cash_in_enabled: bool,
    pub show_discard_legend: bool,
    pub show_play_legend: bool,
    pub hud_text: &'a mut Vec<TextLabel>,
}

pub fn push_gameplay_action_prompts(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    input: GameplayActionPromptInput<'_>,
) {
    let GameplayActionPromptInput {
        discard_btn_rect,
        play_btn_rect,
        trigger_btn_rect,
        cash_in_enabled,
        show_discard_legend,
        show_play_legend,
        hud_text,
    } = input;
    let h = ctx.layout.window_h;
    let w = ctx.layout.window_w;

    let keyboard_icons = gameplay_keyboard_prompt_icons();
    let all_entries = [
        ColumnHintEntry::new(
            HintKey::Action(crate::ui::input::UiAction::WestFacePress),
            keyboard_icons[0].clone(),
            GAMEPLAY_ACTION_PROMPT_LABELS[0],
        ),
        ColumnHintEntry::new(
            HintKey::Action(crate::ui::input::UiAction::NorthFacePress),
            keyboard_icons[1].clone(),
            GAMEPLAY_ACTION_PROMPT_LABELS[1],
        ),
        ColumnHintEntry::new(
            HintKey::Action(crate::ui::input::UiAction::TriggerStructure),
            keyboard_icons[2].clone(),
            GAMEPLAY_ACTION_PROMPT_LABELS[2],
        ),
    ];

    let rects: [(f32, f32, f32, f32); 3] = [discard_btn_rect, play_btn_rect, trigger_btn_rect];

    let mut visible: [usize; 3] = [0; 3];
    let mut n_visible = 0usize;
    for (i, rect) in rects.iter().enumerate() {
        let (_dx, _dy, dw, dh) = *rect;
        if dw <= 1.0 || dh <= 1.0 {
            continue;
        }
        let show = match i {
            0 => show_discard_legend,
            1 => show_play_legend,
            2 => cash_in_enabled,
            _ => false,
        };
        if !show {
            continue;
        }
        visible[n_visible] = i;
        n_visible += 1;
    }
    if n_visible == 0 {
        return;
    }

    let entries: Vec<ColumnHintEntry> = visible
        .iter()
        .take(n_visible)
        .map(|&i| all_entries[i].clone())
        .collect();

    let layout = ColumnHintLayout::gameplay_floating_band(w, h, n_visible);
    let mut prompt_texts = Vec::new();
    push_column_hints(
        frame,
        ctx,
        layout,
        &entries,
        ColumnHintStyle::gameplay_floating(h),
        &mut prompt_texts,
    );
    hud_text.extend(prompt_texts);
}
