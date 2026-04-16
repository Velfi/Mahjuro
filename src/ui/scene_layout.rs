//! Serializable scene-layout position data.
//!
//! Every manually-placeable object is a [`Placement`] with a single
//! consistent coordinate system:
//!
//! - `nx`, `ny` — normalized window fractions (0 = left/top, 1 = right/bottom)
//! - `lift_mm` — physical millimeters above the felt, converted to world units
//!   via [`crate::ui::layout::LayoutResult::mm`]
//! - `rx_deg`, `ry_deg`, `rz_deg` — rotation in degrees (Z → Y → X order)
//!
//! Anchor-relative placements (plaque, coin pile, hand strip, yaku tablet,
//! action-bar bowl/mirror) still use the same units — the scene interprets
//! their `nx`/`ny` as fractional *offsets* against a Cassowary-derived anchor
//! rather than absolute screen positions, but the unit system is identical.
//!
//! ## Save / load
//!
//! Positions load from JSON in the app's config directory at startup.
//! Missing files or fields fall back to compiled-in [`Default`] values, so
//! shipping requires no JSON files.
//!
//! ## Arrange mode
//!
//! Both `ShopPositions` and `GameplayPositions` implement
//! [`crate::ui::placement::ArrangeTarget`]; the generic
//! [`crate::ui::placement::apply_arrange`] handler nudges any registered
//! placement by name. The debug menu discovers placements by iterating
//! the known names.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ui::placement::{ArrangeTarget, Node, Placement};

// ── File-system helpers ───────────────────────────────────────────────────────

const APP_DIR: &str = "Mahjuro";
const LAYOUTS_SUBDIR: &str = "layouts";

/// Returns the directory where layout JSON overrides are stored, creating it
/// if necessary.
fn layouts_dir() -> PathBuf {
    let base = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR)
        .join(LAYOUTS_SUBDIR);
    let _ = fs::create_dir_all(&base);
    base
}

// ── ShopPositions ─────────────────────────────────────────────────────────────

/// Serializable position data for the Shop scene.
///
/// Every field is a [`Placement`]; non-spatial tunables (column spreads,
/// camera multipliers) remain as plain scalars — they aren't point-like.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ShopPositions {
    // ── For-sale column cards ──────────────────────────────────────────────
    pub relics: Placement,
    pub packs: Placement,
    pub talismans: Placement,
    pub ribbons: Placement,
    /// Horizontal spread between cards in each column (window-width fraction).
    pub relic_spread_nx: f32,
    pub ribbon_spread_nx: f32,
    pub talisman_spread_nx: f32,

    // ── Counter + shelf ─────────────────────────────────────────────────────
    pub counter: Placement,
    pub relic_dish: Placement,
    pub talisman_tray: Placement,
    pub ribbon_tray: Placement,
    pub coin_dish: Placement,
    pub sell_tray: Placement,

    // ── Props ──────────────────────────────────────────────────────────────
    pub lamp: Placement,
    pub book: Placement,
    pub reroll_prop: Placement,
    pub leave_prop: Placement,
    /// Ofuda scroll above the shop entrance (formerly hard-coded `w*0.23`,
    /// tilt -82° around X). `rx_deg` defaults to -82.0.
    pub ofuda: Placement,

    // ── Hover plaques ──────────────────────────────────────────────────────
    /// Title+CTA plaque that floats above the currently hovered/focused item.
    /// Arrange-mode offsets nudge the rendered model on top of the baseline
    /// position computed at draw time.
    pub hover_title_plaque: Placement,
    /// Description plaque anchored *below* the currently hovered/focused item.
    /// Mirrors `hover_title_plaque` and carries the item's description text so
    /// players can read what a relic/ribbon/talisman/pack actually does.
    pub hover_desc_plaque: Placement,
    /// Combined title + sell price + description plaque for hovered
    /// *player-owned* items on the bottom shelf trays. Positioned
    /// independently from the for-sale hover plaques because owned items
    /// live near the bottom of the screen where "description below item"
    /// would fall off-screen.
    pub hover_owned_plaque: Placement,

    /// Per-pendant offset applied to the player's owned talismans as they
    /// sit on the `talisman_tray`. Kept separate from the `for_sale.talismans`
    /// stall so arrange-mode rotations on the shop wall don't leak into the
    /// owned-inventory pendants.
    pub owned_talismans: Placement,

    // ── Camera (non-spatial; multipliers on window-derived defaults) ───────
    pub camera_eye_y_frac: f32,
    pub camera_eye_z_frac: f32,
    pub camera_target_y_frac: f32,
    pub camera_target_z_frac: f32,

    // ── Pack-opening celebration ───────────────────────────────────────────
    pub celeb_pack_closeup: Placement,
    pub celeb_pack_reveal: Placement,
    pub celeb_zodiac: Placement,
}

