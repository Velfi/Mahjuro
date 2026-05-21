//! On-screen button prompts: face-button glyphs by manufacturer style and
//! small helpers for keyboard labels. SDL gamepad mappings use `South` /
//! `East` / `West` / `North`; this module turns those into what the player
//! expects to see on their controller.

use crate::ui::input::UiAction;

/// Best-effort controller family for prompt text. Derived from USB vendor ID
/// (when present) and lowercase name heuristics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum GamepadStyle {
    /// Microsoft Xbox / XInput-style (A/B/X/Y at cardinal positions).
    Xbox,
    /// Sony DualShock / DualSense (shapes).
    PlayStation,
    /// Nintendo Switch / Pro Controller / Joy-Con (B/A/Y/X at cardinals).
    Nintendo,
    /// Nintendo Switch 2 family — same face layout as Switch 1; separate Kenney atlas.
    NintendoSwitch2,
    /// Valve Steam Deck built-in controls.
    SteamDeck,
    /// Valve Steam Controller.
    SteamController,
    /// Unknown or third-party — use the same **positions** as Xbox letters.
    #[default]
    Generic,
}

impl GamepadStyle {
    /// Classify from OS-reported USB vendor and human-readable name.
    pub fn infer(vendor_id: Option<u16>, name: &str) -> Self {
        let n = name.to_ascii_lowercase();

        if n.contains("steam deck") {
            return Self::SteamDeck;
        }
        if n.contains("steam controller") {
            return Self::SteamController;
        }

        if let Some(v) = vendor_id {
            match v {
                0x045E => return Self::Xbox,        // Microsoft
                0x054C => return Self::PlayStation, // Sony
                0x057E => {
                    if n.contains("switch 2") || n.contains("switch2") {
                        return Self::NintendoSwitch2;
                    }
                    return Self::Nintendo;
                }
                0x28DE => {
                    // Valve — prefer name hints (Steam Virtual Gamepad has no "Deck" in the name).
                    if n.contains("deck") {
                        return Self::SteamDeck;
                    }
                    if n.contains("controller") && n.contains("steam") {
                        return Self::SteamController;
                    }
                    return Self::Generic;
                }
                _ => {}
            }
        }

        if n.contains("xbox")
            || n.contains("microsoft")
            || n.contains("xinput")
            || n.contains("steam virtual gamepad")
        {
            return Self::Xbox;
        }
        if n.contains("dualsense")
            || n.contains("dualshock")
            || n.contains("sony")
            || n.contains("ps5")
            || n.contains("ps4")
            || n.contains("playstation")
        {
            return Self::PlayStation;
        }
        if n.contains("switch 2") || n.contains("switch2") {
            return Self::NintendoSwitch2;
        }
        if n.contains("nintendo")
            || n.contains("switch")
            || n.contains("pro controller")
            || n.contains("joy-con")
            || n.contains("joy con")
        {
            return Self::Nintendo;
        }
        Self::Generic
    }

    /// Analog trigger names in UI copy (shoulder digital bumpers unchanged).
    pub fn analog_trigger_pair_label(self) -> &'static str {
        match self {
            Self::PlayStation | Self::SteamDeck => "L2/R2",
            Self::Xbox
            | Self::Nintendo
            | Self::NintendoSwitch2
            | Self::SteamController
            | Self::Generic => "LT/RT",
        }
    }

    /// Shoulder button names (tab cycling, HUD stepping, etc.).
    pub fn shoulder_pair_label(self) -> &'static str {
        match self {
            Self::PlayStation | Self::SteamDeck => "L1/R1",
            Self::Nintendo | Self::NintendoSwitch2 => "L/R",
            Self::Xbox | Self::SteamController | Self::Generic => "LB/RB",
        }
    }
}

/// Whether on-screen prompts should use controller glyphs or keyboard text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptInputSurface {
    Controller,
    MouseOrKeyboard,
}

/// Core shop HUD actions in **Exit → Select → Sell → Inspect** order (matches [`crate::ui::kenney_prompt_paths::shop_keyboard_prompt_icons`]).
pub const SHOP_LEGEND_VERB_LABELS: [&str; 4] = ["Exit", "Select", "Sell", "Inspect"];

