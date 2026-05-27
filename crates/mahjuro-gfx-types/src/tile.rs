use serde::{Deserialize, Serialize};

/// Mahjong tile size preset. Proportions are taken from Wikipedia's
/// "Mahjong tiles" article and reflect the canonical real-world dimensions
/// of three common regional sets. Each preset controls the face aspect
/// (long edge / short edge) and the slab thickness relative to the short
/// edge — i.e. it changes the *shape* of every rendered tile, not just a
/// uniform size scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TilePreset {
    /// Chinese standard, ~30 × 20 × 15 mm.
    Chinese,
    /// Japanese-style proportions, ~26 × 19 × 16 mm — chunkier and squarer.
    Japanese,
    /// American mah jongg, ~32 × 25 × 19 mm — largest.
    American,
}

impl TilePreset {
    pub fn next(self) -> Self {
        match self {
            Self::Chinese => Self::Japanese,
            Self::Japanese => Self::American,
            Self::American => Self::Chinese,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Chinese => Self::American,
            Self::Japanese => Self::Chinese,
            Self::American => Self::Japanese,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Chinese => "Chinese (30×20×15mm)",
            Self::Japanese => "Japanese (26×19×16mm)",
            Self::American => "American (32×25×19mm)",
        }
    }

    /// Face long edge divided by short edge.
    pub fn face_long_ratio(self) -> f32 {
        match self {
            Self::Chinese => 30.0 / 20.0,
            Self::Japanese => 26.0 / 19.0,
            Self::American => 32.0 / 25.0,
        }
    }

    /// Slab thickness divided by short edge.
    pub fn thickness_ratio(self) -> f32 {
        match self {
            Self::Chinese => 15.0 / 20.0,
            Self::Japanese => 16.0 / 19.0,
            Self::American => 19.0 / 25.0,
        }
    }
}

/// Tile material / colour scheme. Controls the procedural surface
/// appearance in the tile shader — ivory+bamboo, plastic, etc.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub enum TileMaterial {
    /// Traditional ivory face on a bamboo body.
    #[default]
    Bamboo,
    /// Mint-green plastic with a bright white face (common mass-produced set).
    Plastic,
    /// Blonde bekko — honey-amber keratin with dark mahogany mottling.
    TortoiseShell,
}

impl TileMaterial {
    pub fn next(self) -> Self {
        match self {
            Self::Bamboo => Self::Plastic,
            Self::Plastic => Self::TortoiseShell,
            Self::TortoiseShell => Self::Bamboo,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Bamboo => Self::TortoiseShell,
            Self::Plastic => Self::Bamboo,
            Self::TortoiseShell => Self::Plastic,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bamboo => "Bamboo & Ivory",
            Self::Plastic => "Plastic",
            Self::TortoiseShell => "Tortoise Shell",
        }
    }

    pub fn bonus_description(self) -> &'static str {
        match self {
            Self::Bamboo => "+1 Hand per round",
            Self::Plastic => "+1 Discard per round",
            Self::TortoiseShell => "+¥10 starting yen",
        }
    }
}
