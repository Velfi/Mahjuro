//! `{{token}}` placeholders inside `assets/data/memorial_talismans.json` descriptions.
//!
//! Expanded at display time via [`MemorialTalismanKind::description`] so tooltips show
//! values from the frozen defeat journal (blinds skipped, discards, favored yaku, …).
//! Unknown tokens stay literal so typos are visible during development.

use crate::core::memorial_talisman::{
    MemorialJournalSnapshot, MemorialTalismanKind, buff_saint_enhancement,
    discarded_extra_discards, skipper_clear_yen_bonus, transformer_target_suit,
};
use crate::core::tile::TileEnhancement;

fn enhancement_label(enh: TileEnhancement) -> &'static str {
    match enh {
        TileEnhancement::Pearl => "Pearl",
        TileEnhancement::Gilded => "Gilded",
        TileEnhancement::Polychrome => "Polychrome",
    }
}

fn suit_label(suit: crate::core::tile::Suit) -> &'static str {
    match suit {
        crate::core::tile::Suit::Manzu => "Manzu",
        crate::core::tile::Suit::Souzu => "Souzu",
        crate::core::tile::Suit::Pinzu => "Pinzu",
        other => {
            debug_assert!(false, "unexpected memorial transformer suit: {other:?}");
            "Souzu"
        }
    }
}

fn replace_token(
    kind: MemorialTalismanKind,
    key: &str,
    snapshot: Option<&MemorialJournalSnapshot>,
) -> Option<String> {
    match (kind, key.trim()) {
        (MemorialTalismanKind::Skipper, "clear_yen") => {
            Some(skipper_clear_yen_bonus(snapshot).to_string())
        }
        (MemorialTalismanKind::Discarded, "discards") => {
            Some(discarded_extra_discards(snapshot).to_string())
        }
        (MemorialTalismanKind::BuffSaint, "enhancement") => Some(
            snapshot
                .map(buff_saint_enhancement)
                .map(enhancement_label)
                .unwrap_or("your last run's favored enhancement")
                .to_string(),
        ),
        (MemorialTalismanKind::Transformer, "suit") => Some(
            snapshot
                .map(transformer_target_suit)
                .map(suit_label)
                .unwrap_or("your last run's favored suit")
                .to_string(),
        ),
        (MemorialTalismanKind::MeldMason, "yaku") => Some(
            snapshot
                .and_then(|s| s.dominant_yaku)
                .map(|y| y.name().to_string())
                .unwrap_or_else(|| "your favored yaku".to_string()),
        ),
        _ => None,
    }
}

/// Replace every `{{token}}` in `template` for this memorial kind.
pub fn expand_memorial_description_templates(
    kind: MemorialTalismanKind,
    template: &str,
    snapshot: Option<&MemorialJournalSnapshot>,
) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        let Some(end) = rest.find("}}") else {
            out.push_str("{{");
            out.push_str(rest);
            return out;
        };
        let key = &rest[..end];
        if let Some(v) = replace_token(kind, key, snapshot) {
            out.push_str(&v);
        } else {
            out.push_str("{{");
            out.push_str(key);
            out.push_str("}}");
        }
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::memorial_talisman::RunDefeatJournal;
    use crate::core::rules::ChamberKind;
    use mahjuro_types::GameOverReason;

    fn snapshot(skipped: u32) -> MemorialJournalSnapshot {
        MemorialJournalSnapshot {
            journal: RunDefeatJournal {
                chambers_skipped: skipped,
                ..RunDefeatJournal::default()
            },
            loss_reason: GameOverReason::OutOfPlays,
            final_wing: 1,
            final_chamber: ChamberKind::Small,
            final_yen: 0,
            tiles_played: 0,
            tiles_discarded: 0,
            consumables_unused: 0,
            dominant_yaku: None,
            run_number: Some(3),
        }
    }

    #[test]
    fn skipper_clear_yen_token() {
        let s = "Use: +¥{{clear_yen}} yen when you clear this blind.";
        assert_eq!(
            expand_memorial_description_templates(MemorialTalismanKind::Skipper, s, None),
            "Use: +¥4 yen when you clear this blind."
        );
        assert_eq!(
            expand_memorial_description_templates(
                MemorialTalismanKind::Skipper,
                s,
                Some(&snapshot(3)),
            ),
            "Use: +¥7 yen when you clear this blind."
        );
    }

    #[test]
    fn unknown_token_preserved() {
        let s = "x {{not_a_real_token}} y";
        assert_eq!(
            expand_memorial_description_templates(MemorialTalismanKind::Skipper, s, None),
            s
        );
    }
}
