use crate::ui::placement::Placement;

#[derive(Clone, Debug)]
pub struct CollectionPositions {
    pub pedestal: Placement,
    /// Per-cubby offset applied to every Zodiac ribbon in the Archive grid.
    pub cubby_zodiac: Placement,
}

impl Default for CollectionPositions {
    fn default() -> Self {
        Self {
            pedestal: Placement::at(0.0, 0.0, 0.0),
            cubby_zodiac: Placement::at(0.0, 0.0, 0.0),
        }
    }
}
