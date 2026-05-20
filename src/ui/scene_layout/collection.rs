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

pub fn lookup_collection_field(name: &str) -> Option<CollectionField> {
    Some(match name {
        "collection.cabinet" => CollectionField::Cabinet,
        "collection.pedestal" => CollectionField::Pedestal,
        "collection.featured_artifact" => CollectionField::FeaturedArtifact,
        "collection.description_plaque" => CollectionField::DescriptionPlaque,
        "collection.focus_card" => CollectionField::FocusCard,
        "collection.stats_plaque" => CollectionField::StatsPlaque,
        "collection.cubby_zodiac" => CollectionField::CubbyZodiac,
        _ => return None,
    })
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
    pub fn field_mut(&mut self, field: CollectionField) -> &mut Placement {
        match field {
            CollectionField::Cabinet => &mut self.cabinet,
            CollectionField::Pedestal => &mut self.pedestal,
            CollectionField::FeaturedArtifact => &mut self.featured_artifact,
            CollectionField::DescriptionPlaque => &mut self.description_plaque,
            CollectionField::FocusCard => &mut self.focus_card,
            CollectionField::StatsPlaque => &mut self.stats_plaque,
            CollectionField::CubbyZodiac => &mut self.cubby_zodiac,
        }
    }

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

