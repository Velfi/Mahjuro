//! `{{token}}` placeholders inside `assets/data/relics.json` descriptions.
//!
//! Expanded at display time via [`expand_relic_description_templates`] so shop,
//! gameplay, and collection tooltips show live counters. When [`RelicDescContext::live`]
//! is false (archive catalog), live tokens fall back to design-time defaults so
//! run state is not leaked. Unknown tokens stay literal so typos are visible.

use std::collections::BTreeMap;

use crate::core::relic::{
    RelicId, RelicState, CHRYSALIS_HATCH_EXCESS_THRESHOLD, KINDLING_MULT_CAP,
    MELTING_ICE_START_CHIPS, SNOWBALL_STACK_CAP, TAOTIE_CHIPS_PER_DEVOURED,
    golden_engine_mult_bonus, kindling_mult_bonus, monarch_butterfly_bonus_chips,
    monarch_butterfly_tier, monarch_next_tier_excess_floor, relic_sell_price_live,
    snowball_score_chips,
};

// --- Fortune's Favor tuning (keep in sync with `round_flow` / `scoring_flow`) ---

pub const PAPER_LANTERN_DESTROY_DENOM: u32 = 5;
pub const PAPER_LANTERN_DESTROY_DENOM_WITH_FF: u32 = 10;

pub const SILVER_FILIGREE_SHATTER_DENOM: u32 = 1000;
pub const SILVER_FILIGREE_SHATTER_DENOM_WITH_FF: u32 = 2000;

pub const STAR_TILE_LEVELUP_DENOM: u32 = 4;
pub const STAR_TILE_LEVELUP_NUMER: u32 = 1;
pub const STAR_TILE_LEVELUP_NUMER_WITH_FF: u32 = 2;

/// Sweepstakes round-start roll uses this range without Fortune's Favor.
pub const SWEEPSTAKES_ROLL_SPACE: u32 = 4;
/// With Fortune's Favor, each payout (+¥2, +¥4, nothing) has equal weight.
pub const SWEEPSTAKES_ROLL_SPACE_WITH_FF: u32 = 6;

/// Human-readable equal third for tooltips (matches `SWEEPSTAKES_ROLL_SPACE_WITH_FF`).
pub const SWEEPSTAKES_EACH_THIRD_LABEL: &str = "1/3";

/// Inputs for live relic description expansion.
pub struct RelicDescContext<'a> {
    pub id: RelicId,
    pub counters: &'a BTreeMap<RelicId, i32>,
    pub gold: i32,
    pub relics: Option<&'a RelicState>,
    pub slot: Option<usize>,
    pub ghost_hand_chips_preview: Option<i32>,
    pub wing: Option<u32>,
    /// When false, live tokens use design defaults (archive / static catalog).
    pub live: bool,
}

impl RelicDescContext<'_> {
    fn has_fortunes_favor(&self) -> bool {
        self.relics
            .is_some_and(|r| r.has(RelicId::FortunesFavor))
    }

    fn counter(&self, id: RelicId, design_default: i32) -> i32 {
        if self.live {
            self.counters.get(&id).copied().unwrap_or(design_default)
        } else {
            design_default
        }
    }
}

fn ff_paper_destroy_denom(ctx: &RelicDescContext<'_>) -> u32 {
    if ctx.live && ctx.has_fortunes_favor() {
        PAPER_LANTERN_DESTROY_DENOM_WITH_FF
    } else {
        PAPER_LANTERN_DESTROY_DENOM
    }
}

fn ff_stone_destroy_denom(ctx: &RelicDescContext<'_>) -> u32 {
    if ctx.live && ctx.has_fortunes_favor() {
        SILVER_FILIGREE_SHATTER_DENOM_WITH_FF
    } else {
        SILVER_FILIGREE_SHATTER_DENOM
    }
}

fn ff_star_levelup_label(ctx: &RelicDescContext<'_>) -> String {
    let numer = if ctx.live && ctx.has_fortunes_favor() {
        STAR_TILE_LEVELUP_NUMER_WITH_FF
    } else {
        STAR_TILE_LEVELUP_NUMER
    };
    format!("{numer}-in-{STAR_TILE_LEVELUP_DENOM}")
}

