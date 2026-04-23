use super::*;

impl RunState {
    /// Roll fresh tags for the Small and Big blinds of the current ante.
    pub fn roll_ante_tags(&mut self) {
        use crate::core::tag::roll_tag;

        let small = roll_tag(self.ante, None);
        let big = roll_tag(self.ante, Some(small));
        self.small_blind_tag = Some(small);
        self.big_blind_tag = Some(big);
    }

    /// Return the tag assigned to the given blind, if any.
    pub fn tag_for_blind(&self, blind: BlindKind) -> Option<crate::core::tag::TagKind> {
        match blind {
            BlindKind::Small => self.small_blind_tag,
            BlindKind::Big => self.big_blind_tag,
            BlindKind::Boss => None,
        }
    }

    /// Apply a skip-reward tag's effect. Returns a short description for UI feedback.
    pub fn apply_tag(&mut self, tag: crate::core::tag::TagKind) -> &'static str {
        use crate::core::tag::TagKind;

        match tag {
            TagKind::GoldIngot => {
                self.gold = self.gold.saturating_add(8);
                "+8 gold"
            }
            TagKind::TreasureChest => {
                self.gold = self.gold.saturating_add(20);
                "+20 gold"
            }
            TagKind::FreeReroll => {
                self.tag_free_reroll = true;
                "Free reroll"
            }
            TagKind::PatronGift => {
                self.tag_patron_gift = true;
                "Free relic"
            }
            TagKind::RichStock => {
                self.tag_rich_stock = true;
                "+2 shop relics"
            }
            TagKind::ZodiacBlessing => {
                use crate::core::zodiac::ZodiacKind;
                use rand::seq::IndexedRandom;

                let all = ZodiacKind::all();
                let mut rng = rand::rng();
                if let Some(&z) = all.choose(&mut rng) {
                    let yaku = z.yaku();
                    let new_level = self.yaku_levels.level_up(yaku);
                    self.pending_zodiac_celebration = Some((z, yaku, new_level));
                    return "Zodiac activated";
                }
                "No zodiac"
            }
            TagKind::RelicOffering => {
                use crate::core::relic::all_relic_defs;
                use rand::seq::SliceRandom;

                let defs = all_relic_defs();
                let mut pool: Vec<_> = defs
                    .iter()
                    .filter(|d| self.available_relics.contains(&d.id) && !self.relics.owns(d.id))
                    .collect();
                if pool.is_empty() || self.relics.is_full() {
                    self.gold = self.gold.saturating_add(6);
                    return "+6 gold (full)";
                }
                let mut rng = rand::rng();
                pool.shuffle(&mut rng);
                self.relics.active.push(pool[0].id);
                "Relic gained"
            }
            TagKind::BonusPlay => {
                self.tag_bonus_plays += 1;
                "+1 play"
            }
            TagKind::BonusDiscard => {
                self.tag_bonus_discards += 1;
                "+1 discard"
            }
            TagKind::WideHand => {
                self.tag_bonus_hand_size += 2;
                "+2 hand size"
            }
        }
    }

    /// Clear transient skip-tag bonuses that only apply to the very next blind.
    pub(super) fn clear_next_blind_tag_modifiers(&mut self) {
        self.tag_bonus_plays = 0;
        self.tag_bonus_discards = 0;
        self.tag_bonus_hand_size = 0;
    }
}
