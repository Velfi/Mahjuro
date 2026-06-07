//! Shared copy for the Guide Tanuki's Tips page (page 6).

pub const PAGE_TITLE: &str = "Tanuki's Tips";

pub const TIPS: &[&str] = &[
    "Your plays and discards are limited. Use them wisely. You can play or discard many tiles at once.",
    "Can't decide whether to play or discard? Do whatever moves the most tiles.",
    "Honors are worth more than numbered tiles, but the standard wall contains less of them. Unless...",
    "Relics last until the end of the run; They're vital to beating The House.",
    "Some talismans transform tiles or make them more valuable. Others give you small bonuses.",
    "Zodiacs level up your yaku, meaning the same yaku will score more points until the end of the run.",
    "Sometimes it's better to play what you have instead of seeking the lineup you wish for.",
];

pub fn quoted(tip: &str) -> String {
    format!("\u{201C}{tip}\u{201D}")
}
