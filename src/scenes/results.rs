//! Results scene — shown after a round ends; player picks a relic reward.

use crate::core::relic::{all_relic_defs, RelicId};
use crate::render::wgpu_renderer::{build_instances_relic_pick, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::layout::LayoutResult;

use super::{ButtonDef, DrawCtx, Scene, SceneDrawOutput, SceneTransition, UpdateCtx};
use super::game_over::GameOverScene;
use super::shop::ShopScene;

/// Slot indices in `layout.hand_slots` used by the 3 relic-choice quads.
/// Must match `build_instances_relic_pick` in `wgpu_renderer.rs`.
const PICK_SLOT_INDICES: [usize; 3] = [1, 6, 11];

pub struct ResultsScene {
    pub choices: Vec<RelicId>,
    pub cursor: usize,
}

impl ResultsScene {
    pub fn new(choices: Vec<RelicId>) -> Self {
        Self { choices, cursor: 0 }
    }

    /// Screen rects for each relic choice quad, in choice-index order.
    /// Shared between `update()` (hover hit-test) and `draw()` (button rects).
    fn choice_rects(&self, layout: &LayoutResult) -> Vec<(f32, f32, f32, f32)> {
        let mut rects = Vec::with_capacity(self.choices.len());
        for (choice_idx, &slot_idx) in PICK_SLOT_INDICES.iter().enumerate() {
            if choice_idx >= self.choices.len() {
                break;
            }
            let slot = layout
                .hand_slots
                .get(slot_idx)
                .or_else(|| layout.hand_slots.get(choice_idx));
            if let Some(s) = slot {
                rects.push((s.x, s.y, s.w, s.h));
            }
        }
        rects
    }

    pub fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        let max = self.choices.len().saturating_sub(1);

        // Mouse hover → focus the relic choice under the pointer so that a
        // click (which routes through Confirm) acts on the right card.
        let (cx, cy) = ctx.cursor_pos;
        for (i, &(rx, ry, rw, rh)) in self.choice_rects(ctx.layout).iter().enumerate() {
            if cx >= rx && cx <= rx + rw && cy >= ry && cy <= ry + rh {
                self.cursor = i;
                break;
            }
        }

        for a in ctx.actions {
            match a {
                UiAction::FocusNext => self.cursor = (self.cursor + 1).min(max),
                UiAction::FocusPrev => self.cursor = self.cursor.saturating_sub(1),
                UiAction::Confirm | UiAction::CommitDiscard => {
                    let chosen = self.choices[self.cursor];
                    let final_score = ctx.run.round_score;
                    let target = ctx.run.target_score;
                    ctx.run.advance_round(chosen);
                    if ctx.run.is_run_complete() {
                        return Some(Scene::GameOver(GameOverScene::victory(
                            final_score,
                            target,
                        )));
                    }
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

        // Make each relic-choice quad clickable. Hover-focus in update()
        // ensures self.cursor is already pointing at whatever the pointer is
        // over by the time the Confirm action is processed.
        let buttons: Vec<ButtonDef> = self
            .choice_rects(layout)
            .into_iter()
            .map(|rect| ButtonDef::ui(rect, UiAction::Confirm))
            .collect();

        SceneDrawOutput {
            background: super::BackgroundId::Score,
            instances,
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels,
            relic_icons: vec![],
            buttons,
            window_title: format!(
                "Round complete!  Pick:  {}    ←→ choose  Enter confirm",
                choice_text.join("  "),
            ),
            departing_indices: vec![],
            hint_indices: vec![],
        }
    }
}
