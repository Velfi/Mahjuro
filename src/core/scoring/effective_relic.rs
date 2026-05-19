//! Resolve Mirror Tile / Shadow Hand once per score so scoring logic uses a single
//! `has` / `count` API instead of repeating mirror/shadow rules beside every relic.

use crate::core::relic::{RelicId, RelicState, ScoreContext};

#[derive(Clone, Copy, Debug)]
pub(crate) struct EffectiveRelics {
    mirrored: Option<RelicId>,
    shadowed: Option<RelicId>,
}

impl EffectiveRelics {
    /// Mirror Tile / Shadow Hand copy targets from inventory order alone (no score counters).
    pub(crate) fn from_roster(roster: &RelicState) -> Self {
        let mirrored = if roster.has(RelicId::MirrorTile) {
            roster.relic_after(RelicId::MirrorTile)
        } else {
            None
        };
        let shadowed = if roster.has(RelicId::ShadowHand) {
            roster
                .active
                .first()
                .filter(|&&id| id != RelicId::ShadowHand)
                .copied()
        } else {
            None
        };
        Self { mirrored, shadowed }
    }

    pub(crate) fn from_context(ctx: &ScoreContext<'_>) -> Self {
        Self::from_roster(ctx.relic.roster)
    }

    #[inline]
    pub(crate) fn has(self, relics: &RelicState, id: RelicId) -> bool {
        relics.has(id) || self.mirrored == Some(id) || self.shadowed == Some(id)
    }

    #[inline]
    pub(crate) fn count(self, relics: &RelicState, id: RelicId) -> u32 {
        let owned = relics.has(id) as u32;
        let mirror = (self.mirrored == Some(id)) as u32;
        let shadow = (self.shadowed == Some(id)) as u32;
        owned + mirror + shadow
    }
}
