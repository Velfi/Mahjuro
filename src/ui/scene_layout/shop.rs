use serde::{Deserialize, Serialize};

use crate::ui::placement::{ArrangeTarget, Node, Placement};

use super::fs::{load_positions, sanitize_placements, save_positions};

/// Serializable position data for the Shop scene.
///
/// Every field is a [`Placement`]; non-spatial tunables (column spreads,
/// camera multipliers) remain as plain scalars — they aren't point-like.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ShopPositions {
    pub relics: Placement,
    pub packs: Placement,
    pub talismans: Placement,
    pub ribbons: Placement,
    pub relic_spread_nx: f32,
    pub ribbon_spread_nx: f32,
    pub talisman_spread_nx: f32,
    pub counter: Placement,
    pub relic_dish: Placement,
    pub talisman_tray: Placement,
    pub ribbon_tray: Placement,
    pub coin_dish: Placement,
    pub sell_tray: Placement,
    pub lamp: Placement,
    pub book: Placement,
    pub reroll_prop: Placement,
    pub leave_prop: Placement,
    pub ofuda: Placement,
    pub hover_title_plaque: Placement,
    pub hover_desc_plaque: Placement,
    pub hover_owned_plaque: Placement,
    pub owned_talismans: Placement,
    pub smoke_curtain: Placement,
    pub camera_eye_y_frac: f32,
    pub camera_eye_z_frac: f32,
    pub camera_target_y_frac: f32,
    pub camera_target_z_frac: f32,
    pub celeb_pack_closeup: Placement,
    pub celeb_pack_reveal: Placement,
    pub celeb_zodiac: Placement,
}

pub const HFRAC_TO_MM: f32 =
    crate::ui::layout::TILE_WIDTH_MM / crate::ui::layout::HAND_SLOT_W_RATIO;

pub const CANONICAL_WINDOW_W: f32 = 1200.0;

impl Default for ShopPositions {
    fn default() -> Self {
        Self {
            relics: Placement::at(0.22, 0.31, 39.431_37),
            packs: Placement {
                nx: 0.424_016_2,
                ny: 0.478_250_95,
                lift_mm: 43.875_48,
                rx_deg: -16.0,
                ry_deg: 0.0,
                rz_deg: 1.0,
            },
            talismans: Placement {
                nx: 0.58,
                ny: 0.333_764_26,
                lift_mm: 39.431_37,
                rx_deg: -8.0,
                ry_deg: 0.0,
                rz_deg: -27.0,
            },
            ribbons: Placement::at(0.76, 1.571_634_9, -105.245_094),
            relic_spread_nx: 0.075,
            ribbon_spread_nx: 0.050,
            talisman_spread_nx: 0.055,
            counter: Placement::at(0.5, 0.35, 0.0),
            relic_dish: Placement::at(0.20, 0.84, 0.0),
            talisman_tray: Placement::at(0.38, 0.84, 0.0),
            ribbon_tray: Placement::at(0.56, 0.84, 0.0),
            coin_dish: Placement::at(0.742_847_26, 0.84, 0.0),
            sell_tray: Placement::at(0.477_812_53, 0.816_235_66, 35.879_898),
            lamp: Placement::at(0.5, 0.28, 180.575_9),
            book: Placement::at(0.731_041_67, 0.706_481_46, 0.0),
            reroll_prop: Placement {
                nx: 0.136_944_46,
                ny: 0.84,
                lift_mm: 140.379_88,
                rx_deg: 22.0,
                ry_deg: 0.0,
                rz_deg: 35.0,
            },
            leave_prop: Placement {
                nx: 0.854_499_16,
                ny: 0.788_335_8,
                lift_mm: 140.433_52,
                rx_deg: 20.0,
                ry_deg: 2.0,
                rz_deg: -38.0,
            },
            ofuda: Placement::at(0.050_925_925, -0.034_220_53, -4.086_677_6),
            hover_title_plaque: Placement::at(-0.001_736_110_1, 0.285_171_06, -16.900_726),
            hover_desc_plaque: Placement::at(-0.001_446_759_2, 0.0, -29.786_219),
            hover_owned_plaque: Placement::at(0.0, -0.22, 0.0),
            owned_talismans: Placement {
                nx: 0.0,
                ny: 0.0,
                lift_mm: 3.574_346_5,
                rx_deg: -34.0,
                ry_deg: 0.0,
                rz_deg: 0.0,
            },
            smoke_curtain: Placement::at(0.5, -0.642_072_3, -232.332_44),
            camera_eye_y_frac: 0.72,
            camera_eye_z_frac: 0.34,
            camera_target_y_frac: 0.18,
            camera_target_z_frac: 0.10,
            celeb_pack_closeup: Placement::at(0.383_101_85, 0.45, -2.573_530_7),
            celeb_pack_reveal: Placement::at(-3.352_761_3e-8, 0.55, 36.887_23),
            celeb_zodiac: Placement::at(0.5, 0.967_870_7, 242.013_03),
        }
    }
}

