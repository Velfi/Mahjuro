//! Resolve a controller-prompt glyph for a given [`UiAction`].
//!
//! Two-tier lookup:
//!
//! 1. **Steam Input** — when active, asks Steam for the glyph that matches the
//!    user's actual binding (rebinds, Switch button labels, Steam Deck-specific
//!    overlays, etc.). This is the path the player sees on a Steam Deck.
//!
//! 2. **Static atlas** — when Steam Input is unavailable (game launched outside
//!    Steam, `--no-steam`, init failure), falls back to a Kenney Input Prompts
//!    atlas keyed off the OS-detected [`GamepadStyle`]. Honours the player's
//!    `swap_ab` / `swap_xy` settings.
//!
//! Scenes always go through [`GlyphResolver::glyph_for`]; they never branch on
//! "is Steam Input active" themselves.

use crate::render::draw_cmd::PromptIconSource;
use crate::steam::SteamClient;
use crate::ui::button_prompts::GamepadStyle;
use crate::ui::input::UiAction;

/// Borrowing wrapper that captures everything needed to pick a glyph for the
/// active controller. Created in `App::draw` and passed via `DrawCtx`.
#[derive(Clone, Copy)]
pub struct GlyphResolver<'a> {
    steam: &'a SteamClient,
    style: GamepadStyle,
    swap_ab: bool,
    swap_xy: bool,
}

impl<'a> GlyphResolver<'a> {
    pub fn new(
        steam: &'a SteamClient,
        style: GamepadStyle,
        swap_ab: bool,
        swap_xy: bool,
    ) -> Self {
        Self {
            steam,
            style,
            swap_ab,
            swap_xy,
        }
    }

    /// Best-available glyph for `action`. Steam Input wins when it has one;
    /// otherwise falls back to the static [`PromptIconSource::AtlasSprite`].
    /// Returns `None` only when no fallback is defined for the action (most
    /// non-prompt actions).
    pub fn glyph_for(&self, action: UiAction) -> Option<PromptIconSource> {
        if let Some(path) = self.steam.glyph_path_for(action) {
            return Some(PromptIconSource::Filesystem(path));
        }
        static_glyph_sprite(self.style, action, self.swap_ab, self.swap_xy)
            .map(|(sheet, name)| PromptIconSource::AtlasSprite { sheet, name })
    }
}

const XBOX_SHEET: &str = "kenney_input-prompts/Xbox Series/xbox-series_sheet_double.png";
const PLAYSTATION_SHEET: &str =
    "kenney_input-prompts/PlayStation Series/playstation-series_sheet_double.png";
const SWITCH_SHEET: &str = "kenney_input-prompts/Nintendo Switch/nintendo-switch_sheet_double.png";

/// Logical face the action is rendered as after applying the player's
/// `swap_ab` / `swap_xy` preferences.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Face {
    /// Bottom face (Xbox A, PS Cross, Switch B by label / Switch position B).
    South,
    /// Right face (Xbox B, PS Circle, Switch A by label).
    East,
    /// Left face (Xbox X, PS Square, Switch Y by label).
    West,
    /// Top face (Xbox Y, PS Triangle, Switch X by label).
    North,
    /// Left analog trigger (LT / L2 / ZL).
    LeftTrigger,
}

fn action_face(action: UiAction, swap_ab: bool, swap_xy: bool) -> Option<Face> {
    Some(match action {
        UiAction::Confirm => {
            if swap_ab {
                Face::East
            } else {
                Face::South
            }
        }
        UiAction::Cancel => {
            if swap_ab {
                Face::South
            } else {
                Face::East
            }
        }
        UiAction::WestFacePress => {
            if swap_xy {
                Face::North
            } else {
                Face::West
            }
        }
        UiAction::NorthFacePress => {
            if swap_xy {
                Face::West
            } else {
                Face::North
            }
        }
        UiAction::TriggerStructure => Face::LeftTrigger,
        _ => return None,
    })
}

