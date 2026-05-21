use crate::ui::placement::Placement;

#[derive(Clone, Debug)]
pub struct CollectionPositions {
    pub cabinet: Placement,
    pub pedestal: Placement,
    pub featured_artifact: Placement,
    pub focus_card: Placement,
    /// Stats annex plaque under the focus description card (procedural archive HUD).
    pub stats_plaque: Placement,
    /// Per-cubby offset applied to every Zodiac ribbon in the Archive grid.
    pub cubby_zodiac: Placement,
}

impl Default for CollectionPositions {
    fn default() -> Self {
        Self {
            cabinet: Placement::at(0.0, 0.0, 0.0),
            pedestal: Placement::at(0.0, 0.0, 0.0),
            featured_artifact: Placement::at(0.0, 0.0, 0.0),
            focus_card: Placement::at(0.0, 0.0, 0.0),
            stats_plaque: Placement::at(0.0, 0.0, 0.0),
            cubby_zodiac: Placement::at(0.0, 0.0, 0.0),
        }
    }
}
