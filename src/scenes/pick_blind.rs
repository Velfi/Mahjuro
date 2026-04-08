//! Pick-blind scene — Balatro-style: shows the next blind in the
//! Small → Big → Boss cycle. The player can play it, or skip it
//! (Small/Big only) to jump straight to the next blind.

use crate::core::rules::BlindKind;
use crate::render::theme::{ButtonState, ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::layout::LayoutResult;
use crate::ui::widget::{self, PanelVariant};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::gameplay::GameplayScene;
use super::pause_menu::PauseMenu;
use super::{DrawCtx, Scene, SceneBehavior, SceneDrawOutput, SceneTransition, UpdateCtx, relic_row};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlindAction {
    PlayBlind,
    SkipBlind,
}

impl BlindAction {
    fn id(self) -> FocusId {
        FocusId(self as u32 + 1)
    }
}

pub struct PickBlindScene {
    tree: TreeState,
    pause_menu: PauseMenu,
}

impl PickBlindScene {
    pub fn new() -> Self {
        let mut tree = TreeState::new();
        tree.set_focus(BlindAction::PlayBlind.id());
        Self {
            tree,
            pause_menu: PauseMenu::new(),
        }
    }

    fn can_skip(blind: BlindKind) -> bool {
        !matches!(blind, BlindKind::Boss)
    }

    /// Single source of truth: card rect + skip button rect, registered as
    /// flat hit-targets shared between update() and draw().
    fn flat_items(layout: &LayoutResult, can_skip: bool) -> Vec<FlatItem<BlindAction>> {
        let w = layout.window_w;
        let h = layout.window_h;
        let hs = layout.hand_strip;
        let card_w = hs.w * 0.42;
        let pad_y = hs.h * 0.08;
        let card_h = hs.h - pad_y * 2.0;
        let card_x = hs.x + (hs.w - card_w) * 0.5;
        let card_y = hs.y + pad_y;
        let scale = (w.min(h)) / 600.0;
        let btn_w = (160.0 * scale).max(80.0);
        let btn_h = (38.0 * scale).max(24.0);
        let btn_x = (w - btn_w) * 0.5;
        let btn_y = h - btn_h - (12.0 * scale);

        let mut items = vec![FlatItem::new(
            BlindAction::PlayBlind.id(),
            [card_x, card_y, card_w, card_h],
            BlindAction::PlayBlind,
        )];
        if can_skip {
            items.push(FlatItem::new(
                BlindAction::SkipBlind.id(),
                [btn_x, btn_y, btn_w, btn_h],
                BlindAction::SkipBlind,
            ));
        }
        items
    }

    fn skip_focused(&self) -> bool {
        self.tree.focused() == Some(BlindAction::SkipBlind.id())
    }

}

impl SceneBehavior for PickBlindScene {
    fn pause_options_overlay(&self) -> Option<&super::options::OptionsScene> {
        self.pause_menu.options_overlay()
    }

    fn has_blocking_overlay(&self) -> bool {
        self.pause_menu.paused
    }

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        // Pause menu handling — drives the menu while paused and intercepts
        // the open-on-Pause shortcut. Returns immediately if either applies.
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            return t;
        }

        let upcoming = ctx.run.upcoming_blind;
        let can_skip = Self::can_skip(upcoming);

        let items = Self::flat_items(ctx.layout, can_skip);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
            },
        );

        // Cancel-to-skip shortcut. (The pause shortcut is already handled
        // by `pause_menu.handle()` above.)
        for a in ctx.actions {
            if matches!(a, UiAction::Cancel) && can_skip {
                let reward = upcoming.skip_reward();
                ctx.run.gold = ctx.run.gold.saturating_add(reward);
                ctx.run.skip_to_next_blind();
                return Some(Scene::PickBlind(PickBlindScene::new()));
            }
        }

        match action {
            Some(BlindAction::SkipBlind) if can_skip => {
                let reward = upcoming.skip_reward();
                ctx.run.gold = ctx.run.gold.saturating_add(reward);
                ctx.run.skip_to_next_blind();
                Some(Scene::PickBlind(PickBlindScene::new()))
            }
            Some(BlindAction::PlayBlind) | Some(BlindAction::SkipBlind) => {
                ctx.run.apply_blind(upcoming);
                Some(Scene::Gameplay(GameplayScene::new()))
            }
            None => None,
        }
    }

    fn draw(&self, ctx: DrawCtx<'_>) -> SceneDrawOutput {
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let hs = ctx.layout.hand_strip;

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::OBSIDIAN,
        }];

        // Relic row in its own strip.
        let (relic_insts, relic_labels, relic_icons) =
            relic_row(&ctx.run.relics, &ctx.layout.relic_strip, w);
        instances.extend(relic_insts);

        let upcoming = ctx.run.upcoming_blind;
        let can_skip = Self::can_skip(upcoming);

        // Single centered card for the upcoming blind.
        let card_w = hs.w * 0.42;
        let pad_y = hs.h * 0.08;
        let card_h = hs.h - pad_y * 2.0;
        let card_x = hs.x + (hs.w - card_w) * 0.5;
        let card_y = hs.y + pad_y;

        // Faint gold halo when the card is the focused choice.
        let skip_focused = self.skip_focused();
        if !skip_focused {
            let halo = card_h * 0.04;
            instances.push(GpuInstance {
                rect: [
                    card_x - halo,
                    card_y - halo,
                    card_w + halo * 2.0,
                    card_h + halo * 2.0,
                ],
                color: color::alpha(color::GOLD, 0.45),
            });
        }
        // Card background — Hero panel for boss, Default otherwise.
        let variant = match upcoming {
            BlindKind::Boss => PanelVariant::Hero,
            _ => PanelVariant::Default,
        };
        widget::push_panel(&mut instances, [card_x, card_y, card_w, card_h], variant);

        // Skip button at bottom (Small/Big only).
        let scale = (w.min(h)) / 600.0;
        let btn_w = (160.0 * scale).max(80.0);
        let btn_h = (38.0 * scale).max(24.0);
        let btn_x = (w - btn_w) * 0.5;
        let btn_y = h - btn_h - (12.0 * scale);

        // Text labels.
        let ms = ctx.layout.modifier_strip;
        let base = ctx.run.base_target;
        let gold = ctx.run.gold;
        let effective_target = (base as f32 * upcoming.target_multiplier()) as u32;

        // Score header — Hero panel.
        widget::push_panel(
            &mut instances,
            [
                ctx.layout.score_panel.x,
                ctx.layout.score_panel.y,
                ctx.layout.score_panel.w,
                ctx.layout.score_panel.h,
            ],
            PanelVariant::Hero,
        );
        let mut text_labels = vec![
            TextLabel {
                rect: [
                    ctx.layout.score_panel.x,
                    ctx.layout.score_panel.y,
                    ctx.layout.score_panel.w,
                    ctx.layout.score_panel.h,
                ],
                text: format!(
                    "ANTE {}/{}   ·   Gold {}   ·   Target {}",
                    ctx.run.ante,
                    crate::game::run::FINAL_ANTE,
                    gold,
                    effective_target
                ),
                color: color::CHAMPAGNE,
                ..Default::default()
            },
            TextLabel {
                rect: [ms.x, ms.y, ms.w, ms.h],
                text: format!(
                    "{}   |   Enter play   {}",
                    upcoming.description(),
                    if can_skip { "Esc/↓ skip" } else { "" }
                ),
                color: color::PARCHMENT,
                ..Default::default()
            },
        ];
        text_labels.extend(relic_labels);

        // rasterize_label renders glyphs at ~0.55 of rect.h, so rect heights
        // need a 1.8× bump for the rendered text to match the typography tier.
        let name_h = typography::size(typography::TITLE, h) * 1.8;
        let name_y = card_y + card_h * 0.16;
        text_labels.push(TextLabel {
            rect: [card_x, name_y, card_w, name_h],
            text: upcoming.name().to_string(),
            color: color::CHAMPAGNE,
            ..Default::default()
        });
        let mult = upcoming.target_multiplier();
        let desc_h = typography::size(typography::HEADING, h) * 1.8;
        let desc_y = card_y + card_h * 0.42;
        text_labels.push(TextLabel {
            rect: [card_x, desc_y, card_w, desc_h],
            text: format!("×{:.1} target", mult),
            color: color::PARCHMENT,
            ..Default::default()
        });
        // Show forced modifier on Boss card.
        if let Some(modifier) = upcoming.forced_modifier(ctx.run.run_number) {
            let mod_h = typography::size(typography::CAPTION, h) * 1.8;
            let mod_y = card_y + card_h * 0.68;
            text_labels.push(TextLabel {
                rect: [card_x, mod_y, card_w, mod_h],
                text: format!("{}: {}", modifier.name(), modifier.description()),
                color: color::AMBER,
                ..Default::default()
            });
        }

        // Round wind for the upcoming ante (Patch B). Triplets/kongs of this
        // wind fire the Yakuhai yaku, so showing it here lets the player plan
        // their hand before sitting down.
        let wind_rank = BlindKind::round_wind_for_ante(ctx.run.ante);
        let wind_h = typography::size(typography::CAPTION, h) * 1.8;
        let wind_y = card_y + card_h * 0.83;
        text_labels.push(TextLabel {
            rect: [card_x, wind_y, card_w, wind_h],
            text: format!("Round Wind: {}", BlindKind::wind_name(wind_rank)),
            color: color::GOLD,
            ..Default::default()
        });

        let mut buttons = Vec::new();
        if can_skip {
            let reward = upcoming.skip_reward();
            widget::push_button(
                &mut instances,
                &mut text_labels,
                &mut buttons,
                [btn_x, btn_y, btn_w, btn_h],
                &format!("Skip (+{}g)", reward),
                ButtonVariant::Subtle,
                if skip_focused {
                    ButtonState::Hover
                } else {
                    ButtonState::Rest
                },
                UiAction::Cancel,
            );
            // Drop the synthetic ButtonDef::ui from push_button — flat_items
            // re-registers with stable click ids below.
            buttons.pop();
        }
        // Single hit-target list shared with update() — single source of truth.
        let items = Self::flat_items(ctx.layout, can_skip);
        self.tree.register_flat_buttons(&items, &mut buttons);

        // Pause overlay.
        self.pause_menu
            .draw(w, h, scale, &mut instances, &mut text_labels, &mut buttons);

        SceneDrawOutput {
            background: Default::default(),
            tray_instances: vec![],
            instances,
            hand_tiles: vec![],
            hand_slots: vec![],
            focus: 0,
            selected_tiles: vec![],
            text_labels,
            relic_icons,
            buttons,
            window_title: format!(
                "Next Blind: {}    Enter play  {}",
                upcoming.name(),
                if can_skip { "Esc skip" } else { "(must play)" },
            ),
            departing_indices: vec![],
            hint_indices: vec![],
            flame_instances: vec![],
            point_lights: vec![],
            candles: vec![],
            relic_placements: vec![],
            draw_table: false,
            wind_gusts: Vec::new(),
        }
    }
}