/// For [`ButtonPrompt::shop_floating_legend`] unit tests only.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShopLegendTextStyle {
    /// Text carries face/key identity, e.g. `(A) Exit · …` or `Backspace exit · …`.
    #[default]
    InlineGlyphs,
    /// Action verbs only — pair with on-screen prompt artwork (same line for controller and keyboard).
    VerbsOnly,
}

/// Build strings like `(A)` or `(Cross)` for floating hints.
#[derive(Clone, Copy, Debug, Default)]
pub struct ButtonPrompt;

impl ButtonPrompt {
    fn action_face(action: UiAction, swap_ab: bool, swap_xy: bool) -> Option<FaceButton> {
        Some(match action {
            UiAction::Confirm => {
                if swap_ab {
                    FaceButton::East
                } else {
                    FaceButton::South
                }
            }
            UiAction::Cancel => {
                if swap_ab {
                    FaceButton::South
                } else {
                    FaceButton::East
                }
            }
            UiAction::WestFacePress => {
                if swap_xy {
                    FaceButton::North
                } else {
                    FaceButton::West
                }
            }
            UiAction::NorthFacePress => {
                if swap_xy {
                    FaceButton::West
                } else {
                    FaceButton::North
                }
            }
            _ => return None,
        })
    }

    /// Human-readable face-button label for the action under the active swap options.
    pub fn controller_action_label(
        style: GamepadStyle,
        action: UiAction,
        swap_ab: bool,
        swap_xy: bool,
    ) -> Option<&'static str> {
        let face = Self::action_face(action, swap_ab, swap_xy)?;
        Some(face.label(style))
    }

    fn inspect_camera_extras(style: GamepadStyle) -> String {
        let t = style.analog_trigger_pair_label();
        format!("Right stick: orbit item  ·  Left stick: cycle items  ·  {t} zoom")
    }

    /// Second shop HUD line while **item inspect** is active (gamepad vs mouse).
    pub fn shop_inspect_mode_hint(surface: PromptInputSurface, style: GamepadStyle) -> String {
        match surface {
            PromptInputSurface::Controller => Self::inspect_camera_extras(style),
            PromptInputSurface::MouseOrKeyboard => {
                "Arrows or drag: orbit item · WASD: cycle items · Shift+W/↑ zoom in · Shift+S/↓ zoom out · Mouse wheel: zoom (inspect)"
                    .to_string()
            }
        }
    }
}

/// Physical face positions in the SDL semantic layout (south = bottom, etc.),
/// not vendor paint labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum FaceButton {
    South,
    East,
    West,
    North,
}

impl FaceButton {
    fn label(self, style: GamepadStyle) -> &'static str {
        use FaceButton::{East, North, South, West};
        use GamepadStyle::{
            Generic, Nintendo, NintendoSwitch2, PlayStation, SteamController, SteamDeck, Xbox,
        };
        match (style, self) {
            (Xbox | Generic | SteamDeck | SteamController, South) => "A",
            (Xbox | Generic | SteamDeck | SteamController, East) => "B",
            (Xbox | Generic | SteamDeck | SteamController, West) => "X",
            (Xbox | Generic | SteamDeck | SteamController, North) => "Y",

            (PlayStation, South) => "Cross",
            (PlayStation, East) => "Circle",
            (PlayStation, West) => "Square",
            (PlayStation, North) => "Triangle",

            (Nintendo | NintendoSwitch2, South) => "B",
            (Nintendo | NintendoSwitch2, East) => "A",
            (Nintendo | NintendoSwitch2, West) => "Y",
            (Nintendo | NintendoSwitch2, North) => "X",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::input::UiAction;

    fn glyph(face: FaceButton, style: GamepadStyle) -> &'static str {
        face.label(style)
    }

    fn face(style: GamepadStyle, face: FaceButton) -> String {
        format!("({})", glyph(face, style))
    }

    fn face_then(style: GamepadStyle, button: FaceButton, rest: &str) -> String {
        format!("{} {}", face(style, button), rest)
    }

    fn shop_core_inline(surface: PromptInputSurface, style: GamepadStyle, swap_ab: bool) -> String {
        match surface {
            PromptInputSurface::Controller => {
                let (exit_face, select_face) = if swap_ab {
                    (FaceButton::South, FaceButton::East)
                } else {
                    (FaceButton::East, FaceButton::South)
                };
                format!(
                    "{} Exit  ·  {} Select  ·  {} Sell  ·  {} Inspect",
                    face(style, exit_face),
                    face(style, select_face),
                    face(style, FaceButton::West),
                    face(style, FaceButton::North),
                )
            }
            PromptInputSurface::MouseOrKeyboard => {
                "Backspace exit  ·  Space / Enter select  ·  Hold Q sell  ·  E inspect".to_string()
            }
        }
    }

