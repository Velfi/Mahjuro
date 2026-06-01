//! Shared shop visit stock counts (relic / ribbon / pack slot rolls).

use rand::RngExt;

/// Max for-sale relic slots on the kiosk (must match scene stock generation).
pub const KIOSK_RELIC_SLOTS: usize = 3;
pub const MAX_RIBBONS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShopOfferCounts {
    pub n_relics: usize,
    pub n_zodiacs: usize,
    pub n_talismans: usize,
}

/// Roll how many relic / zodiac / talisman offerings appear this shop visit.
/// When `min_relic_slots` is 1 and the eligible pool is non-empty, at least one
/// relic slot is offered (early meta pools no longer whiff on zero relic rows).
pub fn roll_shop_offer_counts(
    extra_relics: usize,
    max_relic_slots: usize,
    min_relic_slots: usize,
    rng: &mut impl rand::Rng,
) -> ShopOfferCounts {
    let lo = min_relic_slots.min(max_relic_slots);
    let mut n_relics = rng.random_range(lo..=max_relic_slots) + extra_relics;
    let mut n_zodiacs = rng.random_range(1..=MAX_RIBBONS);
    let mut n_talismans = rng.random_range(1..=MAX_RIBBONS);
    if n_zodiacs + n_talismans > MAX_RIBBONS {
        while n_zodiacs + n_talismans > MAX_RIBBONS {
            if n_talismans >= n_zodiacs {
                n_talismans -= 1;
            } else {
                n_zodiacs -= 1;
            }
        }
    }
    while n_relics + n_zodiacs + n_talismans < 2 {
        let relics_room = n_relics < max_relic_slots;
        let ribbons_room = n_zodiacs + n_talismans < MAX_RIBBONS;
        let zodiacs_room = ribbons_room && n_zodiacs < MAX_RIBBONS;
        let talismans_room = ribbons_room && n_talismans < MAX_RIBBONS;
        let mut choices: Vec<u8> = Vec::with_capacity(3);
        if relics_room {
            choices.push(0);
        }
        if zodiacs_room {
            choices.push(1);
        }
        if talismans_room {
            choices.push(2);
        }
        if choices.is_empty() {
            break;
        }
        match choices[rng.random_range(0..choices.len())] {
            0 => n_relics += 1,
            1 => n_zodiacs += 1,
            _ => n_talismans += 1,
        }
    }
    ShopOfferCounts {
        n_relics,
        n_zodiacs,
        n_talismans,
    }
}
