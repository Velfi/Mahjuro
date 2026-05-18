use serde::{Deserialize, Serialize};

use crate::ui::placement::{ArrangeTarget, Node, Placement};

use super::fs::{load_positions, sanitize_placements, save_positions};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GameplayPositions {
    pub relic_col: Placement,
    pub relic_col_top_ny: f32,
    pub relic_col_bottom_ny: f32,
    pub relic_cell_height_mm: f32,
    pub plaque: Placement,
    pub counter_draws_fan: Placement,
    pub counter_discards_fan: Placement,
    pub ofuda: Placement,
    pub coin_pile: Placement,
    pub dora: Placement,
    pub round_wind: Placement,
    pub talisman_dish: Placement,
    pub consumable_dish_talisman: Placement,
    pub bowl: Placement,
    pub mirror: Placement,
    pub tablet_cash_in: Placement,
    pub tablet_journal: Placement,
    pub candle_back_z_push_candle_w_frac: f32,
    pub candle_bottom_z_back_candle_h_frac: f32,
    pub hand_strip: Placement,
    pub yaku_tablet: Placement,
    pub camera_eye_y_mul: f32,
    pub camera_eye_z_mul: f32,
    pub camera_target_y_mul: f32,
    pub camera_target_z_mul: f32,
    pub camera_fovy_deg: f32,
}

impl Default for GameplayPositions {
    fn default() -> Self {
        Self {
            relic_col: Placement::at(-0.950_520_9, 0.191_863_13, 2.144_608),
            relic_col_top_ny: 0.22,
            relic_col_bottom_ny: 0.78,
            relic_cell_height_mm: 42.0,
            plaque: Placement {
                nx: 0.0,
                ny: 0.18,
                lift_mm: 139.161_16,
                rx_deg: 0.0,
                ry_deg: 0.0,
                rz_deg: 0.0,
            },
            counter_draws_fan: Placement {
                nx: -0.042_534_72,
                ny: -0.036_121_674,
                lift_mm: 0.744_655_5,
                rx_deg: 0.0,
                ry_deg: 0.0,
                rz_deg: -30.0,
            },
            counter_discards_fan: Placement {
                nx: 0.084_490_73,
                ny: 0.065_589_31,
                lift_mm: 19.361_042,
                rx_deg: 0.0,
                ry_deg: 0.0,
                rz_deg: 45.0,
            },
            ofuda: Placement {
                nx: 0.002_893_518_4,
                ny: 0.0,
                lift_mm: -35.981_754,
                rx_deg: -69.0,
                ry_deg: 0.0,
                rz_deg: 0.0,
            },
            coin_pile: Placement::at(1.179_050_6, 0.13, 2.144_608),
            dora: Placement {
                nx: 0.765_231_4,
                ny: -0.540_456_24,
                lift_mm: 2.144_608_5,
                rx_deg: 0.0,
                ry_deg: 180.0,
                rz_deg: 180.0,
            },
            round_wind: Placement {
                nx: 0.645_231_4,
                ny: -0.540_456_24,
                lift_mm: 2.144_608_5,
                rx_deg: 0.0,
                ry_deg: 180.0,
                rz_deg: 180.0,
            },
            talisman_dish: Placement::at(-0.137_442_13, -0.336_977_2, -3.365_842_3),
            consumable_dish_talisman: Placement {
                nx: -0.008_575_439,
                ny: 0.049_166_66,
                lift_mm: 1.489_310_9,
                rx_deg: 68.0,
                ry_deg: -90.0,
                rz_deg: 0.0,
            },
            bowl: Placement {
                nx: -0.051_770_832,
                ny: -0.115_193_285,
                lift_mm: -32.109_547,
                rx_deg: 0.0,
                ry_deg: 0.0,
                rz_deg: -315.0,
            },
            mirror: Placement {
                nx: -0.006_076_388_5,
                ny: 0.023_764_258,
                lift_mm: 1.995_676_9,
                rx_deg: -159.0,
                ry_deg: 75.0,
                rz_deg: -78.0,
            },
            tablet_cash_in: Placement::at(0.0, 0.0, 2.144_608),
            tablet_journal: Placement::at(0.0, -0.064_638_78, 2.144_608),
            candle_back_z_push_candle_w_frac: 1.0,
            candle_bottom_z_back_candle_h_frac: 0.55,
            hand_strip: Placement {
                nx: 0.0,
                ny: 0.221_292_81,
                lift_mm: 36.458_336,
                rx_deg: -12.0,
                ry_deg: 0.0,
                rz_deg: 0.0,
            },
            yaku_tablet: Placement {
                nx: 0.0,
                ny: 0.155_893_97,
                lift_mm: 19.301_472,
                rx_deg: -20.0,
                ry_deg: 0.0,
                rz_deg: 0.0,
            },
            camera_eye_y_mul: 1.0,
            camera_eye_z_mul: 1.0,
            camera_target_y_mul: 1.0,
            camera_target_z_mul: 1.0,
            camera_fovy_deg: 55.0,
        }
    }
}