/// Conversion constant: a value previously expressed as a fraction of the
/// window height, re-expressed as `lift_mm` so the unit system is uniform.
///
/// Derived from the layout invariants so it automatically tracks changes to
/// `HAND_X_PAD_RATIO`, `HAND_SIZE`, or `TILE_WIDTH_MM`. On a square canonical
/// window the conversion is exactly `h_frac * TILE_WIDTH_MM / HAND_SLOT_W_RATIO`:
/// starting from `layout.mm(lift_mm) = h * h_frac` (the old semantics) and
/// `layout.mm(n) = n * tile_w / TILE_WIDTH_MM`, and `tile_w = w * HAND_SLOT_W_RATIO`,
/// we get `lift_mm = h_frac * h * TILE_WIDTH_MM / (w * HAND_SLOT_W_RATIO)`, which
/// collapses to the constant below at `h == w`.
///
/// Non-square windows trade a small positional drift for uniform semantics;
/// that was the explicit design decision during the consistency unification.
pub const HFRAC_TO_MM: f32 =
    crate::ui::layout::TILE_WIDTH_MM / crate::ui::layout::HAND_SLOT_W_RATIO;

/// Canonical window size used to convert legacy raw-pixel / world-unit
/// defaults into the unified fraction-and-mm system. Chosen to match the
/// typical default-sized window under which the original pixel values were
/// measured.
pub const CANONICAL_WINDOW_W: f32 = 1200.0;

impl Default for ShopPositions {
    fn default() -> Self {
        Self {
            // For-sale columns (was col_nx=[0.22,0.40,0.58,0.76] × stalls).
            relics: Placement::at(0.22, 0.31, 39.431_37),
            packs: Placement {
                nx: 0.413_888_9,
                ny: 0.478_250_95,
                lift_mm: 58.172_863,
                rx_deg: -16.0,
                ry_deg: 0.0,
                rz_deg: 1.0,
            },
            talismans: Placement::at(0.58, 0.333_764_26, 39.431_37),
            ribbons: Placement::at(0.76, 0.43, 23.431_372),
            relic_spread_nx: 0.075,
            ribbon_spread_nx: 0.050,
            talisman_spread_nx: 0.055,

            counter: Placement::at(0.5, 0.35, 0.0),
            // Shelf row (was shelf_ny=0.84 shared).
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
            owned_talismans: Placement::at(0.0, 0.0, 0.0),

            camera_eye_y_frac: 0.72,
            camera_eye_z_frac: 0.34,
            camera_target_y_frac: 0.18,
            camera_target_z_frac: 0.10,

            // Celebrations (was celeb_*_lift_h_frac).
            celeb_pack_closeup: Placement::at(0.5, 0.45, -2.573_530_7),
            celeb_pack_reveal: Placement::at(0.5, 0.55, 36.887_23),
            celeb_zodiac: Placement::at(0.5, 0.35, 257.352_94),
        }
    }
}

/// Canonical hierarchy for the Shop scene. Group names like `"shop.for_sale"`
/// nudge every child column at once; leaf names are the stable dotted paths.
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

/// Typed field identifier for the shop scene. One variant per [`Placement`]
/// on [`ShopPositions`] — used as the single source of truth linking:
///
/// - the canonical dotted name in [`SHOP_HIERARCHY`]
/// - every alias accepted by arrange-mode (from click-pickables, etc.)
/// - the `&mut Placement` accessor used by `apply_arrange`
///
/// Adding a new placement means adding a variant here, one arm in
/// [`ShopPositions::field_mut`], one arm in [`lookup_shop_field`] /
/// [`shop_field_path`], and one leaf in [`SHOP_HIERARCHY`]. The coverage
/// tests guarantee all four agree.
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
    CelebPackCloseup,
    CelebPackReveal,
    CelebZodiac,
}

/// Map a canonical dotted path → the [`ShopField`] it refers to. The
/// renderer emits these exact strings via `arrange_name` on `Object3d` and
/// `apply_arrange_override` / `last_debug_pickables` in shared pipelines,
/// so there's no alias layer — one path per field.
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
        "shop.celebrations.pack_closeup" => ShopField::CelebPackCloseup,
        "shop.celebrations.pack_reveal" => ShopField::CelebPackReveal,
        "shop.celebrations.zodiac" => ShopField::CelebZodiac,
        _ => return None,
    })
}

/// Canonical path for a given [`ShopField`]. Used by coverage tests to
/// assert the hierarchy, `lookup_shop_field`, and render-side pickable
/// strings all agree.
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
        ShopField::CelebPackCloseup => "shop.celebrations.pack_closeup",
        ShopField::CelebPackReveal => "shop.celebrations.pack_reveal",
        ShopField::CelebZodiac => "shop.celebrations.zodiac",
    }
}

impl ShopField {
    /// Every [`ShopField`] variant, in declaration order. Used to iterate
    /// all fields for validation / coverage checks.
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
        ShopField::OwnedTalismans,
        ShopField::CelebPackCloseup,
        ShopField::CelebPackReveal,
        ShopField::CelebZodiac,
    ];
}

