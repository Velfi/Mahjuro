use crate::ui::placement::Placement;

/// Pack-open and zodiac celebration overlays only.
///
/// Not used by the live storeroom scene. In-shop props and stock slots come from
/// `shop.glb` marker transforms (`room_glb.rs` / `shop/view.rs`), not this struct.
#[derive(Clone, Debug)]
pub struct ShopPositions {
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
            celeb_pack_closeup: Placement::at(0.358_506_95, -1.991_540_2, -557.059_51),
            celeb_pack_reveal: Placement::at(-3.352_761_3e-8, 0.55, 36.887_23),
            celeb_zodiac: Placement {
                nx: 0.0,
                ny: 0.0,
                lift_mm: -523.66,
                rx_deg: -12.0,
                ry_deg: 0.0,
                rz_deg: 0.0,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShopField {
    CelebPackCloseup,
    CelebPackReveal,
    CelebZodiac,
}

pub fn lookup_shop_field(name: &str) -> Option<ShopField> {
    Some(match name {
        "shop.celebrations.pack_closeup" => ShopField::CelebPackCloseup,
        "shop.celebrations.pack_reveal" => ShopField::CelebPackReveal,
        "shop.celebrations.zodiac" => ShopField::CelebZodiac,
        _ => return None,
    })
}

pub fn shop_field_path(field: ShopField) -> &'static str {
    match field {
        ShopField::CelebPackCloseup => "shop.celebrations.pack_closeup",
        ShopField::CelebPackReveal => "shop.celebrations.pack_reveal",
        ShopField::CelebZodiac => "shop.celebrations.zodiac",
    }
}

impl ShopField {
    pub const ALL: &'static [ShopField] = &[
        ShopField::CelebPackCloseup,
        ShopField::CelebPackReveal,
        ShopField::CelebZodiac,
    ];
}

impl ShopPositions {
    pub fn field_mut(&mut self, field: ShopField) -> &mut Placement {
        match field {
            ShopField::CelebPackCloseup => &mut self.celeb_pack_closeup,
            ShopField::CelebPackReveal => &mut self.celeb_pack_reveal,
            ShopField::CelebZodiac => &mut self.celeb_zodiac,
        }
    }

    pub fn field_ref(&self, field: ShopField) -> &Placement {
        match field {
            ShopField::CelebPackCloseup => &self.celeb_pack_closeup,
            ShopField::CelebPackReveal => &self.celeb_pack_reveal,
            ShopField::CelebZodiac => &self.celeb_zodiac,
        }
    }
}
