#[cfg(test)]
mod tests {
    use crate::core::hand::validate_selection;
    use crate::core::tile::{Suit, Tile};
    use crate::core::yaku::{YakuKind, detect_yaku_with_wind};
    use crate::scenes::guide::{example_structure_yaku, yaku_page};

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    /// Yaku intro page example hands must decompose and score at least one yaku
    /// so the bone tablets under each structure are non-empty.
    #[test]
    fn yaku_intro_examples_score_yaku_for_tablets() {
        let shousangen = vec![
            tile(Suit::Dragon, 1, 0),
            tile(Suit::Dragon, 1, 1),
            tile(Suit::Dragon, 1, 2),
            tile(Suit::Dragon, 2, 3),
            tile(Suit::Dragon, 2, 4),
            tile(Suit::Dragon, 2, 5),
            tile(Suit::Dragon, 3, 6),
            tile(Suit::Dragon, 3, 7),
            tile(Suit::Manzu, 2, 8),
            tile(Suit::Manzu, 3, 9),
            tile(Suit::Manzu, 4, 10),
            tile(Suit::Souzu, 5, 11),
            tile(Suit::Souzu, 5, 12),
            tile(Suit::Souzu, 5, 13),
        ];
        let with_kong = vec![
            tile(Suit::Manzu, 1, 40),
            tile(Suit::Manzu, 1, 41),
            tile(Suit::Manzu, 1, 42),
            tile(Suit::Manzu, 1, 43),
            tile(Suit::Souzu, 4, 44),
            tile(Suit::Souzu, 5, 45),
            tile(Suit::Souzu, 6, 46),
            tile(Suit::Pinzu, 7, 47),
            tile(Suit::Pinzu, 8, 48),
            tile(Suit::Pinzu, 9, 49),
            tile(Suit::Dragon, 1, 50),
            tile(Suit::Dragon, 1, 51),
            tile(Suit::Dragon, 1, 52),
            tile(Suit::Wind, 2, 53),
            tile(Suit::Wind, 2, 54),
        ];
        let chinitsu = vec![
            tile(Suit::Souzu, 2, 60),
            tile(Suit::Souzu, 3, 61),
            tile(Suit::Souzu, 4, 62),
            tile(Suit::Souzu, 4, 63),
            tile(Suit::Souzu, 5, 64),
            tile(Suit::Souzu, 6, 65),
            tile(Suit::Souzu, 6, 66),
            tile(Suit::Souzu, 7, 67),
            tile(Suit::Souzu, 8, 68),
            tile(Suit::Souzu, 5, 69),
            tile(Suit::Souzu, 5, 70),
            tile(Suit::Souzu, 5, 71),
        ];
        for (name, hand) in [
            ("shousangen", &shousangen),
            ("with kong", &with_kong),
            ("chinitsu", &chinitsu),
        ] {
            assert!(
                validate_selection(hand).is_some(),
                "{name}: example hand must decompose"
            );
            let yaku = example_structure_yaku(hand);
            assert!(
                !yaku.is_empty(),
                "{name}: expected at least one yaku tablet, got none"
            );
        }
        assert!(example_structure_yaku(&shousangen).contains(&YakuKind::Shousangen));
        assert!(example_structure_yaku(&with_kong).contains(&YakuKind::Yakuhai));
        assert!(example_structure_yaku(&chinitsu).contains(&YakuKind::Chinitsu));
        assert!(example_structure_yaku(&chinitsu).contains(&YakuKind::Tanyao));
        assert!(!example_structure_yaku(&chinitsu).contains(&YakuKind::Shousangen));
    }

    /// Every `yaku_page()` canonical hand must actually score as its named
    /// yaku in the real detector. The yaku journal draws these hands as
    /// teaching examples — if one drifts out of sync with the scorer we'd
    /// be teaching a lie, so the test locks the data to the implementation.
    ///
    /// Chicken Hand is skipped: the canonical example is a partial hand for
    /// display; full-run chicken is injected at structure cash-in when no
    /// unlocked yaku applies.
    #[test]
    fn every_yaku_page_hand_scores_as_its_yaku() {
        for &yk in YakuKind::all() {
            if yk == YakuKind::ChickenHand {
                continue;
            }
            let (_desc, groups) = yaku_page(yk);
            let tiles: Vec<_> = groups
                .iter()
                .flat_map(|g| g.tiles.iter().copied())
                .collect();
            let sets = validate_selection(&tiles).unwrap_or_else(|| {
                panic!(
                    "{:?}: yaku_page hand failed to decompose into sets: {:?}",
                    yk, tiles
                )
            });
            // Yakuhai needs a round wind hint for wind triplets to count.
            // The Yakuhai example in yaku_page uses a dragon triplet, which
            // counts regardless, so round_wind=None is still correct.
            let detected = detect_yaku_with_wind(&tiles, &sets, None, None, None);
            assert!(
                detected.contains(&yk),
                "{:?}: canonical hand did not score as {:?}. detected={:?}",
                yk,
                yk,
                detected,
            );
        }
    }
}
