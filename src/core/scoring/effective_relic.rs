//! Resolve Mirror Tile / Shadow Hand once per score so scoring logic uses a single
//! `has` / `count` API instead of repeating mirror/shadow rules beside every relic.

use crate::core::relic::{RelicId, RelicState, ScoreContext};

#[derive(Clone, Copy, Debug)]
pub(crate) struct EffectiveRelics {
    mirrored: Option<RelicId>,
    shadowed: Option<RelicId>,
}

impl EffectiveRelics {
    pub(crate) fn from_context(ctx: &ScoreContext<'_>) -> Self {
        let mirrored = if ctx.relic.roster.has(RelicId::MirrorTile) {
            ctx.relic.roster.relic_after(RelicId::MirrorTile)
        } else {
            None
        };
        let shadowed = if ctx.relic.roster.has(RelicId::ShadowHand) {
            ctx.relic
                .roster
                .active
                .first()
                .filter(|&&id| id != RelicId::ShadowHand)
                .copied()
        } else {
            None
        };
        Self { mirrored, shadowed }
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