pub const GAMEPLAY_HIERARCHY: &[Node] = &[Node::Group {
    name: "gameplay",
    label: "Gameplay",
    children: &[
        Node::Group {
            name: "gameplay.hand",
            label: "Hand area",
            children: &[
                Node::Leaf {
                    name: "gameplay.hand.strip",
                    label: "Hand strip",
                },
                Node::Leaf {
                    name: "gameplay.hand.yaku_tablet",
                    label: "Yaku tablet row",
                },
            ],
        },
        Node::Group {
            name: "gameplay.score_panel",
            label: "Score panel",
            children: &[
                Node::Leaf {
                    name: "gameplay.score_panel.plaque",
                    label: "Blind plaque",
                },
                Node::Leaf {
                    name: "gameplay.score_panel.ofuda",
                    label: "Boss-rule ofuda",
                },
                Node::Leaf {
                    name: "gameplay.score_panel.coin_pile",
                    label: "Coin pile",
                },
            ],
        },
        Node::Group {
            name: "gameplay.action_bar",
            label: "Action bar",
            children: &[
                Node::Leaf {
                    name: "gameplay.action_bar.bowl",
                    label: "Discard bowl",
                },
                Node::Leaf {
                    name: "gameplay.action_bar.mirror",
                    label: "Bronze mirror",
                },
                Node::Leaf {
                    name: "gameplay.action_bar.tablet_cash_in",
                    label: "Tablet — Cash in",
                },
                Node::Leaf {
                    name: "gameplay.action_bar.tablet_journal",
                    label: "Tablet — Journal",
                },
            ],
        },
        Node::Group {
            name: "gameplay.counter",
            label: "Counter fans",
            children: &[
                Node::Leaf {
                    name: "gameplay.counter.draws_fan",
                    label: "Draws fan",
                },
                Node::Leaf {
                    name: "gameplay.counter.discards_fan",
                    label: "Discards fan",
                },
            ],
        },
        Node::Leaf {
            name: "gameplay.relic_col",
            label: "Relic tray (horizontal)",
        },
        Node::Leaf {
            name: "gameplay.dora",
            label: "Dora",
        },
        Node::Leaf {
            name: "gameplay.round_wind",
            label: "Round wind",
        },
        Node::Leaf {
            name: "gameplay.talisman_dish",
            label: "Talisman dish",
        },
        Node::Leaf {
            name: "gameplay.consumable_dish.talisman",
            label: "Talisman pendant",
        },
    ],
}];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameplayField {
    RelicCol,
    Plaque,
    CounterDrawsFan,
    CounterDiscardsFan,
    Ofuda,
    CoinPile,
    Dora,
    RoundWind,
    Bowl,
    Mirror,
    HandStrip,
    YakuTablet,
    TabletCashIn,
    TabletJournal,
    TalismanDish,
    ConsumableDishTalisman,
}

pub fn lookup_gameplay_field(name: &str) -> Option<GameplayField> {
    Some(match name {
        "gameplay.relic_col" => GameplayField::RelicCol,
        "gameplay.score_panel.plaque" => GameplayField::Plaque,
        "gameplay.counter.draws_fan" => GameplayField::CounterDrawsFan,
        "gameplay.counter.discards_fan" => GameplayField::CounterDiscardsFan,
        "gameplay.score_panel.ofuda" => GameplayField::Ofuda,
        "gameplay.score_panel.coin_pile" => GameplayField::CoinPile,
        "gameplay.dora" => GameplayField::Dora,
        "gameplay.round_wind" => GameplayField::RoundWind,
        "gameplay.action_bar.bowl" => GameplayField::Bowl,
        "gameplay.action_bar.mirror" => GameplayField::Mirror,
        "gameplay.action_bar.tablet_cash_in" => GameplayField::TabletCashIn,
        "gameplay.action_bar.tablet_journal" => GameplayField::TabletJournal,
        "gameplay.hand.strip" => GameplayField::HandStrip,
        "gameplay.hand.yaku_tablet" => GameplayField::YakuTablet,
        "gameplay.talisman_dish" => GameplayField::TalismanDish,
        "gameplay.consumable_dish.talisman" => GameplayField::ConsumableDishTalisman,
        _ => return None,
    })
}

