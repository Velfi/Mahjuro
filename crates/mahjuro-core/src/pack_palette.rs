//! Per-pack celebration tints — backdrop washes, auras, tile glow, and lighting accents.

use crate::core::tile_pack::TilePackKind;

/// Per-pack colors for the tile-pack opening celebration (2D backdrop + accents).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackCelebrationPalette {
    pub foil: [f32; 4],
    pub seal: [f32; 4],
    pub bg: [f32; 4],
}

const HONORS: PackCelebrationPalette = PackCelebrationPalette {
    foil: [0.0235, 0.0235, 0.0196, 1.0],
    seal: [0.611, 0.438, 0.459, 1.0],
    bg: [0.055, 0.094, 0.220, 1.0],
};

const TERMINALS: PackCelebrationPalette = PackCelebrationPalette {
    foil: HONORS.foil,
    seal: [0.56, 0.14, 0.12, 1.0],
    bg: [0.102, 0.078, 0.071, 1.0],
};

const FLOWERS: PackCelebrationPalette = PackCelebrationPalette {
    foil: HONORS.foil,
    seal: [0.52, 0.14, 0.30, 1.0],
    bg: [0.110, 0.059, 0.118, 1.0],
};

const SOUZU: PackCelebrationPalette = PackCelebrationPalette {
    foil: HONORS.foil,
    seal: [0.78, 0.18, 0.14, 1.0],
    bg: [0.039, 0.102, 0.055, 1.0],
};

const PINZU: PackCelebrationPalette = PackCelebrationPalette {
    foil: HONORS.foil,
    seal: [0.58, 0.10, 0.18, 1.0],
    bg: [0.102, 0.055, 0.071, 1.0],
};

const MANZU: PackCelebrationPalette = PackCelebrationPalette {
    foil: HONORS.foil,
    seal: [0.72, 0.18, 0.18, 1.0],
    bg: [0.102, 0.078, 0.039, 1.0],
};

pub const fn for_kind(kind: TilePackKind) -> PackCelebrationPalette {
    match kind {
        TilePackKind::Honors => HONORS,
        TilePackKind::Terminals => TERMINALS,
        TilePackKind::Flowers => FLOWERS,
        TilePackKind::Souzu => SOUZU,
        TilePackKind::Pinzu => PINZU,
        TilePackKind::Manzu => MANZU,
    }
}
