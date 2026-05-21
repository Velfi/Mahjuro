use crate::ui::placement::Placement;

#[derive(Clone, Debug)]
pub struct TileSelectPositions {
    /// Top-left of the title / material copy column (normalized window fractions).
    pub left_panel: Placement,
    /// Top-left of the bottom hint line (`Esc to go back`, etc.).
    pub bottom_hint: Placement,
    /// Top-left of the focusable button column (Play, Back, stake row).
    pub button_menu: Placement,
    /// Top-left corner of the tile preview grid.
    pub preview_corner_tl: Placement,
    /// Bottom-right corner of the tile preview grid (nx/ny should stay below/right of TL).
    pub preview_corner_br: Placement,
    /// Aim point for the warm key light (nx/ny only; height uses a fixed screen fraction).
    pub key_light: Placement,
}

impl Default for TileSelectPositions {
    fn default() -> Self {
        Self {
            // Matches legacy `panel_w * 0.05` and `cursor_y = h * 0.10` (non-tutorial base).
            left_panel: Placement::at(0.019, 0.10, 0.0),
            // Approx. `h - hint_h - 12px` at 1080p-scale caption sizing.
            bottom_hint: Placement::at(0.019, 0.971, 0.0),
            // Centered in the left panel column at a typical desktop aspect.
            button_menu: Placement::at(0.122, 0.60, 0.0),
            // Former `TILE_PREVIEW_GRID_*` fractions (reference ~1080p height).
            preview_corner_tl: Placement::at(0.44, 0.08, 0.0),
            preview_corner_br: Placement::at(0.95, 0.84, 0.0),
            key_light: Placement::at(0.70, 0.26, 0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileSelectField {
    LeftPanel,
    BottomHint,
    ButtonMenu,
    PreviewCornerTl,
    PreviewCornerBr,
    KeyLight,
}

pub fn tile_select_field_path(field: TileSelectField) -> &'static str {
    match field {
        TileSelectField::LeftPanel => "tile_select.panel.content",
        TileSelectField::BottomHint => "tile_select.panel.bottom_hint",
        TileSelectField::ButtonMenu => "tile_select.button_menu",
        TileSelectField::PreviewCornerTl => "tile_select.preview.corner_tl",
        TileSelectField::PreviewCornerBr => "tile_select.preview.corner_br",
        TileSelectField::KeyLight => "tile_select.key_light",
    }
}

impl TileSelectField {
    pub const ALL: &'static [TileSelectField] = &[
        TileSelectField::LeftPanel,
        TileSelectField::BottomHint,
        TileSelectField::ButtonMenu,
        TileSelectField::PreviewCornerTl,
        TileSelectField::PreviewCornerBr,
        TileSelectField::KeyLight,
    ];
}

impl TileSelectPositions {
    pub fn field_ref(&self, field: TileSelectField) -> &Placement {
        match field {
            TileSelectField::LeftPanel => &self.left_panel,
            TileSelectField::BottomHint => &self.bottom_hint,
            TileSelectField::ButtonMenu => &self.button_menu,
            TileSelectField::PreviewCornerTl => &self.preview_corner_tl,
            TileSelectField::PreviewCornerBr => &self.preview_corner_br,
            TileSelectField::KeyLight => &self.key_light,
        }
    }
}
