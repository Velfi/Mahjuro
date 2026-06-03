//! Canonical scene-key strings for tonemap, shadows, room env tuning, and CLI.

pub const MAIN_MENU: &str = "main_menu";
pub const SHOP: &str = "shop";
pub const HALLWAY: &str = "hallway";
pub const GAMEPLAY: &str = "gameplay";
pub const ARCHIVE: &str = "archive";
pub const OPTIONS: &str = "options";
pub const STAIRWAY: &str = "stairway";
pub const VICTORY: &str = "victory";
pub const DEFEAT: &str = "defeat";
pub const SHADOW_AO_LAB: &str = "shadow_ao_lab";

/// Primary navigable scenes — the names used in docs, CLI, and persistence keys.
pub const PRIMARY: &[&str] = &[
    MAIN_MENU, SHOP, HALLWAY, GAMEPLAY, ARCHIVE, OPTIONS, STAIRWAY, VICTORY, DEFEAT,
];

/// Legacy `active_scene_key` / tuning suffixes still accepted when parsing input.
pub fn legacy_aliases(key: &str) -> &'static [&'static str] {
    match key {
        MAIN_MENU => &["main_menu_exterior"],
        HALLWAY => &["pick_chamber", "pick_blind"],
        ARCHIVE => &["collection"],
        STAIRWAY => &["staircase"],
        VICTORY => &["game_over_victory"],
        DEFEAT => &["game_over", "game_over_defeat"],
        _ => &[],
    }
}

/// Resolve a user-supplied slug to a canonical scene key when it is a known alias.
pub fn normalize_scene_key(slug: &str) -> &str {
    let s = slug.trim();
    let eq = |name: &str| s.eq_ignore_ascii_case(name);
    if eq(MAIN_MENU) || eq("main_menu_exterior") || eq("start_screen") || eq("main-menu") {
        MAIN_MENU
    } else if eq(SHOP) {
        SHOP
    } else if eq(HALLWAY) || eq("pick_chamber") || eq("pick_blind") {
        HALLWAY
    } else if eq(GAMEPLAY) {
        GAMEPLAY
    } else if eq(ARCHIVE) || eq("collection") {
        ARCHIVE
    } else if eq(OPTIONS) {
        OPTIONS
    } else if eq(STAIRWAY) || eq("staircase") {
        STAIRWAY
    } else if eq(VICTORY) || eq("game_over_victory") {
        VICTORY
    } else if eq(DEFEAT) || eq("game_over") || eq("game_over_defeat") {
        DEFEAT
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_maps_legacy_scene_slugs() {
        assert_eq!(normalize_scene_key("pick_chamber"), HALLWAY);
        assert_eq!(normalize_scene_key("main_menu_exterior"), MAIN_MENU);
        assert_eq!(normalize_scene_key("collection"), ARCHIVE);
        assert_eq!(normalize_scene_key("staircase"), STAIRWAY);
        assert_eq!(normalize_scene_key("game_over_defeat"), DEFEAT);
    }
}
