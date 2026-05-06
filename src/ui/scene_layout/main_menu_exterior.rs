use serde::{Deserialize, Serialize};

use crate::ui::placement::{ArrangeTarget, Node, Placement};

use super::fs::{load_positions, sanitize_placements, save_positions};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MainMenuExteriorPositions {
    pub door_hit: Placement,
    pub sign_hit: Placement,
    pub bike_hit: Placement,
    pub play_label: Placement,
    pub options_label: Placement,
    pub quit_label: Placement,
    pub sign_title: Placement,
    pub sign_body: Placement,
}

impl Default for MainMenuExteriorPositions {
    fn default() -> Self {
        Self {
            // Tuned for `assets/backgrounds/main_menu_exterior.png` (no-text 3D mockup).
            door_hit: Placement::at(0.54, 0.445, 180.0),
            // Options: wall beside the doorway (lantern / plaster), not the blank board above.
            sign_hit: Placement::at(0.375, 0.435, 170.0),
            bike_hit: Placement::at(0.775, 0.605, 140.0),
            // Label anchors — tuned to the current exterior plate (see shipped `main_menu_exterior.json`).
            play_label: Placement::at(0.613206, 0.4833688, 0.0),
            options_label: Placement::at(0.7578125, 0.4855285, 0.0),
            quit_label: Placement::at(0.83287036, 0.8056426, 0.0),
            // Reserved for a future diegetic title strip on the blank board above the door.
            sign_title: Placement::at(0.54, 0.22, 0.0),
            sign_body: Placement::at(0.54, 0.27, 0.0),
        }
    }
}

pub const MAIN_MENU_EXTERIOR_HIERARCHY: &[Node] = &[Node::Group {
    name: "main_menu_exterior",
    label: "Main menu (exterior)",
    children: &[
        Node::Leaf {
            name: "main_menu_exterior.door_hit",
            label: "Play — doorway hit",
        },
        Node::Leaf {
            name: "main_menu_exterior.sign_hit",
            label: "Options — wall beside doorway",
        },
        Node::Leaf {
            name: "main_menu_exterior.bike_hit",
            label: "Quit — bicycle hit",
        },
        Node::Leaf {
            name: "main_menu_exterior.play_label",
            label: "PLAY label (doorway)",
        },
        Node::Leaf {
            name: "main_menu_exterior.options_label",
            label: "OPTIONS label (beside doorway)",
        },
        Node::Leaf {
            name: "main_menu_exterior.quit_label",
            label: "QUIT label (over bicycle)",
        },
        Node::Leaf {
            name: "main_menu_exterior.sign_title",
            label: "Reserved — blank board title band",
        },
        Node::Leaf {
            name: "main_menu_exterior.sign_body",
            label: "Reserved — blank board body band",
        },
    ],
}];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainMenuExteriorField {
    DoorHit,
    SignHit,
    BikeHit,
    PlayLabel,
    OptionsLabel,
    QuitLabel,
    SignTitle,
    SignBody,
}

pub fn lookup_main_menu_exterior_field(name: &str) -> Option<MainMenuExteriorField> {
    Some(match name {
        "main_menu_exterior.door_hit" => MainMenuExteriorField::DoorHit,
        "main_menu_exterior.sign_hit" => MainMenuExteriorField::SignHit,
        "main_menu_exterior.bike_hit" => MainMenuExteriorField::BikeHit,
        "main_menu_exterior.play_label" => MainMenuExteriorField::PlayLabel,
        "main_menu_exterior.options_label" => MainMenuExteriorField::OptionsLabel,
        "main_menu_exterior.quit_label" => MainMenuExteriorField::QuitLabel,
        "main_menu_exterior.sign_title" => MainMenuExteriorField::SignTitle,
        "main_menu_exterior.sign_body" => MainMenuExteriorField::SignBody,
        _ => return None,
    })
}

