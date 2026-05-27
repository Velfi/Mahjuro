//! Controller family detection for Kenney Input Prompts atlases.

/// Best-effort controller family for prompt glyphs. Derived from USB vendor ID
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
}
