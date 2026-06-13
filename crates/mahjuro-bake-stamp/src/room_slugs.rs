//! Offline room bake output slugs (must match `RoomGiRoom::slug` in `mahjuro-render`).

pub const LIGHTMAP_ALL: &[&str] = &[
    "shop",
    "hallway",
    "archive",
    "main_menu",
    "stairway",
    "gameplay",
    "shadow_test_room",
];

pub const SHADOW_ALL: &[&str] = &[
    "shop",
    "hallway",
    "archive",
    "main_menu",
    "stairway",
    "gameplay",
];

pub const ALL: &[&str] = LIGHTMAP_ALL;
