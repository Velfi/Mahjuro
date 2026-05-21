use crate::ui::placement::Placement;

/// Pack-open and zodiac celebration overlays only.
///
/// Not used by the live storeroom scene. In-shop props and stock slots come from
/// `shop.glb` marker transforms (`room_glb.rs` / `shop/view.rs`), not this struct.
#[derive(Clone, Debug)]
pub struct ShopPositions {
    pub celeb_pack_reveal: Placement,
    pub celeb_zodiac: Placement,
}

impl Default for ShopPositions {
    fn default() -> Self {
        Self {
            celeb_pack_reveal: Placement::at(-3.352_761_3e-8, 0.55, 36.887_23),
            celeb_zodiac: Placement {
                nx: 0.0,
                ny: -0.12,
                lift_mm: 0.0,
                rx_deg: -12.0,
                ry_deg: 0.0,
                rz_deg: 0.0,
            },
        }
    }
}
