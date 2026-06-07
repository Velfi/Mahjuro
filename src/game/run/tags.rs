use super::*;

impl RunState {
    /// Roll fresh tags for the Small and Big blinds of the current ante.
    pub fn roll_ante_tags(&mut self) {
        use crate::core::tag::roll_tag;

        let small = roll_tag(self.wing, None);
        let big = roll_tag(self.wing, Some(small));
        self.small_chamber_tag = Some(small);
        self.big_chamber_tag = Some(big);
    }

    /// Return the tag assigned to the given blind, if any.
    pub fn tag_for_chamber(&self, blind: ChamberKind) -> Option<crate::core::tag::TagKind> {
        match blind {
            ChamberKind::Small => self.small_chamber_tag,
            ChamberKind::Big => self.big_chamber_tag,
            ChamberKind::Ordeal => None,
        }
    }

    /// Apply a temptation's effect. Returns a short description for UI feedback.
    pub fn apply_tag(
        &mut self,
        tag: crate::core::tag::TagKind,
        bus: Option<&mut EventBus>,
    ) -> &'static str {
        use crate::core::tag::TagKind;

        self.defeat_journal.tags_taken = self.defeat_journal.tags_taken.saturating_add(1);

        match tag {
            TagKind::GoldIngot => {
                self.apply_yen_reward(8, bus);
                "+8 yen"
            }
            TagKind::TreasureChest => {
                self.apply_yen_reward(20, bus);
                "+20 yen"
            }
            TagKind::FreeRestock => {
                self.tag_free_restock += 1;
                "Free restock"
            }
            TagKind::PatronGift => {
                self.tag_patron_gift += 1;
                "Free relic"
            }
            TagKind::RichStock => {
                self.tag_rich_stock += 1;
                "+2 shop relics"
            }
            TagKind::ZodiacBlessing => {
                use rand::RngExt;

                let mut pool = self.zodiac_spawn_pool();
                let mut rng = rand::rng();
                let mut granted = 0u32;
                for _ in 0..2 {
                    if pool.is_empty() {
                        break;
                    }
                    let idx = rng.random_range(0..pool.len());
                    let z = pool.remove(idx);
                    let yaku = z.yaku();
                    let new_level = self.yaku_levels.level_up_for_zodiac(z);
                    self.pending_zodiac_celebrations.push((z, yaku, new_level));
                    granted += 1;
                }
                match granted {
                    0 => "No zodiac",
                    1 => "Zodiac activated",
                    _ => "2 zodiacs activated",
                }
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

    /// Clear transient temptation bonuses that only apply to the very next blind.
    pub(super) fn clear_next_chamber_tag_modifiers(&mut self) {
        self.tag_bonus_plays = 0;
        self.tag_bonus_discards = 0;
        self.tag_bonus_hand_size = 0;
    }
}