    fn shop_floating_legend(
        surface: PromptInputSurface,
        style: GamepadStyle,
        swap_ab: bool,
        inspect_active: bool,
        text_style: ShopLegendTextStyle,
    ) -> String {
        let core = match text_style {
            ShopLegendTextStyle::InlineGlyphs => shop_core_inline(surface, style, swap_ab),
            ShopLegendTextStyle::VerbsOnly => "Exit  ·  Select  ·  Sell  ·  Inspect".to_string(),
        };
        if inspect_active {
            let inspect_line = ButtonPrompt::shop_inspect_mode_hint(surface, style);
            format!("{core}\n{inspect_line}")
        } else {
            core
        }
    }

    #[test]
    fn infer_vendor_microsoft() {
        assert_eq!(GamepadStyle::infer(Some(0x045E), "Foo"), GamepadStyle::Xbox);
    }

    #[test]
    fn infer_name_dualsense() {
        assert_eq!(
            GamepadStyle::infer(None, "Sony DualSense Wireless Controller"),
            GamepadStyle::PlayStation
        );
    }

    #[test]
    fn infer_vendor_valve_virtual_gamepad_is_generic() {
        assert_eq!(
            GamepadStyle::infer(Some(0x28DE), "Steam Virtual Gamepad"),
            GamepadStyle::Generic
        );
    }

    #[test]
    fn infer_steam_deck_by_name() {
        assert_eq!(
            GamepadStyle::infer(None, "Steam Deck"),
            GamepadStyle::SteamDeck
        );
    }

    #[test]
    fn infer_steam_controller_by_name() {
        assert_eq!(
            GamepadStyle::infer(None, "Valve Steam Controller"),
            GamepadStyle::SteamController
        );
    }

    #[test]
    fn infer_switch2_by_name() {
        assert_eq!(
            GamepadStyle::infer(None, "Nintendo Switch 2 Pro Controller"),
            GamepadStyle::NintendoSwitch2
        );
    }

    #[test]
    fn glyph_xbox_west_is_x() {
        assert_eq!(glyph(FaceButton::West, GamepadStyle::Xbox), "X");
    }

    #[test]
    fn glyph_playstation_west_is_square() {
        assert_eq!(glyph(FaceButton::West, GamepadStyle::PlayStation), "Square");
    }

    #[test]
    fn glyph_nintendo_south_is_b() {
        assert_eq!(glyph(FaceButton::South, GamepadStyle::Nintendo), "B");
    }

    #[test]
    fn controller_action_label_respects_swap_settings() {
        assert_eq!(
            ButtonPrompt::controller_action_label(
                GamepadStyle::Xbox,
                UiAction::Confirm,
                false,
                false
            ),
            Some("A")
        );
        assert_eq!(
            ButtonPrompt::controller_action_label(
                GamepadStyle::Xbox,
                UiAction::Confirm,
                true,
                false
            ),
            Some("B")
        );
        assert_eq!(
            ButtonPrompt::controller_action_label(
                GamepadStyle::PlayStation,
                UiAction::NorthFacePress,
                false,
                false
            ),
            Some("Triangle")
        );
    }

    #[test]
    fn shop_legend_inline_vs_verbs_only() {
        let inline = shop_floating_legend(
            PromptInputSurface::Controller,
            GamepadStyle::Xbox,
            false,
            false,
            ShopLegendTextStyle::InlineGlyphs,
        );
        assert!(inline.contains("(A)"));
        assert!(inline.contains("(B)"));

        let verbs = shop_floating_legend(
            PromptInputSurface::Controller,
            GamepadStyle::Xbox,
            false,
            false,
            ShopLegendTextStyle::VerbsOnly,
        );
        assert!(!verbs.contains('('));
        assert!(verbs.starts_with("Exit"));
    }

    #[test]
    fn face_then_joins_glyph_and_rest() {
        let s = face_then(GamepadStyle::Xbox, FaceButton::South, "Confirm");
        assert_eq!(s, "(A) Confirm");
    }
}