impl ShopPositions {
    /// Map a [`ShopField`] to its backing placement. The only place where
    /// field ids meet struct fields.
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
            ShopField::CelebPackCloseup => &mut self.celeb_pack_closeup,
            ShopField::CelebPackReveal => &mut self.celeb_pack_reveal,
            ShopField::CelebZodiac => &mut self.celeb_zodiac,
        }
    }

    /// Immutable counterpart to [`Self::field_mut`].
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

/// Load [`ShopPositions`] from JSON, falling back to [`Default`] if missing
/// or malformed. Non-finite placements (NaN / Infinity from hand-edited JSON)
/// are replaced with the default value for that field to prevent silent
/// render corruption.
pub fn load_shop_positions() -> ShopPositions {
    let path = layouts_dir().join("shop.json");
    let mut loaded: ShopPositions = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    sanitize_shop_positions(&mut loaded);
    loaded
}

/// Replace any non-finite [`Placement`] fields in `p` with the values from
/// [`ShopPositions::default`], logging a warning for each field corrected.
pub fn sanitize_shop_positions(p: &mut ShopPositions) {
    let mut defaults = ShopPositions::default();
    for &field in ShopField::ALL {
        if !p.field_mut(field).is_finite() {
            log::warn!(
                "[Layout] shop placement {:?} had non-finite values, restoring defaults",
                field
            );
            *p.field_mut(field) = *defaults.field_mut(field);
        }
    }
}

/// Write [`ShopPositions`] to JSON.
pub fn save_shop_positions(pos: &ShopPositions) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(pos)?;
    let path = layouts_dir().join("shop.json");
    fs::write(&path, json)?;
    log::info!("[Layout] Saved shop positions → {}", path.display());
    Ok(())
}

// ── GameplayPositions ─────────────────────────────────────────────────────────

/// Serializable position data for the gameplay scene.
///
/// Every placement uses the same units: `nx`/`ny` as window fractions,
/// `lift_mm` as physical millimeters, rotation in degrees.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GameplayPositions {
    // ── Relic tray (top of screen, horizontal) ───────────────────────────────
    /// `nx` = tray center x (window fraction). `ny` = tray center y
    /// (window fraction). `lift_mm` is the lift of the badges above the
    /// table. The relics render as face-on enamel medallions in a
    /// horizontal row centered on `(nx, ny)`.
    pub relic_col: Placement,
    /// Left edge clamp for the tray (window fraction). The row is centered
    /// on `relic_col.nx` but will not extend past this fraction.
    pub relic_col_top_ny: f32,
    /// Right edge clamp for the tray (window fraction).
    pub relic_col_bottom_ny: f32,
    /// Badge face size and horizontal stride between badge centers (mm).
    pub relic_cell_height_mm: f32,

    // ── Score plaque (score-panel-anchored) ──────────────────────────────────
    /// `ny` is a score-panel-relative offset (fraction of window height,
    /// subtracted from the score-panel center). `lift_mm` is the lift.
    pub plaque: Placement,

    // ── Counter fans (action-bar-anchored) ───────────────────────────────────
    /// Draws counter fan — bone tally sticks standing in front of the bronze
    /// mirror. `nx`/`ny` are window-fraction offsets from the mirror's world
    /// pos; `lift_mm` stacks on top of the mirror's lift; `ry_deg` yaws the
    /// fan plane about world up so the sticks face the camera.
    pub counter_draws_fan: Placement,
    /// Discards counter fan — bone tally sticks in front of the discard
    /// river. Same offset convention as `counter_draws_fan`.
    pub counter_discards_fan: Placement,

    // ── Boss-rule ofuda (score-panel-anchored) ───────────────────────────────
    /// `nx`/`ny` are window fractions for the ofuda card that appears on
    /// boss blinds. Only drawn when a rule text is present.
    pub ofuda: Placement,

    // ── Coin pile (score-panel-anchored) ─────────────────────────────────────
    /// `nx` is absolute window fraction (may exceed 1.0). `ny` is a
    /// score-panel-relative offset.
    pub coin_pile: Placement,

    // ── Dora indicator ───────────────────────────────────────────────────────
    pub dora: Placement,

    // ── Talisman dish (consumables strip, top-right) ─────────────────────────
    /// `nx`/`ny` are pixel offsets (stored as window-fractions) from the
    /// consumables-strip center computed in `gameplay.rs`. `lift_mm` lifts
    /// the brass tray; rotations compose with the tray's built-in flat-lay.
    pub talisman_dish: Placement,

    // ── Discard bowl / Bronze mirror (action-bar-anchored) ───────────────────
    /// `nx`/`ny` are window-fraction offsets from the action-bar anchor.
    pub bowl: Placement,
    pub mirror: Placement,

    // ── Wood tablets (action-bar-anchored offsets) ───────────────────────────
    /// Window-fraction offsets from each tablet's layout-assigned anchor —
    /// same pattern as bowl/mirror. `tablet_sort_suit` = slot 0,
    /// `tablet_sort_rank` = slot 1, `tablet_cash_in` = slot 4 (the Cash-in
    /// tablet next to play/discard), `tablet_journal` = the Journal book
    /// tablet in the sort row.
    pub tablet_sort_suit: Placement,
    pub tablet_sort_rank: Placement,
    pub tablet_cash_in: Placement,
    pub tablet_journal: Placement,

    // ── Candles ──────────────────────────────────────────────────────────────
    pub candle_back_z_push_candle_w_frac: f32,
    pub candle_bottom_z_back_candle_h_frac: f32,

    // ── Hand strip (hand-slot-anchored offset) ───────────────────────────────
    /// `nx`/`ny` are window-fraction offsets from the Cassowary-derived
    /// hand-slot center; rotations compose with the hand tile's built-in pitch.
    pub hand_strip: Placement,

    // ── Yaku tablet row (hand-slot-anchored offset) ──────────────────────────
    pub yaku_tablet: Placement,

    // ── Camera (non-spatial multipliers) ─────────────────────────────────────
    pub camera_eye_y_mul: f32,
    pub camera_eye_z_mul: f32,
    pub camera_target_y_mul: f32,
    pub camera_target_z_mul: f32,
    pub camera_fovy_deg: f32,
}

