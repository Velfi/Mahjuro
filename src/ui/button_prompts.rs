//! On-screen button prompts: face-button glyphs by manufacturer style and
//! small helpers for keyboard labels. SDL gamepad mappings use `South` /
//! `East` / `West` / `North`; this module turns those into what the player
//! expects to see on their controller.

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
}

/// Physical face positions in the SDL **semantic** layout (south =
/// bottom, etc.), not vendor paint. Test-only; runtime glyphs come from
/// [`crate::ui::glyph_source::GlyphResolver`].
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaceButton {
    South,
    East,
    West,
    North,
}

#[cfg(test)]
impl FaceButton {
    /// Short text inside prompts, without wrapping parentheses.
    pub fn glyph(self, style: GamepadStyle) -> &'static str {
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
    fn inspect_camera_extras(style: GamepadStyle) -> String {
        let t = style.analog_trigger_pair_label();
        format!("Right stick orbit  ·  {t} zoom")
    }

    /// Second shop HUD line while **item inspect** is active (gamepad vs mouse).
    pub fn shop_inspect_mode_hint(surface: PromptInputSurface, style: GamepadStyle) -> String {
        match surface {
            PromptInputSurface::Controller => Self::inspect_camera_extras(style),
            PromptInputSurface::MouseOrKeyboard => {
                "Drag to orbit · Mouse wheel: zoom (inspect)".to_string()
            }
        }
    }

    /// Wrapped face label, e.g. `(X)` or `(Square)`.
    #[cfg(test)]
    pub fn face(style: GamepadStyle, face: FaceButton) -> String {
        format!("({})", face.glyph(style))
    }

    /// `{face} {verb}` — e.g. `(X) Grab`; for other scenes building hint lines.
    #[cfg(test)]
    pub fn face_then(style: GamepadStyle, face: FaceButton, rest: &str) -> String {
        format!("{} {}", Self::face(style, face), rest)
    }

    #[cfg(test)]
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
                    Self::face(style, exit_face),
                    Self::face(style, select_face),
                    Self::face(style, FaceButton::West),
                    Self::face(style, FaceButton::North),
                )
            }
            PromptInputSurface::MouseOrKeyboard => {
                "Backspace exit  ·  Space / Enter select  ·  Hold Q sell  ·  E inspect".to_string()
            }
        }
    }

    #[cfg(test)]
    fn shop_core_verbs_only() -> &'static str {
        "Exit  ·  Select  ·  Sell  ·  Inspect"
    }

    /// Full bottom-bar copy for the shop (two lines when `inspect_active`) — unit-test helper only.
    #[cfg(test)]
    pub fn shop_floating_legend(
        surface: PromptInputSurface,
        style: GamepadStyle,
        swap_ab: bool,
        inspect_active: bool,
        text_style: ShopLegendTextStyle,
    ) -> String {
        let core = match text_style {
            ShopLegendTextStyle::InlineGlyphs => Self::shop_core_inline(surface, style, swap_ab),
            ShopLegendTextStyle::VerbsOnly => Self::shop_core_verbs_only().to_string(),
        };
        if inspect_active {
            let inspect_line = Self::shop_inspect_mode_hint(surface, style);
            format!("{core}\n{inspect_line}")
        } else {
            core
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(FaceButton::West.glyph(GamepadStyle::Xbox), "X");
    }

    #[test]
    fn glyph_playstation_west_is_square() {
        assert_eq!(FaceButton::West.glyph(GamepadStyle::PlayStation), "Square");
    }

    #[test]
    fn glyph_nintendo_south_is_b() {
        assert_eq!(FaceButton::South.glyph(GamepadStyle::Nintendo), "B");
    }

    #[test]
    fn shop_legend_inline_vs_verbs_only() {
        let inline = ButtonPrompt::shop_floating_legend(
            PromptInputSurface::Controller,
            GamepadStyle::Xbox,
            false,
            false,
            ShopLegendTextStyle::InlineGlyphs,
        );
        assert!(inline.contains("(A)"));
        assert!(inline.contains("(B)"));

        let verbs = ButtonPrompt::shop_floating_legend(
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
        let s = ButtonPrompt::face_then(GamepadStyle::Xbox, FaceButton::South, "Confirm");
        assert_eq!(s, "(A) Confirm");
    }
}