/// Returns `(sheet_asset_path, sub_texture_name)` for the glyph that represents
/// `action` on the given controller style, after applying the player's `swap_ab`
/// / `swap_xy` preferences. The sub-texture name matches `SubTexture name="…"`
/// in the matching `_sheet_double.xml`.
fn static_glyph_sprite(
    style: GamepadStyle,
    action: UiAction,
    swap_ab: bool,
    swap_xy: bool,
) -> Option<(&'static str, &'static str)> {
    let face = action_face(action, swap_ab, swap_xy)?;
    Some(match (style, face) {
        (GamepadStyle::PlayStation, Face::South) => {
            (PLAYSTATION_SHEET, "playstation_button_color_cross")
        }
        (GamepadStyle::PlayStation, Face::East) => {
            (PLAYSTATION_SHEET, "playstation_button_color_circle")
        }
        (GamepadStyle::PlayStation, Face::West) => {
            (PLAYSTATION_SHEET, "playstation_button_color_square")
        }
        (GamepadStyle::PlayStation, Face::North) => {
            (PLAYSTATION_SHEET, "playstation_button_color_triangle")
        }
        (GamepadStyle::PlayStation, Face::LeftTrigger) => {
            (PLAYSTATION_SHEET, "playstation_trigger_l2")
        }
        (GamepadStyle::Nintendo, Face::South) => (SWITCH_SHEET, "switch_button_b"),
        (GamepadStyle::Nintendo, Face::East) => (SWITCH_SHEET, "switch_button_a"),
        (GamepadStyle::Nintendo, Face::West) => (SWITCH_SHEET, "switch_button_y"),
        (GamepadStyle::Nintendo, Face::North) => (SWITCH_SHEET, "switch_button_x"),
        (GamepadStyle::Nintendo, Face::LeftTrigger) => (SWITCH_SHEET, "switch_button_zl"),
        (GamepadStyle::Xbox | GamepadStyle::Generic, Face::South) => {
            (XBOX_SHEET, "xbox_button_color_a")
        }
        (GamepadStyle::Xbox | GamepadStyle::Generic, Face::East) => {
            (XBOX_SHEET, "xbox_button_color_b")
        }
        (GamepadStyle::Xbox | GamepadStyle::Generic, Face::West) => {
            (XBOX_SHEET, "xbox_button_color_x")
        }
        (GamepadStyle::Xbox | GamepadStyle::Generic, Face::North) => {
            (XBOX_SHEET, "xbox_button_color_y")
        }
        (GamepadStyle::Xbox | GamepadStyle::Generic, Face::LeftTrigger) => (XBOX_SHEET, "xbox_lt"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Each style's `SubTexture` index, embedded into the test binary. We can't
    /// open the live PNGs without a renderer, but the XMLs let us assert that
    /// every static glyph reference resolves to a real entry.
    const XBOX_XML: &str = include_str!(
        "../../assets/kenney_input-prompts/Xbox Series/xbox-series_sheet_double.xml"
    );
    const PLAYSTATION_XML: &str = include_str!(
        "../../assets/kenney_input-prompts/PlayStation Series/playstation-series_sheet_double.xml"
    );
    const SWITCH_XML: &str = include_str!(
        "../../assets/kenney_input-prompts/Nintendo Switch/nintendo-switch_sheet_double.xml"
    );

    fn names_in(xml: &str) -> HashMap<String, ()> {
        let mut out = HashMap::new();
        for line in xml.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("<SubTexture ") else {
                continue;
            };
            let needle = "name=\"";
            if let Some(i) = rest.find(needle) {
                let start = i + needle.len();
                if let Some(end) = rest[start..].find('"') {
                    out.insert(rest[start..start + end].to_string(), ());
                }
            }
        }
        out
    }

    fn xml_for(sheet: &'static str) -> &'static str {
        match sheet {
            s if s == XBOX_SHEET => XBOX_XML,
            s if s == PLAYSTATION_SHEET => PLAYSTATION_XML,
            s if s == SWITCH_SHEET => SWITCH_XML,
            other => panic!("no test XML for sheet {other}"),
        }
    }

    #[test]
    fn nintendo_swap_ab_maps_confirm_to_a_label() {
        // Real Nintendo controller users typically want Confirm on the A-labelled
        // (right) button, which is positional East.
        let sprite = static_glyph_sprite(GamepadStyle::Nintendo, UiAction::Confirm, true, false);
        assert_eq!(sprite, Some((SWITCH_SHEET, "switch_button_a")));
    }

    #[test]
    fn xbox_no_swap_maps_confirm_to_a() {
        let sprite = static_glyph_sprite(GamepadStyle::Xbox, UiAction::Confirm, false, false);
        assert_eq!(sprite, Some((XBOX_SHEET, "xbox_button_color_a")));
    }

    #[test]
    fn playstation_west_face_press_is_square() {
        let sprite =
            static_glyph_sprite(GamepadStyle::PlayStation, UiAction::WestFacePress, false, false);
        assert_eq!(sprite, Some((PLAYSTATION_SHEET, "playstation_button_color_square")));
    }

    #[test]
    fn unmappable_actions_return_none() {
        assert!(
            static_glyph_sprite(GamepadStyle::Xbox, UiAction::FocusNext, false, false).is_none()
        );
    }

    /// Catches typos: every static glyph reference must resolve in the matching
    /// `_sheet_double.xml`, otherwise the renderer would crop nothing at runtime.
    #[test]
    fn every_static_glyph_resolves_in_atlas() {
        use UiAction::*;
        let actions = [Confirm, Cancel, WestFacePress, NorthFacePress, TriggerStructure];
        let styles = [
            GamepadStyle::Xbox,
            GamepadStyle::PlayStation,
            GamepadStyle::Nintendo,
            GamepadStyle::Generic,
        ];
        for &style in &styles {
            for &action in &actions {
                for swap_ab in [false, true] {
                    for swap_xy in [false, true] {
                        let Some((sheet, name)) =
                            static_glyph_sprite(style, action, swap_ab, swap_xy)
                        else {
                            continue;
                        };
                        let names = names_in(xml_for(sheet));
                        assert!(
                            names.contains_key(name),
                            "{style:?} / {action:?} (swap_ab={swap_ab}, swap_xy={swap_xy}) \
                             references '{name}' missing from {sheet}",
                        );
                    }
                }
            }
        }
    }
}
