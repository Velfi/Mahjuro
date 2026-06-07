//! Shared copy for the Guide flowers page.

pub const PAGE_TITLE: &str = "Flowers";

pub const PAGE_SUBTITLE: &str =
    "Flowers are wildcards — they can replace one tile in a three-tile meld. Flowers are useful for forming yaku, but this comes at a cost: they don't contribute to a meld's score.";

pub const SECTION_ALLOWED: &str = "ALLOWED";
pub const ALLOWED_LINES: &[&str] = &[
    "Complete a triplet (7 · 7 · Flower).",
    "Complete a sequence in one suit (4 · Flower · 6).",
    "Two Flowers may form a pair.",
    "Three Flowers may form a triplet.",
];

pub const SECTION_NOT_ALLOWED: &str = "NOT ALLOWED";
pub const NOT_ALLOWED_LINES: &[&str] = &[
    "A Flower paired with a regular tile.",
    "Two Flowers in a three-tile meld.",
];