pub const SHOP_HIERARCHY: &[Node] = &[Node::Group {
    name: "shop",
    label: "Shop",
    children: &[
        Node::Leaf {
            name: "shop.counter",
            label: "Counter",
        },
        Node::Group {
            name: "shop.for_sale",
            label: "For-sale columns",
            children: &[
                Node::Leaf {
                    name: "shop.for_sale.relics",
                    label: "Relics",
                },
                Node::Leaf {
                    name: "shop.for_sale.packs",
                    label: "Packs",
                },
                Node::Leaf {
                    name: "shop.for_sale.talismans",
                    label: "Talismans",
                },
                Node::Leaf {
                    name: "shop.for_sale.ribbons",
                    label: "Ribbons",
                },
            ],
        },
        Node::Group {
            name: "shop.shelf",
            label: "Owned-item shelf",
            children: &[
                Node::Leaf {
                    name: "shop.shelf.relic_dish",
                    label: "Relic dish",
                },
                Node::Leaf {
                    name: "shop.shelf.talisman_tray",
                    label: "Talisman tray",
                },
                Node::Leaf {
                    name: "shop.shelf.ribbon_tray",
                    label: "Ribbon tray",
                },
                Node::Leaf {
                    name: "shop.shelf.coin_dish",
                    label: "Coin dish",
                },
                Node::Leaf {
                    name: "shop.shelf.sell_tray",
                    label: "Sell tray",
                },
                Node::Leaf {
                    name: "shop.shelf.owned_talismans",
                    label: "Owned talismans",
                },
            ],
        },
        Node::Group {
            name: "shop.props",
            label: "Props",
            children: &[
                Node::Leaf {
                    name: "shop.props.lamp",
                    label: "Lamp",
                },
                Node::Leaf {
                    name: "shop.props.book",
                    label: "Journal book",
                },
                Node::Leaf {
                    name: "shop.props.reroll_prop",
                    label: "Restock prop",
                },
                Node::Leaf {
                    name: "shop.props.leave_prop",
                    label: "Leave prop",
                },
                Node::Leaf {
                    name: "shop.props.ofuda",
                    label: "Ofuda sign",
                },
                Node::Leaf {
                    name: "shop.props.smoke_curtain",
                    label: "Smoke curtain",
                },
            ],
        },
        Node::Group {
            name: "shop.hover",
            label: "Hover plaques",
            children: &[
                Node::Leaf {
                    name: "shop.hover.title_plaque",
                    label: "Title plaque",
                },
                Node::Leaf {
                    name: "shop.hover.desc_plaque",
                    label: "Description plaque",
                },
                Node::Leaf {
                    name: "shop.hover.owned_plaque",
                    label: "Owned item plaque",
                },
            ],
        },
        Node::Group {
            name: "shop.celebrations",
            label: "Celebrations",
            children: &[
                Node::Leaf {
                    name: "shop.celebrations.pack_closeup",
                    label: "Pack closeup",
                },
                Node::Leaf {
                    name: "shop.celebrations.pack_reveal",
                    label: "Pack reveal",
                },
                Node::Leaf {
                    name: "shop.celebrations.zodiac",
                    label: "Zodiac ribbon",
                },
            ],
        },
    ],
}];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShopField {
    Relics,
    Packs,
    Talismans,
    Ribbons,
    Counter,
    RelicDish,
    TalismanTray,
    RibbonTray,
    CoinDish,
    SellTray,
    Lamp,
    Book,
    RerollProp,
    LeaveProp,
    Ofuda,
    HoverTitlePlaque,
    HoverDescPlaque,
    HoverOwnedPlaque,
    OwnedTalismans,
    SmokeCurtain,
    CelebPackCloseup,
    CelebPackReveal,
    CelebZodiac,
}

