use crate::ui::placement::Placement;

#[derive(Clone, Debug)]
pub struct CollectionPositions {
    pub cabinet: Placement,
    pub pedestal: Placement,
    pub featured_artifact: Placement,
    pub description_plaque: Placement,
    pub focus_card: Placement,
    /// Stats annex plaque under the focus description card (procedural archive HUD).
    pub stats_plaque: Placement,
    /// Per-cubby offset applied to every Zodiac ribbon in the Archive grid
    /// (shared arrange target so the whole row tunes together).
    pub cubby_zodiac: Placement,
}

impl Default for CollectionPositions {
    fn default() -> Self {
        Self {
            cabinet: Placement::at(0.0, 0.0, 0.0),
            pedestal: Placement::at(0.0, 0.0, 0.0),
            featured_artifact: Placement::at(0.0, 0.0, 0.0),
            description_plaque: Placement::at(0.0, 0.0, 0.0),
            focus_card: Placement::at(0.0, 0.0, 0.0),
            stats_plaque: Placement::at(0.0, 0.0, 0.0),
            cubby_zodiac: Placement::at(0.0, 0.0, 0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionField {
    Cabinet,
    Pedestal,
    FeaturedArtifact,
    DescriptionPlaque,
    FocusCard,
    StatsPlaque,
    CubbyZodiac,
}

pub fn collection_field_path(field: CollectionField) -> &'static str {
    match field {
        CollectionField::Cabinet => "collection.cabinet",
        CollectionField::Pedestal => "collection.pedestal",
        CollectionField::FeaturedArtifact => "collection.featured_artifact",
        CollectionField::DescriptionPlaque => "collection.description_plaque",
        CollectionField::FocusCard => "collection.focus_card",
        CollectionField::StatsPlaque => "collection.stats_plaque",
        CollectionField::CubbyZodiac => "collection.cubby_zodiac",
    }
}

impl CollectionField {
    pub const ALL: &'static [CollectionField] = &[
        CollectionField::Cabinet,
        CollectionField::Pedestal,
        CollectionField::FeaturedArtifact,
        CollectionField::DescriptionPlaque,
        CollectionField::FocusCard,
        CollectionField::StatsPlaque,
        CollectionField::CubbyZodiac,
    ];
}

impl CollectionPositions {
    pub fn field_ref(&self, field: CollectionField) -> &Placement {
        match field {
            CollectionField::Cabinet => &self.cabinet,
            CollectionField::Pedestal => &self.pedestal,
            CollectionField::FeaturedArtifact => &self.featured_artifact,
            CollectionField::DescriptionPlaque => &self.description_plaque,
            CollectionField::FocusCard => &self.focus_card,
            CollectionField::StatsPlaque => &self.stats_plaque,
            CollectionField::CubbyZodiac => &self.cubby_zodiac,
        }
    }
}
