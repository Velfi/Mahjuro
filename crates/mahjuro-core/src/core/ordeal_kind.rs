//! Boss blind identifiers and presentation metadata without run hooks.

use serde::{Deserialize, Serialize};

use mahjuro_types::theme_tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrdealKind {
    Drought,
    Whisper,
    Tribute,
    Gate,
    Grove,
    Coin,
    #[serde(alias = "bloom")]
    Rot,
    Hermit,
    Forest,
    Bureaucrat,
    Drunkard,
    Ash,
    Furnace,
    Relic,
    Blight,
    Hex,
    Famine,
    Tempest,
    Censor,
    Mirror,
    Counterweight,
    TaxCollector,
    Dragon,
    House,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrdealTier {
    Soft,
    Medium,
    Hard,
    Final,
}

impl OrdealKind {
    /// Every boss variant, in `assets/textures/ordeal_icons/atlas.toml` row-major order.
    pub const ALL: &'static [OrdealKind] = &[
        OrdealKind::Drought,
        OrdealKind::Whisper,
        OrdealKind::Tribute,
        OrdealKind::Gate,
        OrdealKind::Grove,
        OrdealKind::Coin,
        OrdealKind::Rot,
        OrdealKind::Hermit,
        OrdealKind::Forest,
        OrdealKind::Bureaucrat,
        OrdealKind::Drunkard,
        OrdealKind::Ash,
        OrdealKind::Furnace,
        OrdealKind::Relic,
        OrdealKind::Blight,
        OrdealKind::Hex,
        OrdealKind::Famine,
        OrdealKind::Tempest,
        OrdealKind::Censor,
        OrdealKind::Mirror,
        OrdealKind::Counterweight,
        OrdealKind::TaxCollector,
        OrdealKind::Dragon,
        OrdealKind::House,
    ];

    /// Stable atlas cell id (`assets/data/ordeals.json` `id`, `textures/ordeal_icons/atlas.toml`).
    pub fn atlas_slug(self) -> &'static str {
        match self {
            OrdealKind::Drought => "drought",
            OrdealKind::Whisper => "whisper",
            OrdealKind::Tribute => "tribute",
            OrdealKind::Gate => "gate",
            OrdealKind::Grove => "grove",
            OrdealKind::Coin => "coin",
            OrdealKind::Rot => "rot",
            OrdealKind::Hermit => "hermit",
            OrdealKind::Forest => "forest",
            OrdealKind::Bureaucrat => "bureaucrat",
            OrdealKind::Drunkard => "drunkard",
            OrdealKind::Ash => "ash",
            OrdealKind::Furnace => "furnace",
            OrdealKind::Relic => "relic",
            OrdealKind::Blight => "blight",
            OrdealKind::Hex => "hex",
            OrdealKind::Famine => "famine",
            OrdealKind::Tempest => "tempest",
            OrdealKind::Censor => "censor",
            OrdealKind::Mirror => "mirror",
            OrdealKind::Counterweight => "counterweight",
            OrdealKind::TaxCollector => "tax_collector",
            OrdealKind::Dragon => "dragon",
            OrdealKind::House => "house",
        }
    }
}

impl OrdealTier {
    pub fn halo_color(self) -> [f32; 4] {
        match self {
            OrdealTier::Soft => theme_tokens::LAPIS,
            OrdealTier::Medium => theme_tokens::GOLD,
            OrdealTier::Hard => theme_tokens::AMBER,
            OrdealTier::Final => theme_tokens::RUBY,
        }
    }
}
