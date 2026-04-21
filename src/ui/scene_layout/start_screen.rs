use serde::{Deserialize, Serialize};

use crate::ui::placement::{ArrangeTarget, Node, Placement};

use super::fs::{load_positions, sanitize_placements, save_positions};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StartScreenPositions {
    pub menu_tablets: Placement,
    pub candle_left: Placement,
    pub candle_right: Placement,
    pub title_plaque: Placement,
}

impl Default for StartScreenPositions {
    fn default() -> Self {
        Self {
            menu_tablets: Placement::at(0.0, 0.0, 0.0),
            candle_left: Placement::at(0.0, 0.0, 0.0),
            candle_right: Placement::at(0.0, 0.0, 0.0),
            title_plaque: Placement::at(0.0, 0.0, 0.0),
        }
    }
}

pub const START_SCREEN_HIERARCHY: &[Node] = &[Node::Group {
    name: "start_screen",
    label: "Start screen",
    children: &[
        Node::Leaf {
            name: "start_screen.menu_tablets",
            label: "Menu tablet column",
        },
        Node::Leaf {
            name: "start_screen.candle_left",
            label: "Candle (left)",
        },
        Node::Leaf {
            name: "start_screen.candle_right",
            label: "Candle (right)",
        },
        Node::Leaf {
            name: "start_screen.title_plaque",
            label: "Title plaque",
        },
    ],
}];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartScreenField {
    MenuTablets,
    CandleLeft,
    CandleRight,
    TitlePlaque,
}

pub fn lookup_start_screen_field(name: &str) -> Option<StartScreenField> {
    Some(match name {
        "start_screen.menu_tablets" => StartScreenField::MenuTablets,
        "start_screen.candle_left" => StartScreenField::CandleLeft,
        "start_screen.candle_right" => StartScreenField::CandleRight,
        "start_screen.title_plaque" => StartScreenField::TitlePlaque,
        _ => return None,
    })
}

#[cfg(test)]
pub fn start_screen_field_path(field: StartScreenField) -> &'static str {
    match field {
        StartScreenField::MenuTablets => "start_screen.menu_tablets",
        StartScreenField::CandleLeft => "start_screen.candle_left",
        StartScreenField::CandleRight => "start_screen.candle_right",
        StartScreenField::TitlePlaque => "start_screen.title_plaque",
    }
}

impl StartScreenField {
    pub const ALL: &'static [StartScreenField] = &[
        StartScreenField::MenuTablets,
        StartScreenField::CandleLeft,
        StartScreenField::CandleRight,
        StartScreenField::TitlePlaque,
    ];
}

impl StartScreenPositions {
    pub fn field_mut(&mut self, field: StartScreenField) -> &mut Placement {
        match field {
            StartScreenField::MenuTablets => &mut self.menu_tablets,
            StartScreenField::CandleLeft => &mut self.candle_left,
            StartScreenField::CandleRight => &mut self.candle_right,
            StartScreenField::TitlePlaque => &mut self.title_plaque,
        }
    }

    pub fn field_ref(&self, field: StartScreenField) -> &Placement {
        match field {
            StartScreenField::MenuTablets => &self.menu_tablets,
            StartScreenField::CandleLeft => &self.candle_left,
            StartScreenField::CandleRight => &self.candle_right,
            StartScreenField::TitlePlaque => &self.title_plaque,
        }
    }
}

impl ArrangeTarget for StartScreenPositions {
    fn placement_mut(&mut self, name: &str) -> Option<&mut Placement> {
        lookup_start_screen_field(name).map(|f| self.field_mut(f))
    }

    fn placement(&self, name: &str) -> Option<&Placement> {
        lookup_start_screen_field(name).map(|f| self.field_ref(f))
    }

    fn hierarchy(&self) -> &'static [Node] {
        START_SCREEN_HIERARCHY
    }
}

pub fn load_start_screen_positions() -> StartScreenPositions {
    let mut loaded = load_positions("start_screen.json");
    sanitize_start_screen_positions(&mut loaded);
    loaded
}

pub fn sanitize_start_screen_positions(p: &mut StartScreenPositions) {
    sanitize_placements(
        "start_screen",
        p,
        StartScreenField::ALL,
        |positions, field| positions.field_mut(field),
    );
}

pub fn save_start_screen_positions(pos: &StartScreenPositions) -> anyhow::Result<()> {
    save_positions("start_screen.json", "start_screen", pos)
}
