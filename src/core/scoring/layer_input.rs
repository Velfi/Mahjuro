//! Shared inputs for scoring pipeline layers ([`super::pre_yaku_layer`], etc.).

use crate::core::hand::DetectedMeld;
use crate::core::relic::ScoreContext;
use crate::core::tile::Tile;

use super::ScoreStep;
use super::effective_relic::EffectiveRelics;

/// Hand tiles, melds, and relic mirror state for one scoring pass.
pub(crate) struct ScoringLayerInput<'a> {
    pub ctx: &'a ScoreContext<'a>,
    pub tiles: &'a [Tile],
    pub sets: &'a [DetectedMeld],
    pub eff: EffectiveRelics,
}

/// Running chip/mult totals and step log mutated by each layer.
pub(crate) struct ScoringLayerOut<'a> {
    pub chips: &'a mut i32,
    pub mult: &'a mut f64,
    pub steps: &'a mut Vec<ScoreStep>,
}

/// Options for [`super::pre_yaku_layer::apply_pre_yaku_scoring`].
pub(crate) struct PreYakuLayerOpts<'a> {
    pub pair_double: bool,
    pub has_triplet_boost: bool,
    pub flower_gold: &'a mut i32,
}

/// Options for [`super::dora_yaku_layer::apply_dora_yaku_and_structure`].
pub(crate) struct DoraYakuLayerOpts<'a> {
    pub censor_repeats: bool,
    pub original_tiles: Option<&'a [Tile]>,
}

/// Options for [`super::relic_mult_layer::apply_post_yaku_relic_modifiers`].
pub(crate) struct PostYakuRelicLayerOpts {
    pub honor_triple: bool,
    pub no_seq_bonus: bool,
    pub has_triplet_boost: bool,
}
