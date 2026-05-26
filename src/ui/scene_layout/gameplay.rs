//! Tunables for gameplay that are not authored as `gameplay.glb` spawn empties.
//!
//! Prop positions, hand rack, relics, plinths, consumables, and action buttons
//! come from GLB markers via [`crate::scenes::gameplay::glb_anchors`].

/// Non-GLB gameplay layout knobs (2D score cascade reel).
#[derive(Clone, Debug)]
pub struct GameplayPositions {
    /// Lift (mm) for cascade reel / popup anchors above the 2D score strip.
    pub score_reel_lift_mm: f32,
}

impl Default for GameplayPositions {
    fn default() -> Self {
        Self {
            score_reel_lift_mm: 139.161_16,
        }
    }
}