impl Default for GameplayPositions {
    fn default() -> Self {
        Self {
            relic_col: Placement::at(0.5, 0.045, 2.144_608),
            relic_col_top_ny: 0.22,
            relic_col_bottom_ny: 0.78,
            relic_cell_height_mm: 42.0,

            plaque: Placement {
                nx: 0.0,
                ny: 0.18,
                lift_mm: 139.161_16,
                rx_deg: 0.0,
                ry_deg: 0.0,
                rz_deg: 0.0,
            },

            counter_draws_fan: Placement {
                nx: -0.042_534_72,
                ny: -0.036_121_674,
                lift_mm: 0.744_655_5,
                rx_deg: 0.0,
                ry_deg: 0.0,
                rz_deg: -30.0,
            },
            counter_discards_fan: Placement {
                nx: 0.016_493_056,
                ny: -0.102_661_6,
                lift_mm: -25.318_287,
                rx_deg: 0.0,
                ry_deg: 0.0,
                rz_deg: 45.0,
            },

            ofuda: Placement {
                nx: 0.002_893_518_4,
                ny: 0.0,
                lift_mm: -22.875_816,
                rx_deg: -69.0,
                ry_deg: 0.0,
                rz_deg: 0.0,
            },

            coin_pile: Placement::at(1.179_050_6, 0.13, 2.144_608),

            dora: Placement::at(0.94, 0.22, 2.144_608),

            talisman_dish: Placement::at(0.0, 0.0, 2.144_608),

            bowl: Placement {
                nx: -0.002_581_019,
                ny: 0.027_867_55,
                lift_mm: 2.144_608,
                rx_deg: 0.0,
                ry_deg: 0.0,
                rz_deg: -315.0,
            },
            mirror: Placement {
                nx: -0.006_076_388_5,
                ny: 0.023_764_258,
                lift_mm: 1.995_676_9,
                rx_deg: -159.0,
                ry_deg: 75.0,
                rz_deg: -78.0,
            },

            tablet_sort_suit: Placement::at(0.0, -0.061_787_07, 2.144_608),
            tablet_sort_rank: Placement::at(0.0, -0.063_212_92, 2.144_608),
            tablet_cash_in: Placement::at(0.0, 0.0, 2.144_608),
            tablet_journal: Placement::at(0.0, -0.064_638_78, 2.144_608),

            candle_back_z_push_candle_w_frac: 1.0,
            candle_bottom_z_back_candle_h_frac: 0.55,

            hand_strip: Placement {
                nx: 0.0,
                ny: 0.221_292_81,
                lift_mm: 36.458_336,
                rx_deg: -12.0,
                ry_deg: 0.0,
                rz_deg: 0.0,
            },

            yaku_tablet: Placement {
                nx: 0.0,
                ny: 0.155_893_97,
                lift_mm: 19.301_472,
                rx_deg: -20.0,
                ry_deg: 0.0,
                rz_deg: 0.0,
            },

            camera_eye_y_mul: 1.0,
            camera_eye_z_mul: 1.0,
            camera_target_y_mul: 1.0,
            camera_target_z_mul: 1.0,
            camera_fovy_deg: 55.0,
        }
    }
}

