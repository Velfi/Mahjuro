//! Keyboard / mouse glyph references for in-game prompt rows.
//!
//! Controller glyphs are resolved by [`crate::ui::glyph_source::GlyphResolver`]
//! (Kenney Input Prompts atlases keyed by [`crate::ui::button_prompts::GamepadStyle`]).
//! This module owns the keyboard / mouse glyphs that neither path provides.

use crate::render::draw_cmd::PromptIconSource;

const KEYBOARD_SHEET: &str =
    "kenney_input-prompts/Keyboard & Mouse/keyboard-&-mouse_sheet_double.png";

const fn key(name: &'static str) -> PromptIconSource {
    PromptIconSource::AtlasSprite {
        sheet: KEYBOARD_SHEET,
        name,
    }
}

/// Four keyboard icons matching the shop legend (Exit, Select, Sell, Inspect).
pub fn shop_keyboard_prompt_icons() -> [PromptIconSource; 4] {
    [
        key("keyboard_backspace"),
        key("keyboard_space"),
        key("keyboard_q"),
        key("keyboard_e"),
    ]
}

/// Three keyboard icons under the discard bowl, play mirror, and cash-in tablet.
pub fn gameplay_keyboard_prompt_icons() -> [PromptIconSource; 3] {
    [key("keyboard_q"), key("keyboard_e"), key("keyboard_t")]
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYBOARD_XML: &str = include_str!(
        "../../assets/kenney_input-prompts/Keyboard & Mouse/keyboard-&-mouse_sheet_double.xml"
    );

    fn expect_resolves(icon: &PromptIconSource) {
        let PromptIconSource::AtlasSprite { sheet, name } = icon else {
            panic!("expected AtlasSprite, got {icon:?}");
        };
        assert_eq!(*sheet, KEYBOARD_SHEET);
        let needle = format!(r#"name="{name}""#);
        assert!(
            KEYBOARD_XML.contains(&needle),
            "{name:?} missing from keyboard atlas",
        );
    }

    #[test]
    fn shop_keyboard_icons_resolve_in_atlas() {
        for icon in shop_keyboard_prompt_icons() {
            expect_resolves(&icon);
        }
    }

    #[test]
    fn gameplay_keyboard_icons_resolve_in_atlas() {
        for icon in gameplay_keyboard_prompt_icons() {
            expect_resolves(&icon);
        }
    }
}