pub fn lookup_shop_field(name: &str) -> Option<ShopField> {
    Some(match name {
        "shop.counter" => ShopField::Counter,
        "shop.for_sale.relics" => ShopField::Relics,
        "shop.for_sale.packs" => ShopField::Packs,
        "shop.for_sale.talismans" => ShopField::Talismans,
        "shop.for_sale.ribbons" => ShopField::Ribbons,
        "shop.shelf.relic_dish" => ShopField::RelicDish,
        "shop.shelf.talisman_tray" => ShopField::TalismanTray,
        "shop.shelf.ribbon_tray" => ShopField::RibbonTray,
        "shop.shelf.coin_dish" => ShopField::CoinDish,
        "shop.shelf.sell_tray" => ShopField::SellTray,
        "shop.props.lamp" => ShopField::Lamp,
        "shop.props.book" => ShopField::Book,
        "shop.props.reroll_prop" => ShopField::RerollProp,
        "shop.props.leave_prop" => ShopField::LeaveProp,
        "shop.props.ofuda" => ShopField::Ofuda,
        "shop.hover.title_plaque" => ShopField::HoverTitlePlaque,
        "shop.hover.desc_plaque" => ShopField::HoverDescPlaque,
        "shop.hover.owned_plaque" => ShopField::HoverOwnedPlaque,
        "shop.shelf.owned_talismans" => ShopField::OwnedTalismans,
        "shop.props.smoke_curtain" => ShopField::SmokeCurtain,
        "shop.celebrations.pack_closeup" => ShopField::CelebPackCloseup,
        "shop.celebrations.pack_reveal" => ShopField::CelebPackReveal,
        "shop.celebrations.zodiac" => ShopField::CelebZodiac,
        _ => return None,
    })
}

#[cfg(test)]
pub fn shop_field_path(field: ShopField) -> &'static str {
    match field {
        ShopField::Counter => "shop.counter",
        ShopField::Relics => "shop.for_sale.relics",
        ShopField::Packs => "shop.for_sale.packs",
        ShopField::Talismans => "shop.for_sale.talismans",
        ShopField::Ribbons => "shop.for_sale.ribbons",
        ShopField::RelicDish => "shop.shelf.relic_dish",
        ShopField::TalismanTray => "shop.shelf.talisman_tray",
        ShopField::RibbonTray => "shop.shelf.ribbon_tray",
        ShopField::CoinDish => "shop.shelf.coin_dish",
        ShopField::SellTray => "shop.shelf.sell_tray",
        ShopField::Lamp => "shop.props.lamp",
        ShopField::Book => "shop.props.book",
        ShopField::RerollProp => "shop.props.reroll_prop",
        ShopField::LeaveProp => "shop.props.leave_prop",
        ShopField::Ofuda => "shop.props.ofuda",
        ShopField::HoverTitlePlaque => "shop.hover.title_plaque",
        ShopField::HoverDescPlaque => "shop.hover.desc_plaque",
        ShopField::HoverOwnedPlaque => "shop.hover.owned_plaque",
        ShopField::OwnedTalismans => "shop.shelf.owned_talismans",
        ShopField::SmokeCurtain => "shop.props.smoke_curtain",
        ShopField::CelebPackCloseup => "shop.celebrations.pack_closeup",
        ShopField::CelebPackReveal => "shop.celebrations.pack_reveal",
        ShopField::CelebZodiac => "shop.celebrations.zodiac",
    }
}

impl ShopField {
    pub const ALL: &'static [ShopField] = &[
        ShopField::Relics,
        ShopField::Packs,
        ShopField::Talismans,
        ShopField::Ribbons,
        ShopField::Counter,
        ShopField::RelicDish,
        ShopField::TalismanTray,
        ShopField::RibbonTray,
        ShopField::CoinDish,
        ShopField::SellTray,
        ShopField::Lamp,
        ShopField::Book,
        ShopField::RerollProp,
        ShopField::LeaveProp,
        ShopField::Ofuda,
        ShopField::HoverTitlePlaque,
        ShopField::HoverDescPlaque,
        ShopField::HoverOwnedPlaque,
        ShopField::OwnedTalismans,
        ShopField::SmokeCurtain,
        ShopField::CelebPackCloseup,
        ShopField::CelebPackReveal,
        ShopField::CelebZodiac,
    ];
}

