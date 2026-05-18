//! Per-ante blind score targets (Balatro-style).
//!
//! Each ante has a chip base of `base_target * TARGET_SCALING^(ante - 1)`.
//! Small / Big / Boss multiply that base by 1× / 1.5× / 2×.

use crate::core::rules::BlindKind;

/// Spring-stake chip base for ante 1 Small Blind (`base_target` before stake mult).
pub const DEFAULT_BASE_TARGET: u32 = 500;

/// Per-ante multiplier on the run's `base_target`.
pub const TARGET_SCALING: f32 = 1.6;

/// Small-blind multiplier for the ante's chip base.
pub const SMALL_MULT: f32 = 1.0;
/// Big-blind multiplier.
pub const BIG_MULT: f32 = 1.5;
/// Boss-blind multiplier (before per-boss hooks such as Famine ×2).
pub const BOSS_MULT: f32 = 2.0;

/// Chip base for an ante (Small-blind equivalent), before blind-type multiplier.
pub fn ante_chip_base(ante: u32, base_target: u32) -> u32 {
    let ante = ante.max(1);
    let exponent = (ante - 1) as i32;
    let factor = TARGET_SCALING.powi(exponent);
    let raw = (base_target as f64) * (factor as f64);
    raw.round().max(1.0) as u32
}

/// Score required to clear a blind at the given ante.
pub fn score_for(ante: u32, blind: BlindKind, base_target: u32) -> u32 {
    let base = ante_chip_base(ante, base_target);
    let mult = blind.target_multiplier();
    let raw = (base as f64) * (mult as f64);
    raw.round().max(1.0) as u32
}

impl BlindKind {
    /// Target multiplier applied to the ante chip base.
    pub fn target_multiplier(self) -> f32 {
        match self {
            BlindKind::Small => SMALL_MULT,
            BlindKind::Big => BIG_MULT,
            BlindKind::Boss => BOSS_MULT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ante_one_matches_base_and_blind_mults() {
        assert_eq!(ante_chip_base(1, DEFAULT_BASE_TARGET), DEFAULT_BASE_TARGET);
        assert_eq!(score_for(1, BlindKind::Small, DEFAULT_BASE_TARGET), DEFAULT_BASE_TARGET);
        assert_eq!(score_for(1, BlindKind::Big, DEFAULT_BASE_TARGET), 750);
        assert_eq!(score_for(1, BlindKind::Boss, DEFAULT_BASE_TARGET), 1000);
    }

    #[test]
    fn scales_exponentially_per_ante() {
        assert_eq!(ante_chip_base(3, DEFAULT_BASE_TARGET), 1280);
        assert_eq!(score_for(3, BlindKind::Boss, DEFAULT_BASE_TARGET), 2560);
    }
}