/// Canonical hierarchy for the Gameplay scene.
pub const GAMEPLAY_HIERARCHY: &[Node] = &[Node::Group {
    name: "gameplay",
    label: "Gameplay",
    children: &[
        Node::Group {
            name: "gameplay.hand",
            label: "Hand area",
            children: &[
                Node::Leaf {
                    name: "gameplay.hand.strip",
                    label: "Hand strip",
                },
                Node::Leaf {
                    name: "gameplay.hand.yaku_tablet",
                    label: "Yaku tablet row",
                },
            ],
        },
        Node::Group {
            name: "gameplay.score_panel",
            label: "Score panel",
            children: &[
                Node::Leaf {
                    name: "gameplay.score_panel.plaque",
                    label: "Blind plaque",
                },
                Node::Leaf {
                    name: "gameplay.score_panel.ofuda",
                    label: "Boss-rule ofuda",
                },
                Node::Leaf {
                    name: "gameplay.score_panel.coin_pile",
                    label: "Coin pile",
                },
            ],
        },
        Node::Group {
            name: "gameplay.action_bar",
            label: "Action bar",
            children: &[
                Node::Leaf {
                    name: "gameplay.action_bar.bowl",
                    label: "Discard bowl",
                },
                Node::Leaf {
                    name: "gameplay.action_bar.mirror",
                    label: "Bronze mirror",
                },
                Node::Leaf {
                    name: "gameplay.action_bar.tablet_sort_suit",
                    label: "Tablet — Sort by suit",
                },
                Node::Leaf {
                    name: "gameplay.action_bar.tablet_sort_rank",
                    label: "Tablet — Sort by rank",
                },
                Node::Leaf {
                    name: "gameplay.action_bar.tablet_cash_in",
                    label: "Tablet — Cash in",
                },
                Node::Leaf {
                    name: "gameplay.action_bar.tablet_journal",
                    label: "Tablet — Journal",
                },
            ],
        },
        Node::Group {
            name: "gameplay.counter",
            label: "Counter fans",
            children: &[
                Node::Leaf {
                    name: "gameplay.counter.draws_fan",
                    label: "Draws fan",
                },
                Node::Leaf {
                    name: "gameplay.counter.discards_fan",
                    label: "Discards fan",
                },
            ],
        },
        Node::Leaf {
            name: "gameplay.relic_col",
            label: "Relic sidebar",
        },
        Node::Leaf {
            name: "gameplay.dora",
            label: "Dora indicator",
        },
        Node::Leaf {
            name: "gameplay.talisman_dish",
            label: "Talisman dish",
        },
    ],
}];

/// Typed field identifier for the gameplay scene. Same pattern as
/// [`ShopField`] — one variant per [`Placement`] on [`GameplayPositions`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameplayField {
    RelicCol,
    Plaque,
    CounterDrawsFan,
    CounterDiscardsFan,
    Ofuda,
    CoinPile,
    Dora,
    Bowl,
    Mirror,
    HandStrip,
    YakuTablet,
    TabletSortSuit,
    TabletSortRank,
    TabletCashIn,
    TabletJournal,
    TalismanDish,
}

pub fn lookup_gameplay_field(name: &str) -> Option<GameplayField> {
    Some(match name {
        "gameplay.relic_col" => GameplayField::RelicCol,
        "gameplay.score_panel.plaque" => GameplayField::Plaque,
        "gameplay.counter.draws_fan" => GameplayField::CounterDrawsFan,
        "gameplay.counter.discards_fan" => GameplayField::CounterDiscardsFan,
        "gameplay.score_panel.ofuda" => GameplayField::Ofuda,
        "gameplay.score_panel.coin_pile" => GameplayField::CoinPile,
        "gameplay.dora" => GameplayField::Dora,
        "gameplay.action_bar.bowl" => GameplayField::Bowl,
        "gameplay.action_bar.mirror" => GameplayField::Mirror,
        "gameplay.action_bar.tablet_sort_suit" => GameplayField::TabletSortSuit,
        "gameplay.action_bar.tablet_sort_rank" => GameplayField::TabletSortRank,
        "gameplay.action_bar.tablet_cash_in" => GameplayField::TabletCashIn,
        "gameplay.action_bar.tablet_journal" => GameplayField::TabletJournal,
        "gameplay.hand.strip" => GameplayField::HandStrip,
        "gameplay.hand.yaku_tablet" => GameplayField::YakuTablet,
        "gameplay.talisman_dish" => GameplayField::TalismanDish,
        _ => return None,
    })
}

#[cfg(test)]
pub fn gameplay_field_path(field: GameplayField) -> &'static str {
    match field {
        GameplayField::RelicCol => "gameplay.relic_col",
        GameplayField::Plaque => "gameplay.score_panel.plaque",
        GameplayField::CounterDrawsFan => "gameplay.counter.draws_fan",
        GameplayField::CounterDiscardsFan => "gameplay.counter.discards_fan",
        GameplayField::Ofuda => "gameplay.score_panel.ofuda",
        GameplayField::CoinPile => "gameplay.score_panel.coin_pile",
        GameplayField::Dora => "gameplay.dora",
        GameplayField::Bowl => "gameplay.action_bar.bowl",
        GameplayField::Mirror => "gameplay.action_bar.mirror",
        GameplayField::TabletSortSuit => "gameplay.action_bar.tablet_sort_suit",
        GameplayField::TabletSortRank => "gameplay.action_bar.tablet_sort_rank",
        GameplayField::TabletCashIn => "gameplay.action_bar.tablet_cash_in",
        GameplayField::TabletJournal => "gameplay.action_bar.tablet_journal",
        GameplayField::HandStrip => "gameplay.hand.strip",
        GameplayField::YakuTablet => "gameplay.hand.yaku_tablet",
        GameplayField::TalismanDish => "gameplay.talisman_dish",
    }
}