#[cfg(test)]
pub fn gameplay_field_path(field: GameplayField) -> &'static str {
    match field {
        GameplayField::RelicCol => "gameplay.relic_col",
        GameplayField::Plaque => "gameplay.score_panel.plaque",
        GameplayField::CounterDrawsFan => "gameplay.counter.draws_fan",
        GameplayField::CounterDiscardsFan => "gameplay.counter.discards_fan",
        GameplayField::Ofuda => "gameplay.score_panel.ofuda",
        GameplayField::CoinPile => "gameplay.score_panel.coin_pile",
        GameplayField::Dora => "gameplay.dora",
        GameplayField::RoundWind => "gameplay.round_wind",
        GameplayField::Bowl => "gameplay.action_bar.bowl",
        GameplayField::Mirror => "gameplay.action_bar.mirror",
        GameplayField::TabletCashIn => "gameplay.action_bar.tablet_cash_in",
        GameplayField::TabletJournal => "gameplay.action_bar.tablet_journal",
        GameplayField::HandStrip => "gameplay.hand.strip",
        GameplayField::YakuTablet => "gameplay.hand.yaku_tablet",
        GameplayField::TalismanDish => "gameplay.talisman_dish",
        GameplayField::ConsumableDishTalisman => "gameplay.consumable_dish.talisman",
    }
}

impl GameplayField {
    pub const ALL: &'static [GameplayField] = &[
        GameplayField::RelicCol,
        GameplayField::Plaque,
        GameplayField::CounterDrawsFan,
        GameplayField::CounterDiscardsFan,
        GameplayField::Ofuda,
        GameplayField::CoinPile,
        GameplayField::Dora,
        GameplayField::RoundWind,
        GameplayField::Bowl,
        GameplayField::Mirror,
        GameplayField::HandStrip,
        GameplayField::YakuTablet,
        GameplayField::TabletCashIn,
        GameplayField::TabletJournal,
        GameplayField::TalismanDish,
        GameplayField::ConsumableDishTalisman,
    ];
}

impl GameplayPositions {
    pub fn field_mut(&mut self, field: GameplayField) -> &mut Placement {
        match field {
            GameplayField::RelicCol => &mut self.relic_col,
            GameplayField::Plaque => &mut self.plaque,
            GameplayField::CounterDrawsFan => &mut self.counter_draws_fan,
            GameplayField::CounterDiscardsFan => &mut self.counter_discards_fan,
            GameplayField::Ofuda => &mut self.ofuda,
            GameplayField::CoinPile => &mut self.coin_pile,
            GameplayField::Dora => &mut self.dora,
            GameplayField::RoundWind => &mut self.round_wind,
            GameplayField::Bowl => &mut self.bowl,
            GameplayField::Mirror => &mut self.mirror,
            GameplayField::HandStrip => &mut self.hand_strip,
            GameplayField::YakuTablet => &mut self.yaku_tablet,
            GameplayField::TabletCashIn => &mut self.tablet_cash_in,
            GameplayField::TabletJournal => &mut self.tablet_journal,
            GameplayField::TalismanDish => &mut self.talisman_dish,
            GameplayField::ConsumableDishTalisman => &mut self.consumable_dish_talisman,
        }
    }

    pub fn field_ref(&self, field: GameplayField) -> &Placement {
        match field {
            GameplayField::RelicCol => &self.relic_col,
            GameplayField::Plaque => &self.plaque,
            GameplayField::CounterDrawsFan => &self.counter_draws_fan,
            GameplayField::CounterDiscardsFan => &self.counter_discards_fan,
            GameplayField::Ofuda => &self.ofuda,
            GameplayField::CoinPile => &self.coin_pile,
            GameplayField::Dora => &self.dora,
            GameplayField::RoundWind => &self.round_wind,
            GameplayField::Bowl => &self.bowl,
            GameplayField::Mirror => &self.mirror,
            GameplayField::HandStrip => &self.hand_strip,
            GameplayField::YakuTablet => &self.yaku_tablet,
            GameplayField::TabletCashIn => &self.tablet_cash_in,
            GameplayField::TabletJournal => &self.tablet_journal,
            GameplayField::TalismanDish => &self.talisman_dish,
            GameplayField::ConsumableDishTalisman => &self.consumable_dish_talisman,
        }
    }
}

impl ArrangeTarget for GameplayPositions {
    fn placement_mut(&mut self, name: &str) -> Option<&mut Placement> {
        lookup_gameplay_field(name).map(|f| self.field_mut(f))
    }

    fn placement(&self, name: &str) -> Option<&Placement> {
        lookup_gameplay_field(name).map(|f| self.field_ref(f))
    }

    fn hierarchy(&self) -> &'static [Node] {
        GAMEPLAY_HIERARCHY
    }
}

pub fn load_gameplay_positions() -> GameplayPositions {
    let mut loaded = load_positions("gameplay.json");
    sanitize_gameplay_positions(&mut loaded);
    loaded
}

pub fn sanitize_gameplay_positions(p: &mut GameplayPositions) {
    sanitize_placements("gameplay", p, GameplayField::ALL, |positions, field| {
        positions.field_mut(field)
    });
}

pub fn save_gameplay_positions(pos: &GameplayPositions) -> anyhow::Result<()> {
    save_positions("gameplay.json", "gameplay", pos)
}
