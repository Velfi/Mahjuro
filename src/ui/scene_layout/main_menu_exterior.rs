use crate::ui::placement::Placement;

#[derive(Clone, Debug)]
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
            // Legacy arrange anchors from the flat exterior mockup; unused by the GLB hub UI.
            door_hit: Placement::at(0.54, 0.445, 180.0),
            // Options: wall beside the doorway (lantern / plaster), not the blank board above.
            sign_hit: Placement::at(0.375, 0.435, 170.0),
            bike_hit: Placement::at(0.775, 0.605, 140.0),
            // Label anchors — tuned to the current exterior plate.
            play_label: Placement::at(0.613206, 0.4833688, 0.0),
            options_label: Placement::at(0.7578125, 0.4855285, 0.0),
            quit_label: Placement::at(0.83287036, 0.8056426, 0.0),
            // Reserved for a future diegetic title strip on the blank board above the door.
            sign_title: Placement::at(0.54, 0.22, 0.0),
            sign_body: Placement::at(0.54, 0.27, 0.0),
        }
    }
}

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