impl ShopPositions {
    pub fn field_mut(&mut self, field: ShopField) -> &mut Placement {
        match field {
            ShopField::Relics => &mut self.relics,
            ShopField::Packs => &mut self.packs,
            ShopField::Talismans => &mut self.talismans,
            ShopField::Ribbons => &mut self.ribbons,
            ShopField::Counter => &mut self.counter,
            ShopField::RelicDish => &mut self.relic_dish,
            ShopField::TalismanTray => &mut self.talisman_tray,
            ShopField::RibbonTray => &mut self.ribbon_tray,
            ShopField::CoinDish => &mut self.coin_dish,
            ShopField::SellTray => &mut self.sell_tray,
            ShopField::Lamp => &mut self.lamp,
            ShopField::Book => &mut self.book,
            ShopField::RerollProp => &mut self.reroll_prop,
            ShopField::LeaveProp => &mut self.leave_prop,
            ShopField::Ofuda => &mut self.ofuda,
            ShopField::HoverTitlePlaque => &mut self.hover_title_plaque,
            ShopField::HoverDescPlaque => &mut self.hover_desc_plaque,
            ShopField::HoverOwnedPlaque => &mut self.hover_owned_plaque,
            ShopField::OwnedTalismans => &mut self.owned_talismans,
            ShopField::SmokeCurtain => &mut self.smoke_curtain,
            ShopField::CelebPackCloseup => &mut self.celeb_pack_closeup,
            ShopField::CelebPackReveal => &mut self.celeb_pack_reveal,
            ShopField::CelebZodiac => &mut self.celeb_zodiac,
        }
    }

    pub fn field_ref(&self, field: ShopField) -> &Placement {
        match field {
            ShopField::Relics => &self.relics,
            ShopField::Packs => &self.packs,
            ShopField::Talismans => &self.talismans,
            ShopField::Ribbons => &self.ribbons,
            ShopField::Counter => &self.counter,
            ShopField::RelicDish => &self.relic_dish,
            ShopField::TalismanTray => &self.talisman_tray,
            ShopField::RibbonTray => &self.ribbon_tray,
            ShopField::CoinDish => &self.coin_dish,
            ShopField::SellTray => &self.sell_tray,
            ShopField::Lamp => &self.lamp,
            ShopField::Book => &self.book,
            ShopField::RerollProp => &self.reroll_prop,
            ShopField::LeaveProp => &self.leave_prop,
            ShopField::Ofuda => &self.ofuda,
            ShopField::HoverTitlePlaque => &self.hover_title_plaque,
            ShopField::HoverDescPlaque => &self.hover_desc_plaque,
            ShopField::HoverOwnedPlaque => &self.hover_owned_plaque,
            ShopField::OwnedTalismans => &self.owned_talismans,
            ShopField::SmokeCurtain => &self.smoke_curtain,
            ShopField::CelebPackCloseup => &self.celeb_pack_closeup,
            ShopField::CelebPackReveal => &self.celeb_pack_reveal,
            ShopField::CelebZodiac => &self.celeb_zodiac,
        }
    }
}

impl ArrangeTarget for ShopPositions {
    fn placement_mut(&mut self, name: &str) -> Option<&mut Placement> {
        lookup_shop_field(name).map(|f| self.field_mut(f))
    }

    fn placement(&self, name: &str) -> Option<&Placement> {
        lookup_shop_field(name).map(|f| self.field_ref(f))
    }

    fn hierarchy(&self) -> &'static [Node] {
        SHOP_HIERARCHY
    }
}

pub fn load_shop_positions() -> ShopPositions {
    let mut loaded = load_positions("shop.json");
    sanitize_shop_positions(&mut loaded);
    loaded
}

pub fn sanitize_shop_positions(p: &mut ShopPositions) {
    sanitize_placements("shop", p, ShopField::ALL, |positions, field| {
        positions.field_mut(field)
    });
}

pub fn save_shop_positions(pos: &ShopPositions) -> anyhow::Result<()> {
    save_positions("shop.json", "shop", pos)
}