fn ff_sweepstakes_odds(ctx: &RelicDescContext<'_>) -> &'static str {
    if ctx.live && ctx.has_fortunes_favor() {
        SWEEPSTAKES_EACH_THIRD_LABEL
    } else {
        "25% +¥2, 25% +¥4, 50% nothing"
    }
}

fn replace_ff_token(key: &str, ctx: &RelicDescContext<'_>) -> Option<String> {
    match key {
        "ff_paper_destroy" => Some(ff_paper_destroy_denom(ctx).to_string()),
        "ff_paper_destroy_without" => Some(PAPER_LANTERN_DESTROY_DENOM.to_string()),
        "ff_paper_destroy_with" => Some(PAPER_LANTERN_DESTROY_DENOM_WITH_FF.to_string()),
        "ff_stone_destroy" => Some(ff_stone_destroy_denom(ctx).to_string()),
        "ff_silver_shatter_without" => Some(SILVER_FILIGREE_SHATTER_DENOM.to_string()),
        "ff_silver_shatter_with" => Some(SILVER_FILIGREE_SHATTER_DENOM_WITH_FF.to_string()),
        "ff_star_chance" => Some(ff_star_levelup_label(ctx)),
        "ff_star_numer_without" => Some(STAR_TILE_LEVELUP_NUMER.to_string()),
        "ff_star_numer_with" => Some(STAR_TILE_LEVELUP_NUMER_WITH_FF.to_string()),
        "ff_star_denom" => Some(STAR_TILE_LEVELUP_DENOM.to_string()),
        "ff_sweepstakes_odds" => Some(ff_sweepstakes_odds(ctx).to_string()),
        "ff_sweepstakes_each" => Some(SWEEPSTAKES_EACH_THIRD_LABEL.to_string()),
        _ => None,
    }
}

fn replace_relic_token(key: &str, ctx: &RelicDescContext<'_>) -> Option<String> {
    match (ctx.id, key) {
        (RelicId::MeltingIce, "chips_left") => Some(
            ctx.counter(RelicId::MeltingIce, MELTING_ICE_START_CHIPS)
                .to_string(),
        ),
        (RelicId::Taotie, "devoured") => {
            let chips = ctx.counter(RelicId::Taotie, 0).max(0);
            Some((chips / TAOTIE_CHIPS_PER_DEVOURED).to_string())
        }
        (RelicId::Taotie, "taotie_chips") => {
            Some(ctx.counter(RelicId::Taotie, 0).max(0).to_string())
        }
        (RelicId::SilkThread, "silk_mult_left") => {
            let thread = ctx.counter(RelicId::SilkThread, 40);
            Some(format!("{:.1}", thread as f64 / 10.0))
        }
        (RelicId::SilkMoth, "silk_moth_yen") => {
            Some(ctx.counter(RelicId::SilkMoth, 0).max(0).to_string())
        }
        (RelicId::XxxlEgg, "egg_charges") => {
            Some(ctx.counter(RelicId::XxxlEgg, 3).max(0).to_string())
        }
        (RelicId::IGotAGuy, "restocks_left") => {
            Some(ctx.counter(RelicId::IGotAGuy, 4).max(0).to_string())
        }
        (RelicId::TeaCeremony, "tea_next") => {
            let phase = ctx.counter(RelicId::TeaCeremony, 0).clamp(0, 3);
            let names = ["Harmony", "Respect", "Purity", "Tranquility"];
            Some(names[phase as usize].to_string())
        }
        (RelicId::TeaCeremony, "tea_hands_left") => {
            let phase = ctx.counter(RelicId::TeaCeremony, 0).clamp(0, 3);
            Some((4 - phase).to_string())
        }
        (RelicId::Chrysalis, "chrysalis_excess") => {
            let excess = ctx.counter(RelicId::MonarchButterfly, 0).max(0);
            Some(excess.to_string())
        }
        (RelicId::Chrysalis, "chrysalis_need") => {
            Some(CHRYSALIS_HATCH_EXCESS_THRESHOLD.max(1).to_string())
        }
        (RelicId::MonarchButterfly, "monarch_tier") => {
            let excess = ctx.counter(RelicId::MonarchButterfly, 0).max(0);
            Some(monarch_butterfly_tier(excess).to_string())
        }
        (RelicId::MonarchButterfly, "monarch_chips") => {
            let excess = ctx.counter(RelicId::MonarchButterfly, 0).max(0);
            Some(monarch_butterfly_bonus_chips(excess).to_string())
        }
        (RelicId::MonarchButterfly, "monarch_excess") => {
            Some(ctx.counter(RelicId::MonarchButterfly, 0).max(0).to_string())
        }
        (RelicId::MonarchButterfly, "monarch_next") => {
            let excess = ctx.counter(RelicId::MonarchButterfly, 0).max(0);
            Some(
                monarch_next_tier_excess_floor(excess)
                    .map(|n| format!("next tier ≥{n}"))
                    .unwrap_or_else(|| "max tier".to_string()),
            )
        }
        (RelicId::Humility, "humility_streak") => {
            Some(ctx.counter(RelicId::Humility, 0).max(0).to_string())
        }
        (RelicId::Humility, "humility_mult") => {
            let streak = ctx.counter(RelicId::Humility, 0).max(0);
            Some(format!("{:.1}", 0.5 * streak as f64))
        }
        (RelicId::Temperance, "temperance_mult") => {
            let stacks = ctx.counter(RelicId::Temperance, 0).max(0);
            Some(format!("{:.1}", (stacks as f64 / 8.0).min(10.0)))
        }
        (RelicId::Obsession, "obsession_rounds") => {
            Some(ctx.counter(RelicId::Obsession, 0).max(0).to_string())
        }
        (RelicId::Obsession, "obsession_mult") => {
            let rounds = ctx.counter(RelicId::Obsession, 0).max(0);
            Some(format!("{:.1}", 0.3 * rounds as f64))
        }
        (RelicId::Bonfire, "bonfire_sold") => {
            Some(ctx.counter(RelicId::Bonfire, 0).max(0).to_string())
        }
        (RelicId::Bonfire, "bonfire_mult") => {
            let sold = ctx.counter(RelicId::Bonfire, 0).max(0);
            Some(format!("{:.1}", 0.4 * sold as f64))
        }
        (RelicId::HungryGhost, "hungry_ghost_mult") => {
            let perm = ctx.counter(RelicId::HungryGhost, 0).max(0);
            Some(format!("{:.1}", (perm as f64 / 10.0).min(20.0)))
        }
        (RelicId::NestEgg, "nest_egg_rounds") => {
            Some(ctx.counter(RelicId::NestEgg, 0).max(0).to_string())
        }
        (RelicId::NestEgg, "nest_egg_sell") => Some(
            if ctx.live {
                relic_sell_price_live(RelicId::NestEgg, ctx.counters).to_string()
            } else {
                "4".to_string()
            },
        ),
        (RelicId::Kindling, "kindling_cashins") => {
            Some(ctx.counter(RelicId::Kindling, 0).max(0).to_string())
        }
        (RelicId::Kindling, "kindling_mult") => {
            let total = ctx.counter(RelicId::Kindling, 0).max(0);
            Some(format!("{:.1}", kindling_mult_bonus(total)))
        }
        (RelicId::Kindling, "kindling_cap") => Some(format!("{KINDLING_MULT_CAP:.1}")),
        (RelicId::Snowball, "snowball_stacks") => {
            let raw = ctx.counter(RelicId::Snowball, 0);
            Some(raw.clamp(0, SNOWBALL_STACK_CAP).to_string())
        }
        (RelicId::Snowball, "snowball_cap") => Some(SNOWBALL_STACK_CAP.to_string()),
        (RelicId::Snowball, "snowball_chips") => Some(
            snowball_score_chips(ctx.counter(RelicId::Snowball, 0)).to_string(),
        ),
        (RelicId::Kintsugi, "kintsugi_mult") => {
            Some(ctx.counter(RelicId::Kintsugi, 0).max(0).to_string())
        }
        (RelicId::Heirloom, "heirloom_blinds") => {
            Some(ctx.counter(RelicId::Heirloom, 0).max(0).to_string())
        }
        (RelicId::Heirloom, "heirloom_mult") => {
            let blinds = ctx.counter(RelicId::Heirloom, 0).max(0);
            Some(format!("{:.1}", (blinds as f64).min(12.0)))
        }
        (RelicId::GoldenEngine, "gold_held") => Some(ctx.gold.max(0).to_string()),
        (RelicId::GoldenEngine, "golden_mult") => {
            Some(golden_engine_mult_bonus(ctx.gold).to_string())
        }
        (RelicId::BeggarsCup, "beggar_ante") | (RelicId::BeggarsCup, "beggar_yen") => {
            Some(ctx.wing.unwrap_or(1).max(1).to_string())
        }
        (RelicId::WallWeaver, "wall_weaver_mult") => {
            let added = ctx.counter(RelicId::WallWeaver, 0).max(0);
            let overflow = if ctx.live
                && ctx
                    .relics
                    .is_some_and(|r| r.has(RelicId::StrengthInNumbers))
            {
                68
            } else {
                0
            };
            let excess = overflow + added;
            Some(format!("{:.1}", (0.35 * excess as f64).min(8.0)))
        }
        (RelicId::CurioCabinet, "curio_mult") => {
            let bonus = if let Some(relics) = ctx.relics {
                relics
                    .active
                    .iter()
                    .copied()
                    .filter(|&rid| rid != RelicId::CurioCabinet)
                    .map(|rid| relic_sell_price_live(rid, ctx.counters))
                    .sum::<u32>()
            } else {
                0
            };
            Some(format!("{:.1}", (bonus as f64).min(15.0)))
        }
        (RelicId::SolitarySage, "sage_empty_slots") => {
            let empty = ctx
                .relics
                .map(|r| r.max_slots.saturating_sub(r.active.len()))
                .unwrap_or(0);
            Some(empty.to_string())
        }
        (RelicId::SolitarySage, "sage_mult") => {
            let empty = ctx
                .relics
                .map(|r| r.max_slots.saturating_sub(r.active.len()))
                .unwrap_or(0);
            Some(format!("{:.1}", 2.5 * empty as f64))
        }
        (RelicId::MultiplierMaster, "multiplier_master_mult") => {
            let n = ctx.relics.map(|r| r.len()).unwrap_or(0);
            Some(format!("{:.1}", n as f64 * 1.5))
        }
        (RelicId::RiverRunner, "river_runner_chips") => {
            Some(ctx.counter(RelicId::RiverRunner, 0).max(0).to_string())
        }
        (RelicId::GhostHand, "ghost_hand_chips") => ctx
            .ghost_hand_chips_preview
            .map(|n| n.to_string())
            .or_else(|| if ctx.live { None } else { Some("0".to_string()) }),
        (RelicId::LotusBloom, "lotus_blooms") => {
            Some(ctx.counter(RelicId::LotusBloom, 0).max(0).to_string())
        }
        (RelicId::LotusBloom, "lotus_mult") => {
            let blooms = ctx.counter(RelicId::LotusBloom, 0).max(0);
            Some(format!("{:.1}", (0.75 * blooms as f64).min(12.0)))
        }
        _ => None,
    }
}

fn replace_token(key: &str, ctx: &RelicDescContext<'_>) -> Option<String> {
    replace_ff_token(key, ctx).or_else(|| replace_relic_token(key, ctx))
}

/// Replace every `{{token}}` in `template` for this relic.
pub fn expand_relic_description_templates(template: &str, ctx: &RelicDescContext<'_>) -> String {
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
        if let Some(v) = replace_token(key.trim(), ctx) {
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

/// Back-compat alias for static Fortune's Favor token expansion.
pub fn expand_relic_description_templates_static(s: &str) -> String {
    let ctx = RelicDescContext {
        id: RelicId::FortunesFavor,
        counters: &BTreeMap::new(),
        gold: 0,
        relics: None,
        slot: None,
        ghost_hand_chips_preview: None,
        wing: None,
        live: false,
    };
    expand_relic_description_templates(s, &ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_known_ff_tokens() {
        let s = "a {{ff_paper_destroy_without}} b {{ff_star_numer_with}}/{{ff_star_denom}} c";
        assert_eq!(
            expand_relic_description_templates_static(s),
            format!(
                "a {PAPER_LANTERN_DESTROY_DENOM} b {STAR_TILE_LEVELUP_NUMER_WITH_FF}/{STAR_TILE_LEVELUP_DENOM} c"
            )
        );
    }

    #[test]
    fn melting_ice_chips_live_vs_catalog() {
        let template = "+{{chips_left}} chips. Loses 12 chips per play.";
        let mut counters = BTreeMap::new();
        counters.insert(RelicId::MeltingIce, 87);
        let live = RelicDescContext {
            id: RelicId::MeltingIce,
            counters: &counters,
            gold: 0,
            relics: None,
            slot: None,
            ghost_hand_chips_preview: None,
            wing: None,
            live: true,
        };
        assert_eq!(
            expand_relic_description_templates(template, &live),
            "+87 chips. Loses 12 chips per play."
        );
        let catalog = RelicDescContext {
            live: false,
            ..live
        };
        assert_eq!(
            expand_relic_description_templates(template, &catalog),
            "+120 chips. Loses 12 chips per play."
        );
    }

    #[test]
    fn unknown_token_preserved() {
        let s = "x {{not_a_real_token}} y";
        let ctx = RelicDescContext {
            id: RelicId::MeltingIce,
            counters: &BTreeMap::new(),
            gold: 0,
            relics: None,
            slot: None,
            ghost_hand_chips_preview: None,
            wing: None,
            live: false,
        };
        assert_eq!(expand_relic_description_templates(s, &ctx), s);
    }
}
