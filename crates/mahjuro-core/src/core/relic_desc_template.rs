//! `{{token}}` placeholders inside `assets/data/relics.json` descriptions.
//!
//! Expanded once when relic defs load so shop, collection, and HUD all match.
//! Unknown tokens are left unchanged so typos are visible during development.

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

fn replace_token(key: &str) -> Option<String> {
    match key.trim() {
        "ff_paper_destroy_without" => Some(PAPER_LANTERN_DESTROY_DENOM.to_string()),
        "ff_paper_destroy_with" => Some(PAPER_LANTERN_DESTROY_DENOM_WITH_FF.to_string()),
        "ff_silver_shatter_without" => Some(SILVER_FILIGREE_SHATTER_DENOM.to_string()),
        "ff_silver_shatter_with" => Some(SILVER_FILIGREE_SHATTER_DENOM_WITH_FF.to_string()),
        "ff_star_numer_without" => Some(STAR_TILE_LEVELUP_NUMER.to_string()),
        "ff_star_numer_with" => Some(STAR_TILE_LEVELUP_NUMER_WITH_FF.to_string()),
        "ff_star_denom" => Some(STAR_TILE_LEVELUP_DENOM.to_string()),
        "ff_sweepstakes_each" => Some(SWEEPSTAKES_EACH_THIRD_LABEL.to_string()),
        _ => None,
    }
}

/// Replace every `{{token}}` in `s` using [`replace_token`]. Unknown keys stay literal.
pub fn expand_relic_description_templates(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        let Some(end) = rest.find("}}") else {
            out.push_str("{{");
            out.push_str(rest);
            return out;
        };
        let key = &rest[..end];
        if let Some(v) = replace_token(key) {
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

    #[test]
    fn expand_known_tokens() {
        let s = "a {{ff_paper_destroy_without}} b {{ff_star_numer_with}}/{{ff_star_denom}} c";
        assert_eq!(
            expand_relic_description_templates(s),
            format!(
                "a {PAPER_LANTERN_DESTROY_DENOM} b {STAR_TILE_LEVELUP_NUMER_WITH_FF}/{STAR_TILE_LEVELUP_DENOM} c"
            )
        );
    }

    #[test]
    fn unknown_token_preserved() {
        let s = "x {{not_a_real_token}} y";
        assert_eq!(expand_relic_description_templates(s), s);
    }
}
