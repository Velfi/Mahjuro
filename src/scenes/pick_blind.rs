//! Pick-blind scene — Balatro-style: shows all three blinds for the
//! current ante (Small / Big / Boss) at once, with the upcoming one
//! highlighted. Already-cleared blinds are dimmed; future blinds preview
//! their target so the player can plan around the boss before reaching it.
//!
//! The boss card always shows its themed name and effect, since
//! `RunState::upcoming_boss` is rolled at the ante boundary.

use crate::core::rules::BlindKind;
use crate::render::theme::{ButtonState, ButtonVariant, color, typography};
use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
use crate::ui::input::UiAction;
use crate::ui::layout::LayoutResult;
use crate::ui::widget::{self, PanelVariant};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::gameplay::GameplayScene;
use super::pause_menu::PauseMenu;
use super::{
    DrawCtx, Scene, SceneBehavior, SceneDrawOutput, SceneTransition, UpdateCtx, relic_row,
};

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

/// Visual state of one of the three blind cards in the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CardState {
    /// Already played and cleared this ante. Drawn dim with a "DONE" tag.
    Cleared,
    /// The blind the player is about to face — has the focus halo.
    Upcoming,
    /// Future blind in this ante — drawn dim, used for preview only.
    Future,
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

    /// Compute the card state for `card` given the player's `upcoming` blind.
    /// Cards earlier in the Small → Big → Boss cycle are Cleared; the
    /// upcoming one is Upcoming; later ones are Future.
    fn card_state(card: BlindKind, upcoming: BlindKind) -> CardState {
        let order = |b: BlindKind| match b {
            BlindKind::Small => 0,
            BlindKind::Big => 1,
            BlindKind::Boss => 2,
        };
        let c = order(card);
        let u = order(upcoming);
        if c < u {
            CardState::Cleared
        } else if c == u {
            CardState::Upcoming
        } else {
            CardState::Future
        }
    }

    /// Layout: three card rects in a row, plus the skip button rect.
    /// The upcoming card sits in the centre slot so the player's eye lands on
    /// the choice they're actually making, not on the blind that already passed.
    fn layout_cards(layout: &LayoutResult) -> [[f32; 4]; 3] {
        let hs = layout.hand_strip;
        let pad_y = hs.h * 0.08;
        let card_h = hs.h - pad_y * 2.0;
        let gap = hs.w * 0.02;
        let card_w = (hs.w - gap * 4.0) / 3.0;
        let card_y = hs.y + pad_y;
        let row_w = card_w * 3.0 + gap * 2.0;
        let row_x = hs.x + (hs.w - row_w) * 0.5;
        [
            [row_x, card_y, card_w, card_h],
            [row_x + card_w + gap, card_y, card_w, card_h],
            [row_x + (card_w + gap) * 2.0, card_y, card_w, card_h],
        ]
    }

    /// Hit-test rects shared between update() and draw(). The play target is
    /// always the upcoming card (whichever slot it's in), so the focus tree
    /// has just one card click target plus the skip button.
    fn flat_items(layout: &LayoutResult, upcoming: BlindKind, can_skip: bool) -> Vec<FlatItem<BlindAction>> {
        let cards = Self::layout_cards(layout);
        let upcoming_idx = match upcoming {
            BlindKind::Small => 0,
            BlindKind::Big => 1,
            BlindKind::Boss => 2,
        };
        let w = layout.window_w;
        let h = layout.window_h;
        let scale = (w.min(h)) / 600.0;
        let btn_w = (160.0 * scale).max(80.0);
        let btn_h = (38.0 * scale).max(24.0);
        let btn_x = (w - btn_w) * 0.5;
        let btn_y = h - btn_h - (12.0 * scale);

        let mut items = vec![FlatItem::new(
            BlindAction::PlayBlind.id(),
            cards[upcoming_idx],
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
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            return t;
        }

        let upcoming = ctx.run.upcoming_blind;
        let can_skip = Self::can_skip(upcoming);

        let items = Self::flat_items(ctx.layout, upcoming, can_skip);
        let action = self.tree.update_flat(
            &items,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
            },
        );

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

        let mut instances = vec![GpuInstance {
            rect: [0.0, 0.0, w, h],
            color: color::OBSIDIAN,
        }];

        let (relic_insts, relic_labels, relic_icons) =
            relic_row(&ctx.run.relics, &ctx.layout.relic_strip, w);
        instances.extend(relic_insts);

        let upcoming = ctx.run.upcoming_blind;
        let can_skip = Self::can_skip(upcoming);
        let cards = Self::layout_cards(ctx.layout);
        let blinds = [BlindKind::Small, BlindKind::Big, BlindKind::Boss];
        let base = ctx.run.base_target;
        let gold = ctx.run.gold;

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
        let mut text_labels = vec![TextLabel {
            rect: [
                ctx.layout.score_panel.x,
                ctx.layout.score_panel.y,
                ctx.layout.score_panel.w,
                ctx.layout.score_panel.h,
            ],
            text: format!(
                "ANTE {}/{}   ·   Gold {}",
                ctx.run.ante,
                crate::game::run::FINAL_ANTE,
                gold
            ),
            color: color::CHAMPAGNE,
            ..Default::default()
        }];
        text_labels.extend(relic_labels);

        // Subtitle / instruction strip.
        let ms = ctx.layout.modifier_strip;
        text_labels.push(TextLabel {
            rect: [ms.x, ms.y, ms.w, ms.h],
            text: format!(
                "Choose: {} is up — Enter play   {}",
                upcoming.name(),
                if can_skip { "Esc/↓ skip" } else { "" }
            ),
            color: color::PARCHMENT,
            ..Default::default()
        });

        let skip_focused = self.skip_focused();

        // ── Render the three blind cards ──────────────────────────────
        for (i, &card_blind) in blinds.iter().enumerate() {
            let rect = cards[i];
            let state = Self::card_state(card_blind, upcoming);
            let [cx, cy, cw, ch] = rect;

            // Halo: only the upcoming card gets one. Boss bosses get a
            // tier-tinted halo so the player can read severity at a glance.
            if state == CardState::Upcoming && !skip_focused {
                let halo = ch * 0.04;
                let halo_color = if card_blind == BlindKind::Boss {
                    if let Some(kind) = ctx.run.upcoming_boss {
                        color::alpha(kind.tier().halo_color(), 0.55)
                    } else {
                        color::alpha(color::GOLD, 0.45)
                    }
                } else {
                    color::alpha(color::GOLD, 0.45)
                };
                instances.push(GpuInstance {
                    rect: [cx - halo, cy - halo, cw + halo * 2.0, ch + halo * 2.0],
                    color: halo_color,
                });
            }

            // Panel variant: boss → Hero (gold border), others → Default.
            // Cleared cards drop to Sunken so they read as "done."
            let variant = match (state, card_blind) {
                (CardState::Cleared, _) => PanelVariant::Sunken,
                (_, BlindKind::Boss) => PanelVariant::Hero,
                _ => PanelVariant::Default,
            };
            widget::push_panel(&mut instances, rect, variant);

            // Per-state text colors so dim cards visibly recede.
            let title_color = match state {
                CardState::Upcoming => color::CHAMPAGNE,
                CardState::Cleared => color::SLATE,
                CardState::Future => color::MIST,
            };
            let body_color = match state {
                CardState::Upcoming => color::PARCHMENT,
                CardState::Cleared => color::SLATE,
                CardState::Future => color::MIST,
            };

            // Card title — boss card prefers the themed name.
            let title_text: String = if card_blind == BlindKind::Boss {
                ctx.run
                    .upcoming_boss
                    .map(|k| k.def().name.to_string())
                    .unwrap_or_else(|| "Boss Blind".to_string())
            } else {
                card_blind.name().to_string()
            };
            let title_h = typography::size(typography::HEADING, h) * 1.8;
            let title_y = cy + ch * 0.10;
            text_labels.push(TextLabel {
                rect: [cx, title_y, cw, title_h],
                text: title_text,
                color: title_color,
                ..Default::default()
            });

            // Target chip count (each card shows its own derived target).
            let target = (base as f32 * card_blind.target_multiplier()) as u32;
            let target_h = typography::size(typography::CAPTION, h) * 1.8;
            let target_y = cy + ch * 0.34;
            text_labels.push(TextLabel {
                rect: [cx, target_y, cw, target_h],
                text: format!("Target {}", target),
                color: body_color,
                ..Default::default()
            });

            // Boss-only: name + effect description on the boss card.
            if card_blind == BlindKind::Boss {
                if let Some(kind) = ctx.run.upcoming_boss {
                    let def = kind.def();
                    // Reactive bosses (Mirror, Tax Collector) override the
                    // static description with the variant chosen at reveal
                    // time, so the player sees the *actual* rule before
                    // they ever fight it. Static bosses fall through.
                    let description: &str = ctx
                        .run
                        .upcoming_boss_effect
                        .as_ref()
                        .and_then(|e| e.description_override.as_deref())
                        .unwrap_or(def.description);
                    let desc_h = typography::size(typography::CAPTION, h) * 1.8;
                    let desc_y = cy + ch * 0.50;
                    text_labels.push(TextLabel {
                        rect: [cx, desc_y, cw, desc_h],
                        text: description.to_string(),
                        color: color::AMBER,
                        ..Default::default()
                    });
                    let tier_y = cy + ch * 0.66;
                    text_labels.push(TextLabel {
                        rect: [cx, tier_y, cw, desc_h],
                        text: format!("[{}]", def.tier.label()),
                        color: def.tier.halo_color(),
                        ..Default::default()
                    });
                }
            } else {
                // Reward summary for non-boss cards.
                let reward_h = typography::size(typography::CAPTION, h) * 1.8;
                let reward_y = cy + ch * 0.50;
                text_labels.push(TextLabel {
                    rect: [cx, reward_y, cw, reward_h],
                    text: format!("×{:.2} gold", card_blind.gold_multiplier()),
                    color: body_color,
                    ..Default::default()
                });
            }

            // State stamp at the bottom of the card.
            let stamp_h = typography::size(typography::CAPTION, h) * 1.6;
            let stamp_y = cy + ch * 0.83;
            let stamp_text = match state {
                CardState::Cleared => "DONE",
                CardState::Upcoming => "▶ NEXT",
                CardState::Future => "Coming up",
            };
            let stamp_color = match state {
                CardState::Cleared => color::SLATE,
                CardState::Upcoming => color::GOLD,
                CardState::Future => color::MIST,
            };
            text_labels.push(TextLabel {
                rect: [cx, stamp_y, cw, stamp_h],
                text: stamp_text.to_string(),
                color: stamp_color,
                ..Default::default()
            });
        }

        // Round wind line (kept from the prior layout — drives Yakuhai
        // planning before the player commits to a hand).
        let wind_rank = BlindKind::round_wind_for_ante(ctx.run.ante);
        let wind_strip = ctx.layout.hand_strip;
        let wind_h = typography::size(typography::CAPTION, h) * 1.8;
        let wind_y = wind_strip.y + wind_strip.h - wind_h - 6.0;
        text_labels.push(TextLabel {
            rect: [wind_strip.x, wind_y, wind_strip.w, wind_h],
            text: format!("Round Wind: {}", BlindKind::wind_name(wind_rank)),
            color: color::GOLD,
            ..Default::default()
        });

        // Skip button at bottom (Small/Big only).
        let scale = (w.min(h)) / 600.0;
        let btn_w = (160.0 * scale).max(80.0);
        let btn_h = (38.0 * scale).max(24.0);
        let btn_x = (w - btn_w) * 0.5;
        let btn_y = h - btn_h - (12.0 * scale);

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
            buttons.pop();
        }
        let items = Self::flat_items(ctx.layout, upcoming, can_skip);
        self.tree.register_flat_buttons(&items, &mut buttons);

        // While paused, drop the scene's own buttons so only the pause
        // menu's own clickable surfaces survive into `frame.buttons`.
        if self.pause_menu.paused {
            buttons.clear();
        }
        self.pause_menu
            .draw(w, h, scale, &mut instances, &mut text_labels, &mut buttons);
        if self.pause_menu.paused {
            buttons.push(crate::scenes::ButtonDef::scene((0.0, 0.0, w, h), u32::MAX));
        }

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
