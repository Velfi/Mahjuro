use serde::{Deserialize, Serialize};

use crate::ui::placement::{ArrangeTarget, Node, Placement};

use super::fs::{load_positions, sanitize_placements, save_positions};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TutorialPositions {
    pub shop_relic: Placement,
    pub shop_ribbon: Placement,
    pub shop_talisman: Placement,
    pub shop_pack: Placement,
    pub try_it_mirror: Placement,
    pub try_it_trigger: Placement,
}

impl Default for TutorialPositions {
    fn default() -> Self {
        Self {
            shop_relic: Placement::at(0.0, 0.0, 0.0),
            shop_ribbon: Placement::at(0.0, 0.0, 0.0),
            shop_talisman: Placement::at(0.0, 0.0, 0.0),
            shop_pack: Placement::at(0.0, 0.0, 0.0),
            try_it_mirror: Placement::at(0.0, 0.0, 0.0),
            try_it_trigger: Placement::at(0.0, 0.0, 0.0),
        }
    }
}

pub const TUTORIAL_HIERARCHY: &[Node] = &[Node::Group {
    name: "tutorial",
    label: "Tutorial",
    children: &[
        Node::Group {
            name: "tutorial.shop",
            label: "Shop preview",
            children: &[
                Node::Leaf {
                    name: "tutorial.shop.relic",
                    label: "Preview relic",
                },
                Node::Leaf {
                    name: "tutorial.shop.ribbon",
                    label: "Preview ribbon",
                },
                Node::Leaf {
                    name: "tutorial.shop.talisman",
                    label: "Preview talisman",
                },
                Node::Leaf {
                    name: "tutorial.shop.pack",
                    label: "Preview pack",
                },
            ],
        },
        Node::Group {
            name: "tutorial.try_it",
            label: "Try-it demo",
            children: &[
                Node::Leaf {
                    name: "tutorial.try_it.mirror",
                    label: "Play mirror",
                },
                Node::Leaf {
                    name: "tutorial.try_it.trigger",
                    label: "Trigger tablet",
                },
            ],
        },
    ],
}];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TutorialField {
    ShopRelic,
    ShopRibbon,
    ShopTalisman,
    ShopPack,
    TryItMirror,
    TryItTrigger,
}

pub fn lookup_tutorial_field(name: &str) -> Option<TutorialField> {
    Some(match name {
        "tutorial.shop.relic" => TutorialField::ShopRelic,
        "tutorial.shop.ribbon" => TutorialField::ShopRibbon,
        "tutorial.shop.talisman" => TutorialField::ShopTalisman,
        "tutorial.shop.pack" => TutorialField::ShopPack,
        "tutorial.try_it.mirror" => TutorialField::TryItMirror,
        "tutorial.try_it.trigger" => TutorialField::TryItTrigger,
        _ => return None,
    })
}

#[cfg(test)]
pub fn tutorial_field_path(field: TutorialField) -> &'static str {
    match field {
        TutorialField::ShopRelic => "tutorial.shop.relic",
        TutorialField::ShopRibbon => "tutorial.shop.ribbon",
        TutorialField::ShopTalisman => "tutorial.shop.talisman",
        TutorialField::ShopPack => "tutorial.shop.pack",
        TutorialField::TryItMirror => "tutorial.try_it.mirror",
        TutorialField::TryItTrigger => "tutorial.try_it.trigger",
    }
}

impl TutorialField {
    pub const ALL: &'static [TutorialField] = &[
        TutorialField::ShopRelic,
        TutorialField::ShopRibbon,
        TutorialField::ShopTalisman,
        TutorialField::ShopPack,
        TutorialField::TryItMirror,
        TutorialField::TryItTrigger,
    ];
}

impl TutorialPositions {
    pub fn field_mut(&mut self, field: TutorialField) -> &mut Placement {
        match field {
            TutorialField::ShopRelic => &mut self.shop_relic,
            TutorialField::ShopRibbon => &mut self.shop_ribbon,
            TutorialField::ShopTalisman => &mut self.shop_talisman,
            TutorialField::ShopPack => &mut self.shop_pack,
            TutorialField::TryItMirror => &mut self.try_it_mirror,
            TutorialField::TryItTrigger => &mut self.try_it_trigger,
        }
    }

    pub fn field_ref(&self, field: TutorialField) -> &Placement {
        match field {
            TutorialField::ShopRelic => &self.shop_relic,
            TutorialField::ShopRibbon => &self.shop_ribbon,
            TutorialField::ShopTalisman => &self.shop_talisman,
            TutorialField::ShopPack => &self.shop_pack,
            TutorialField::TryItMirror => &self.try_it_mirror,
            TutorialField::TryItTrigger => &self.try_it_trigger,
        }
    }
}

impl ArrangeTarget for TutorialPositions {
    fn placement_mut(&mut self, name: &str) -> Option<&mut Placement> {
        lookup_tutorial_field(name).map(|f| self.field_mut(f))
    }

    fn placement(&self, name: &str) -> Option<&Placement> {
        lookup_tutorial_field(name).map(|f| self.field_ref(f))
    }

    fn hierarchy(&self) -> &'static [Node] {
        TUTORIAL_HIERARCHY
    }
}

pub fn load_tutorial_positions() -> TutorialPositions {
    let mut loaded = load_positions("tutorial.json");
    sanitize_tutorial_positions(&mut loaded);
    loaded
}

pub fn sanitize_tutorial_positions(p: &mut TutorialPositions) {
    sanitize_placements("tutorial", p, TutorialField::ALL, |positions, field| {
        positions.field_mut(field)
    });
}

pub fn save_tutorial_positions(pos: &TutorialPositions) -> anyhow::Result<()> {
    save_positions("tutorial.json", "tutorial", pos)
}