#[cfg(test)]
pub fn main_menu_exterior_field_path(field: MainMenuExteriorField) -> &'static str {
    match field {
        MainMenuExteriorField::DoorHit => "main_menu_exterior.door_hit",
        MainMenuExteriorField::SignHit => "main_menu_exterior.sign_hit",
        MainMenuExteriorField::BikeHit => "main_menu_exterior.bike_hit",
        MainMenuExteriorField::PlayLabel => "main_menu_exterior.play_label",
        MainMenuExteriorField::OptionsLabel => "main_menu_exterior.options_label",
        MainMenuExteriorField::QuitLabel => "main_menu_exterior.quit_label",
        MainMenuExteriorField::SignTitle => "main_menu_exterior.sign_title",
        MainMenuExteriorField::SignBody => "main_menu_exterior.sign_body",
    }
}

impl MainMenuExteriorField {
    pub const ALL: &'static [MainMenuExteriorField] = &[
        MainMenuExteriorField::DoorHit,
        MainMenuExteriorField::SignHit,
        MainMenuExteriorField::BikeHit,
        MainMenuExteriorField::PlayLabel,
        MainMenuExteriorField::OptionsLabel,
        MainMenuExteriorField::QuitLabel,
        MainMenuExteriorField::SignTitle,
        MainMenuExteriorField::SignBody,
    ];
}

impl MainMenuExteriorPositions {
    pub fn field_mut(&mut self, field: MainMenuExteriorField) -> &mut Placement {
        match field {
            MainMenuExteriorField::DoorHit => &mut self.door_hit,
            MainMenuExteriorField::SignHit => &mut self.sign_hit,
            MainMenuExteriorField::BikeHit => &mut self.bike_hit,
            MainMenuExteriorField::PlayLabel => &mut self.play_label,
            MainMenuExteriorField::OptionsLabel => &mut self.options_label,
            MainMenuExteriorField::QuitLabel => &mut self.quit_label,
            MainMenuExteriorField::SignTitle => &mut self.sign_title,
            MainMenuExteriorField::SignBody => &mut self.sign_body,
        }
    }

    pub fn field_ref(&self, field: MainMenuExteriorField) -> &Placement {
        match field {
            MainMenuExteriorField::DoorHit => &self.door_hit,
            MainMenuExteriorField::SignHit => &self.sign_hit,
            MainMenuExteriorField::BikeHit => &self.bike_hit,
            MainMenuExteriorField::PlayLabel => &self.play_label,
            MainMenuExteriorField::OptionsLabel => &self.options_label,
            MainMenuExteriorField::QuitLabel => &self.quit_label,
            MainMenuExteriorField::SignTitle => &self.sign_title,
            MainMenuExteriorField::SignBody => &self.sign_body,
        }
    }
}

impl ArrangeTarget for MainMenuExteriorPositions {
    fn placement_mut(&mut self, name: &str) -> Option<&mut Placement> {
        lookup_main_menu_exterior_field(name).map(|f| self.field_mut(f))
    }

    fn placement(&self, name: &str) -> Option<&Placement> {
        lookup_main_menu_exterior_field(name).map(|f| self.field_ref(f))
    }

    fn hierarchy(&self) -> &'static [Node] {
        MAIN_MENU_EXTERIOR_HIERARCHY
    }
}

pub fn load_main_menu_exterior_positions() -> MainMenuExteriorPositions {
    let mut loaded = load_positions("main_menu_exterior.json");
    sanitize_main_menu_exterior_positions(&mut loaded);
    loaded
}

pub fn sanitize_main_menu_exterior_positions(p: &mut MainMenuExteriorPositions) {
    sanitize_placements(
        "main_menu_exterior",
        p,
        MainMenuExteriorField::ALL,
        |positions, field| positions.field_mut(field),
    );
}

pub fn save_main_menu_exterior_positions(pos: &MainMenuExteriorPositions) -> anyhow::Result<()> {
    save_positions("main_menu_exterior.json", "main_menu_exterior", pos)
}
