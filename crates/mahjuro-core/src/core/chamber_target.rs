//! Per-wing chamber score targets (Balatro-style).
//!
//! Each wing has a chip base of `base_target * TARGET_SCALING^(wing - 1)`.
//! Small / Big / Boss multiply that base by 1× / 1.5× / 2×.

use crate::core::rules::ChamberKind;

/// Spring-season chip base for wing 1 Small Chamber (`base_target` before season mult).
pub const DEFAULT_BASE_TARGET: u32 = 1000;

/// Per-wing multiplier on the run's `base_target`.
///
/// Golden ratio φ = (1+√5)/2; decimal expansion [OEIS A001622](https://oeis.org/A001622).
pub const TARGET_SCALING: f32 = 1.618_034;

/// Small-chamber multiplier for the wing's chip base.
pub const SMALL_MULT: f32 = 1.0;
/// Big-chamber multiplier.
pub const BIG_MULT: f32 = 1.5;
/// Boss-chamber multiplier (before per-boss hooks such as Famine ×2).
pub const BOSS_MULT: f32 = 2.0;

/// Chip base for a wing (Small-chamber equivalent), before chamber-type multiplier.
pub fn wing_chip_base(wing: u32, base_target: u32) -> u32 {
    let wing = wing.max(1);
    let exponent = (wing - 1) as i32;
    let factor = TARGET_SCALING.powi(exponent);
    let raw = (base_target as f64) * (factor as f64);
    raw.round().max(1.0) as u32
}

/// Score required to clear a chamber at the given wing.
pub fn score_for(wing: u32, chamber: ChamberKind, base_target: u32) -> u32 {
    let base = wing_chip_base(wing, base_target);
    let mult = chamber.target_multiplier();
    let raw = (base as f64) * (mult as f64);
    raw.round().max(1.0) as u32
}

impl ChamberKind {
    /// Target multiplier applied to the wing chip base.
    pub fn target_multiplier(self) -> f32 {
        match self {
            ChamberKind::Small => SMALL_MULT,
            ChamberKind::Big => BIG_MULT,
            ChamberKind::Ordeal => BOSS_MULT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wing_one_matches_base_and_chamber_mults() {
        assert_eq!(wing_chip_base(1, DEFAULT_BASE_TARGET), DEFAULT_BASE_TARGET);
        assert_eq!(
            score_for(1, ChamberKind::Small, DEFAULT_BASE_TARGET),
            DEFAULT_BASE_TARGET
        );
        assert_eq!(score_for(1, ChamberKind::Big, DEFAULT_BASE_TARGET), 648);
        assert_eq!(score_for(1, ChamberKind::Ordeal, DEFAULT_BASE_TARGET), 864);
    }

    #[test]
    fn scales_exponentially_per_wing() {
        assert_eq!(wing_chip_base(3, DEFAULT_BASE_TARGET), 1131);
        assert_eq!(score_for(3, ChamberKind::Ordeal, DEFAULT_BASE_TARGET), 2262);
    }
}