impl GameplayField {
    pub const ALL: &'static [GameplayField] = &[
        GameplayField::RelicCol,
        GameplayField::Plaque,
        GameplayField::CounterDrawsFan,
        GameplayField::CounterDiscardsFan,
        GameplayField::Ofuda,
        GameplayField::CoinPile,
        GameplayField::Dora,
        GameplayField::Bowl,
        GameplayField::Mirror,
        GameplayField::HandStrip,
        GameplayField::YakuTablet,
        GameplayField::TabletSortSuit,
        GameplayField::TabletSortRank,
        GameplayField::TabletCashIn,
        GameplayField::TabletJournal,
        GameplayField::TalismanDish,
    ];
}

impl GameplayPositions {
    pub fn field_mut(&mut self, field: GameplayField) -> &mut Placement {
        match field {
            GameplayField::RelicCol => &mut self.relic_col,
            GameplayField::Plaque => &mut self.plaque,
            GameplayField::CounterDrawsFan => &mut self.counter_draws_fan,
            GameplayField::CounterDiscardsFan => &mut self.counter_discards_fan,
            GameplayField::Ofuda => &mut self.ofuda,
            GameplayField::CoinPile => &mut self.coin_pile,
            GameplayField::Dora => &mut self.dora,
            GameplayField::Bowl => &mut self.bowl,
            GameplayField::Mirror => &mut self.mirror,
            GameplayField::HandStrip => &mut self.hand_strip,
            GameplayField::YakuTablet => &mut self.yaku_tablet,
            GameplayField::TabletSortSuit => &mut self.tablet_sort_suit,
            GameplayField::TabletSortRank => &mut self.tablet_sort_rank,
            GameplayField::TabletCashIn => &mut self.tablet_cash_in,
            GameplayField::TabletJournal => &mut self.tablet_journal,
            GameplayField::TalismanDish => &mut self.talisman_dish,
        }
    }

    pub fn field_ref(&self, field: GameplayField) -> &Placement {
        match field {
            GameplayField::RelicCol => &self.relic_col,
            GameplayField::Plaque => &self.plaque,
            GameplayField::CounterDrawsFan => &self.counter_draws_fan,
            GameplayField::CounterDiscardsFan => &self.counter_discards_fan,
            GameplayField::Ofuda => &self.ofuda,
            GameplayField::CoinPile => &self.coin_pile,
            GameplayField::Dora => &self.dora,
            GameplayField::Bowl => &self.bowl,
            GameplayField::Mirror => &self.mirror,
            GameplayField::HandStrip => &self.hand_strip,
            GameplayField::YakuTablet => &self.yaku_tablet,
            GameplayField::TabletSortSuit => &self.tablet_sort_suit,
            GameplayField::TabletSortRank => &self.tablet_sort_rank,
            GameplayField::TabletCashIn => &self.tablet_cash_in,
            GameplayField::TabletJournal => &self.tablet_journal,
            GameplayField::TalismanDish => &self.talisman_dish,
        }
    }
}

impl ArrangeTarget for GameplayPositions {
    fn placement_mut(&mut self, name: &str) -> Option<&mut Placement> {
        lookup_gameplay_field(name).map(|f| self.field_mut(f))
    }

    fn placement(&self, name: &str) -> Option<&Placement> {
        lookup_gameplay_field(name).map(|f| self.field_ref(f))
    }

    fn hierarchy(&self) -> &'static [Node] {
        GAMEPLAY_HIERARCHY
    }
}

/// Load [`GameplayPositions`] from JSON, falling back to [`Default`].
/// Non-finite placement fields are replaced with defaults.
pub fn load_gameplay_positions() -> GameplayPositions {
    let path = layouts_dir().join("gameplay.json");
    let mut loaded: GameplayPositions = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    sanitize_gameplay_positions(&mut loaded);
    loaded
}

/// Replace any non-finite [`Placement`] fields in `p` with the values from
/// [`GameplayPositions::default`], logging a warning for each.
pub fn sanitize_gameplay_positions(p: &mut GameplayPositions) {
    let mut defaults = GameplayPositions::default();
    for &field in GameplayField::ALL {
        if !p.field_mut(field).is_finite() {
            log::warn!(
                "[Layout] gameplay placement {:?} had non-finite values, restoring defaults",
                field
            );
            *p.field_mut(field) = *defaults.field_mut(field);
        }
    }
}

