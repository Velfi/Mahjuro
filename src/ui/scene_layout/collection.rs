use serde::{Deserialize, Serialize};

use crate::ui::placement::{ArrangeTarget, Node, Placement};

use super::fs::{load_positions, sanitize_placements, save_positions};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectionPositions {
    pub cabinet: Placement,
    pub pedestal: Placement,
    pub featured_artifact: Placement,
    pub description_plaque: Placement,
    pub focus_card: Placement,
}

impl Default for CollectionPositions {
    fn default() -> Self {
        Self {
            cabinet: Placement::at(0.0, 0.0, 0.0),
            pedestal: Placement::at(0.0, 0.0, 0.0),
            featured_artifact: Placement::at(0.0, 0.0, 0.0),
            description_plaque: Placement::at(0.0, 0.0, 0.0),
            focus_card: Placement::at(0.0, 0.0, 0.0),
        }
    }
}

pub const COLLECTION_HIERARCHY: &[Node] = &[Node::Group {
    name: "collection",
    label: "Archive",
    children: &[
        Node::Leaf {
            name: "collection.cabinet",
            label: "Grid backdrop",
        },
        Node::Leaf {
            name: "collection.pedestal",
            label: "Orbit inspect anchor",
        },
        Node::Leaf {
            name: "collection.featured_artifact",
            label: "Featured artifact",
        },
        Node::Leaf {
            name: "collection.description_plaque",
            label: "Description plaque",
        },
        Node::Leaf {
            name: "collection.focus_card",
            label: "Focus description card",
        },
    ],
}];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionField {
    Cabinet,
    Pedestal,
    FeaturedArtifact,
    DescriptionPlaque,
    FocusCard,
}

pub fn lookup_collection_field(name: &str) -> Option<CollectionField> {
    Some(match name {
        "collection.cabinet" => CollectionField::Cabinet,
        "collection.pedestal" => CollectionField::Pedestal,
        "collection.featured_artifact" => CollectionField::FeaturedArtifact,
        "collection.description_plaque" => CollectionField::DescriptionPlaque,
        "collection.focus_card" => CollectionField::FocusCard,
        _ => return None,
    })
}

#[cfg(test)]
pub fn collection_field_path(field: CollectionField) -> &'static str {
    match field {
        CollectionField::Cabinet => "collection.cabinet",
        CollectionField::Pedestal => "collection.pedestal",
        CollectionField::FeaturedArtifact => "collection.featured_artifact",
        CollectionField::DescriptionPlaque => "collection.description_plaque",
        CollectionField::FocusCard => "collection.focus_card",
    }
}

impl CollectionField {
    pub const ALL: &'static [CollectionField] = &[
        CollectionField::Cabinet,
        CollectionField::Pedestal,
        CollectionField::FeaturedArtifact,
        CollectionField::DescriptionPlaque,
        CollectionField::FocusCard,
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
        }
    }

    pub fn field_ref(&self, field: CollectionField) -> &Placement {
        match field {
            CollectionField::Cabinet => &self.cabinet,
            CollectionField::Pedestal => &self.pedestal,
            CollectionField::FeaturedArtifact => &self.featured_artifact,
            CollectionField::DescriptionPlaque => &self.description_plaque,
            CollectionField::FocusCard => &self.focus_card,
        }
    }
}

impl ArrangeTarget for CollectionPositions {
    fn placement_mut(&mut self, name: &str) -> Option<&mut Placement> {
        lookup_collection_field(name).map(|f| self.field_mut(f))
    }

    fn placement(&self, name: &str) -> Option<&Placement> {
        lookup_collection_field(name).map(|f| self.field_ref(f))
    }

    fn hierarchy(&self) -> &'static [Node] {
        COLLECTION_HIERARCHY
    }
}

pub fn load_collection_positions() -> CollectionPositions {
    let mut loaded = load_positions("collection.json");
    sanitize_collection_positions(&mut loaded);
    loaded
}

pub fn sanitize_collection_positions(p: &mut CollectionPositions) {
    sanitize_placements("collection", p, CollectionField::ALL, |positions, field| {
        positions.field_mut(field)
    });
}

pub fn save_collection_positions(pos: &CollectionPositions) -> anyhow::Result<()> {
    save_positions("collection.json", "collection", pos)
}
