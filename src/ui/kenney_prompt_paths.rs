//! Relative paths under `assets/` for Kenney Input Prompts SVGs.
//!
//! Controller glyphs come from Steam Input (`ISteamInput::GetGlyphSVGForActionOrigin`);
//! see [`crate::steam::SteamInputBridge::glyph_path_for`]. This module only owns
//! the keyboard / mouse icons that Steam does not provide.

/// Four keyboard icons matching the shop legend (Exit, Select, Sell, Inspect).
pub fn shop_keyboard_prompt_icon_paths() -> [&'static str; 4] {
    [
        "kenney_input-prompts/Keyboard & Mouse/Vector/keyboard_backspace.svg",
        "kenney_input-prompts/Keyboard & Mouse/Vector/keyboard_space.svg",
        "kenney_input-prompts/Keyboard & Mouse/Vector/keyboard_q.svg",
        "kenney_input-prompts/Keyboard & Mouse/Vector/keyboard_e.svg",
    ]
}

/// Three keyboard icons under the discard bowl, play mirror, and cash-in tablet.
pub fn gameplay_keyboard_prompt_icon_paths() -> [&'static str; 3] {
    [
        "kenney_input-prompts/Keyboard & Mouse/Vector/keyboard_q.svg",
        "kenney_input-prompts/Keyboard & Mouse/Vector/keyboard_e.svg",
        "kenney_input-prompts/Keyboard & Mouse/Vector/keyboard_t.svg",
    ]
}