/// Write [`GameplayPositions`] to JSON.
pub fn save_gameplay_positions(pos: &GameplayPositions) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(pos)?;
    let path = layouts_dir().join("gameplay.json");
    fs::write(&path, json)?;
    log::info!("[Layout] Saved gameplay positions → {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::placement::apply_arrange;

    const EPS: f32 = 1e-5;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn shop_positions_serde_roundtrip() {
        let orig = ShopPositions::default();
        let json = serde_json::to_string(&orig).unwrap();
        let restored: ShopPositions = serde_json::from_str(&json).unwrap();
        assert!(approx(restored.counter.nx, orig.counter.nx));
        assert!(approx(restored.relics.nx, orig.relics.nx));
        assert!(approx(restored.lamp.lift_mm, orig.lamp.lift_mm));
    }

    #[test]
    fn gameplay_positions_serde_roundtrip() {
        let orig = GameplayPositions::default();
        let json = serde_json::to_string(&orig).unwrap();
        let restored: GameplayPositions = serde_json::from_str(&json).unwrap();
        assert!(approx(restored.relic_col.nx, orig.relic_col.nx));
        assert!(approx(restored.dora.nx, orig.dora.nx));
        assert!(approx(restored.hand_strip.ny, orig.hand_strip.ny));
        assert!(approx(restored.plaque.lift_mm, orig.plaque.lift_mm));
    }

    #[test]
    fn shop_positions_sparse_json_uses_defaults() {
        let json = r#"{ "counter": { "nx": 0.42 } }"#;
        let p: ShopPositions = serde_json::from_str(json).unwrap();
        assert!(approx(p.counter.nx, 0.42));
        let default = ShopPositions::default();
        assert!(approx(p.relics.nx, default.relics.nx));
        assert!(approx(p.lamp.lift_mm, default.lamp.lift_mm));
    }

    #[test]
    fn arrange_counter_via_generic_handler() {
        let mut p = ShopPositions::default();
        let before = p.counter.nx;
        let ok = apply_arrange(&mut p, "shop.counter", 0.01, 0.0, 0.0, 0.0, 0.0, 0.0);
        assert!(ok);
        assert!(approx(p.counter.nx, before + 0.01));
    }

    #[test]
    fn arrange_hand_strip_accumulates_rotation() {
        let mut p = GameplayPositions::default();
        let before_rx = p.hand_strip.rx_deg;
        let ok = apply_arrange(&mut p, "gameplay.hand.strip", 0.0, 0.0, 0.0, 2.5, 0.0, 0.0);
        assert!(ok);
        assert!(approx(p.hand_strip.rx_deg, before_rx + 2.5));
    }

    #[test]
    fn arrange_bowl_is_a_regular_placement() {
        // After unification, bowl is a first-class Placement — no special nudge needed.
        let mut p = GameplayPositions::default();
        let before_nx = p.bowl.nx;
        let ok = apply_arrange(&mut p, "gameplay.action_bar.bowl", 0.01, 0.0, 0.0, 0.0, 0.0, 0.0);
        assert!(ok);
        assert!(approx(p.bowl.nx, before_nx + 0.01));
    }

    #[test]
    fn arrange_shop_group_moves_every_child_column() {
        // Selecting the "shop.for_sale" group should nudge relics, packs,
        // talismans, and ribbons all at once.
        let mut p = ShopPositions::default();
        let before_relics = p.relics.nx;
        let before_packs = p.packs.nx;
        let before_talismans = p.talismans.nx;
        let before_ribbons = p.ribbons.nx;
        let ok = apply_arrange(&mut p, "shop.for_sale", 0.01, 0.0, 0.0, 0.0, 0.0, 0.0);
        assert!(ok);
        assert!(approx(p.relics.nx, before_relics + 0.01));
        assert!(approx(p.packs.nx, before_packs + 0.01));
        assert!(approx(p.talismans.nx, before_talismans + 0.01));
        assert!(approx(p.ribbons.nx, before_ribbons + 0.01));
    }

    #[test]
    fn arrange_shop_dotted_path_matches_leaf() {
        let mut p = ShopPositions::default();
        let before = p.counter.nx;
        let ok = apply_arrange(&mut p, "shop.counter", 0.01, 0.0, 0.0, 0.0, 0.0, 0.0);
        assert!(ok);
        assert!(approx(p.counter.nx, before + 0.01));
    }

    #[test]
    fn arrange_gameplay_group_moves_hand_area() {
        let mut p = GameplayPositions::default();
        let before_strip_rx = p.hand_strip.rx_deg;
        let before_yaku_rx = p.yaku_tablet.rx_deg;
        let ok = apply_arrange(&mut p, "gameplay.hand", 0.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        assert!(ok);
        assert!(approx(p.hand_strip.rx_deg, before_strip_rx + 1.0));
        assert!(approx(p.yaku_tablet.rx_deg, before_yaku_rx + 1.0));
    }

    #[test]
    fn arrange_plaque_is_a_regular_placement() {
        let mut p = GameplayPositions::default();
        let before_ny = p.plaque.ny;
        let ok = apply_arrange(&mut p, "gameplay.score_panel.plaque", 0.0, 0.01, 0.0, 0.0, 0.0, 0.0);
        assert!(ok);
        assert!(approx(p.plaque.ny, before_ny + 0.01));
    }

    // ── Coverage: hierarchy ↔ aliases ↔ field_mut must all agree ────────────

    /// Every leaf name declared in `SHOP_HIERARCHY` must resolve to a field
    /// via `placement_mut`. Catches typos or a newly added leaf without a
    /// corresponding alias entry.
    #[test]
    fn shop_hierarchy_leaves_all_resolve() {
        use crate::ui::placement::all_leaf_names;
        let mut p = ShopPositions::default();
        for leaf in all_leaf_names(SHOP_HIERARCHY) {
            assert!(
                p.placement_mut(leaf).is_some(),
                "SHOP_HIERARCHY leaf {:?} has no placement_mut arm",
                leaf,
            );
        }
    }

    /// Every `ShopField` variant must round-trip through `shop_field_path`
    /// → `lookup_shop_field` as the same variant.
    #[test]
    fn shop_field_path_roundtrip() {
        for &field in ShopField::ALL {
            let path = shop_field_path(field);
            assert_eq!(
                lookup_shop_field(path),
                Some(field),
                "path {:?} did not round-trip to ShopField::{:?}",
                path,
                field,
            );
        }
    }

    /// Every canonical `ShopField` path must appear as a leaf in
    /// `SHOP_HIERARCHY`. Catches typos that would make a field
    /// un-browseable via Tab cycling.
    #[test]
    fn shop_field_paths_all_in_hierarchy() {
        use crate::ui::placement::all_leaf_names;
        let leaves: Vec<&'static str> = all_leaf_names(SHOP_HIERARCHY);
        for &field in ShopField::ALL {
            let path = shop_field_path(field);
            assert!(
                leaves.contains(&path),
                "shop_field_path(ShopField::{:?}) = {:?} not found in SHOP_HIERARCHY",
                field,
                path,
            );
        }
    }

    #[test]
    fn gameplay_hierarchy_leaves_all_resolve() {
        use crate::ui::placement::all_leaf_names;
        let mut p = GameplayPositions::default();
        for leaf in all_leaf_names(GAMEPLAY_HIERARCHY) {
            assert!(
                p.placement_mut(leaf).is_some(),
                "GAMEPLAY_HIERARCHY leaf {:?} has no placement_mut arm",
                leaf,
            );
        }
    }

    #[test]
    fn gameplay_field_path_roundtrip() {
        for &field in GameplayField::ALL {
            let path = gameplay_field_path(field);
            assert_eq!(
                lookup_gameplay_field(path),
                Some(field),
                "path {:?} did not round-trip to GameplayField::{:?}",
                path,
                field,
            );
        }
    }

    #[test]
    fn gameplay_field_paths_all_in_hierarchy() {
        use crate::ui::placement::all_leaf_names;
        let leaves: Vec<&'static str> = all_leaf_names(GAMEPLAY_HIERARCHY);
        for &field in GameplayField::ALL {
            let path = gameplay_field_path(field);
            assert!(
                leaves.contains(&path),
                "gameplay_field_path(GameplayField::{:?}) = {:?} not found in GAMEPLAY_HIERARCHY",
                field,
                path,
            );
        }
    }

    // ── Validation: non-finite JSON gets sanitized ──────────────────────────

    #[test]
    fn sanitize_shop_restores_non_finite_fields() {
        let mut p = ShopPositions::default();
        let default_counter_nx = p.counter.nx;
        p.counter.nx = f32::NAN;
        p.lamp.lift_mm = f32::INFINITY;
        sanitize_shop_positions(&mut p);
        assert!(approx(p.counter.nx, default_counter_nx));
        assert!(p.lamp.lift_mm.is_finite());
    }

    #[test]
    fn sanitize_gameplay_restores_non_finite_fields() {
        let mut p = GameplayPositions::default();
        let default_dora_ny = p.dora.ny;
        p.dora.ny = f32::NAN;
        sanitize_gameplay_positions(&mut p);
        assert!(approx(p.dora.ny, default_dora_ny));
    }

    #[test]
    fn sanitize_leaves_valid_placements_alone() {
        let mut p = ShopPositions::default();
        p.counter.nx = 0.42;
        sanitize_shop_positions(&mut p);
        assert!(approx(p.counter.nx, 0.42));
    }

    // ── HFRAC_TO_MM is the derived constant from layout invariants ──────────

    #[test]
    fn hfrac_to_mm_matches_layout_constants() {
        use crate::ui::layout::{HAND_SLOT_W_RATIO, TILE_WIDTH_MM};
        let expected = TILE_WIDTH_MM / HAND_SLOT_W_RATIO;
        assert!(
            (HFRAC_TO_MM - expected).abs() < 1e-3,
            "HFRAC_TO_MM ({}) drifted from TILE_WIDTH_MM / HAND_SLOT_W_RATIO ({})",
            HFRAC_TO_MM,
            expected,
        );
    }
}
