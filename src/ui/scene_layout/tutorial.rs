use crate::ui::placement::Placement;

#[derive(Clone, Debug)]
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

