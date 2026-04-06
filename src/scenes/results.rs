//! Results scene — shown after a round ends; player picks a relic reward.

use crate::core::relic::{all_relic_defs, RelicId};
use crate::render::wgpu_renderer::{build_instances_relic_pick, TextLabel};
use crate::ui::input::UiAction;

use super::{DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx};
use super::shop::ShopScene;

pub struct ResultsScene {
    pub choices: Vec<RelicId>,
    pub cursor: usize,
}

impl ResultsScene {
    pub fn new(choices: Vec<RelicId>) -> Self {
        Self { choices, cursor: 0 }
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let max = self.choices.len().saturating_sub(1);
        for a in ctx.actions {
            match a {
                UiAction::FocusNext => self.cursor = (self.cursor + 1).min(max),
                UiAction::FocusPrev => self.cursor = self.cursor.saturating_sub(1),
                UiAction::Confirm | UiAction::CommitDiscard => {
                    let chosen = self.choices[self.cursor];
                    ctx.run.advance_round(chosen);
                    return Some(Scene::Shop(ShopScene::new(ctx.run.run_number, &ctx.run.relics)));
                }
                _ => {}
            }
        }
        None
    }

    pub fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let layout = ctx.layout;
        let slots: Vec<(f32, f32, f32, f32)> = layout
            .hand_slots
            .iter()
            .map(|r| (r.x, r.y, r.w, r.h))
            .collect();

        let defs = all_relic_defs();
        let name = |id: RelicId| defs.iter().find(|d| d.id == id).map(|d| d.name).unwrap_or("?");
        let desc = |id: RelicId| defs.iter().find(|d| d.id == id).map(|d| d.description).unwrap_or("");

        let labels: Vec<&str> = self.choices.iter().map(|id| name(*id)).collect();
        let fmt = |i: usize| -> String {
            if self.cursor == i {
                format!("<<{}>>", labels[i])
            } else {
                format!("[{}]", labels[i])
            }
        };

        let instances = build_instances_relic_pick(
            (layout.score_panel.x, layout.score_panel.y, layout.score_panel.w, layout.score_panel.h),
            (layout.modifier_strip.x, layout.modifier_strip.y, layout.modifier_strip.w, layout.modifier_strip.h),
            &slots,
            self.cursor,
        );

        // Show gold earned and selected relic description.
        let selected_desc = desc(self.choices[self.cursor]);
        let gold = ctx.run.gold;

        // Text: header in score panel + a label in each relic choice quad.
        let pick_indices = [1usize, 6, 11];
        let mut text_labels = vec![
            TextLabel {
                rect: [layout.score_panel.x, layout.score_panel.y, layout.score_panel.w, layout.score_panel.h],
                text: format!("Round complete!   Gold: {}   ←→ choose  Enter pick", gold),
                color: [1.0, 1.0, 1.0, 1.0],
            },
            TextLabel {
                rect: [layout.modifier_strip.x, layout.modifier_strip.y, layout.modifier_strip.w, layout.modifier_strip.h],
                text: selected_desc.to_string(),
                color: [0.9, 0.85, 0.6, 1.0],
            },
        ];
        for (ci, &si) in pick_indices.iter().enumerate() {
            if ci >= self.choices.len() {
                break;
            }
            if let Some(s) = slots.get(si).or_else(|| slots.get(ci)) {
                text_labels.push(TextLabel {
                    rect: [s.0, s.1, s.2, s.3],
                    text: labels[ci].to_string(),
                    color: [1.0, 1.0, 1.0, 1.0],
                });
            }
        }

        let choice_text: Vec<String> = (0..self.choices.len()).map(|i| fmt(i)).collect();

        SceneDrawOutput {
            instances,
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels,
            relic_icons: vec![],
            buttons: vec![],
            window_title: format!(
                "Round complete!  Pick:  {}    ←→ choose  Enter confirm",
                choice_text.join("  "),
            ),
        }
    }
}
