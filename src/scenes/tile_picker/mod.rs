//! Shared tile grid layouts for stress lab (accordion drawers) and stairway decimation (scroll).

use crate::core::tile::{Suit, Tile};
use crate::render::draw_cmd::{CameraParams, ShowcaseTilePlacement};
use crate::render::theme::{metrics, typography};
use crate::ui::controller_hints::screen_footer_reserve;
use crate::ui::widget::PLAIN_TEXT_LINE_STEP_MUL;
use crate::ui::widget_tree::{FlatItem, FocusId};
use super::header_chrome::HeaderChromeMetrics;

pub const TILE_ROTATION: [f32; 3] = [0.0, 0.0, std::f32::consts::PI];
const GRID_GAP_FRAC: f32 = 0.08;

#[derive(Clone, Copy, Debug, PartialEq)]
struct TileGridPlan {
    cols: usize,
    rows: usize,
    size_px: f32,
    gap: f32,
    face_aspect: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuitDrawer {
    Manzu,
    Souzu,
    Pinzu,
    Honors,
    Flowers,
}

impl SuitDrawer {
    pub const ALL: [Self; 5] = [
        Self::Manzu,
        Self::Souzu,
        Self::Pinzu,
        Self::Honors,
        Self::Flowers,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Manzu => "Manzu",
            Self::Souzu => "Souzu",
            Self::Pinzu => "Pinzu",
            Self::Honors => "Honors",
            Self::Flowers => "Flowers",
        }
    }

    pub fn accent(self) -> [f32; 4] {
        match self {
            Self::Manzu => Suit::Manzu.keyword_color(),
            Self::Souzu => Suit::Souzu.keyword_color(),
            Self::Pinzu => Suit::Pinzu.keyword_color(),
            Self::Honors => Suit::Wind.keyword_color(),
            Self::Flowers => Suit::Flower.keyword_color(),
        }
    }

    pub fn contains(self, suit: Suit) -> bool {
        match self {
            Self::Manzu => suit == Suit::Manzu,
            Self::Souzu => suit == Suit::Souzu,
            Self::Pinzu => suit == Suit::Pinzu,
            Self::Honors => matches!(suit, Suit::Wind | Suit::Dragon),
            Self::Flowers => suit == Suit::Flower,
        }
    }

    pub fn flat_id(self) -> FocusId {
        FocusId(self as u32 + 1)
    }
}

pub fn tile_highlight_for(
    id: u32,
    selected: &[u32],
    player: Option<&[u32]>,
    house: Option<&[u32]>,
) -> TileHighlight {
    if house.is_some_and(|s| s.contains(&id)) {
        TileHighlight::HouseClaim
    } else if player.is_some_and(|s| s.contains(&id)) {
        TileHighlight::PlayerClaim
    } else if selected.contains(&id) {
        TileHighlight::Selected
    } else {
        TileHighlight::None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileHighlight {
    None,
    Selected,
    PlayerClaim,
    HouseClaim,
}

#[derive(Clone, Copy)]
pub struct DrawerLayout {
    pub drawer: SuitDrawer,
    pub header: [f32; 4],
    pub expanded: bool,
}

pub struct TilePickerLayout<A: Copy> {
    pub drawers: [DrawerLayout; 5],
    pub flat_items: Vec<FlatItem<A>>,
    pub placements: Vec<ShowcaseTilePlacement>,
    /// Maps pick slot index → tile id in the source list.
    pub pick_tile_ids: Vec<u32>,
    /// Screen-space pick targets aligned with `pick_tile_ids`.
    pub pick_tile_rects: Vec<[f32; 4]>,
    pub visible_count: usize,
    pub expanded_drawers: usize,
}

pub struct TilePickerScrollMeta {
    pub viewport: [f32; 4],
    pub scroll_y: f32,
    pub max_scroll_y: f32,
    pub content_height: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct TilePickerScrollbar {
    pub track: [f32; 4],
    pub thumb: [f32; 4],
    pub hit_track: [f32; 4],
    pub thumb_travel: f32,
}

#[derive(Clone, Copy)]
pub struct TilePickerSectionHeader {
    pub drawer: SuitDrawer,
    pub rect: [f32; 4],
}

/// One suit drawer band in a scrollable picker (content-space coordinates).
#[derive(Clone, Copy, Debug)]
pub struct TilePickerSectionMeta {
    pub drawer: SuitDrawer,
    /// Scroll target: Y of the section header in content space.
    pub header_content_y: f32,
    pub first_pick_index: usize,
    pub pick_count: usize,
}

pub struct ScrollableTilePickerLayout<A: Copy> {
    pub flat_items: Vec<FlatItem<A>>,
    pub placements: Vec<ShowcaseTilePlacement>,
    pub pick_tile_ids: Vec<u32>,
    pub pick_tile_rects: Vec<[f32; 4]>,
    pub visible_count: usize,
    pub scroll: TilePickerScrollMeta,
    pub section_headers: Vec<TilePickerSectionHeader>,
    pub sections: Vec<TilePickerSectionMeta>,
    /// Pinned font size for [`ScrollableTilePickerConfig::grouped_rows`] labels (0 otherwise).
    pub grouped_label_font_px: f32,
}

pub struct ScrollableTilePickerConfig<'a, A: Copy> {
    pub tiles: &'a [Tile],
    pub face_aspect: f32,
    pub scroll_y: f32,
    pub pickable: bool,
    pub dim_unmarked: bool,
    pub hovered_pick: Option<usize>,
    pub selected_ids: &'a [u32],
    pub player_claim_ids: Option<&'a [u32]>,
    pub house_claim_ids: Option<&'a [u32]>,
    /// Header chrome hit targets (Back, Seal, …) — not scrolled with tile rows.
    pub chrome_actions: &'a [(A, [f32; 4])],
    pub selection_outline_sel: Option<f32>,
    pub grid_cols: usize,
    /// When set, use this scroll viewport; otherwise derive from legacy title/footer bands.
    pub viewport: Option<[f32; 4]>,
    /// Left-column row labels instead of full-width drawer headers.
    pub grouped_rows: bool,
    /// Reserve a right gutter and expose scrollbar geometry when content overflows.
    pub show_scrollbar: bool,
}

pub struct TilePickerConfig<'a, A: Copy> {
    pub tiles: &'a [Tile],
    pub expanded: &'a [bool; 5],
    pub face_aspect: f32,
    pub pickable: bool,
    /// Dim unmarked tiles (decimation picker); claimed tiles keep their own brightness.
    pub dim_unmarked: bool,
    pub hovered_pick: Option<usize>,
    pub selected_ids: &'a [u32],
    pub player_claim_ids: Option<&'a [u32]>,
    pub house_claim_ids: Option<&'a [u32]>,
    pub drawer_action: Option<fn(SuitDrawer) -> A>,
    pub footer_actions: &'a [(A, [f32; 4])],
    pub content_top_extra: f32,
    /// When > 1, each expanded drawer repeats its suit tiles this many times.
    pub tile_repeat: usize,
    /// When set, selected tiles use this outline rim (`2.0` = decimation crimson).
    pub selection_outline_sel: Option<f32>,
}

pub fn camera_params(h: f32) -> CameraParams {
    let cam_scale = h / 1600.0;
    CameraParams {
        eye: [0.0, -200.0 * cam_scale, 2040.0 * cam_scale],
        target: [0.0, -50.0 * cam_scale, 0.0],
        up: [0.0, 0.0, 1.0],
        fovy_deg: 45.0,
        clip_near: None,
        clip_far: None,
    }
}

pub fn tiles_for_drawer(tiles: &[Tile], drawer: SuitDrawer) -> Vec<Tile> {
    tiles
        .iter()
        .copied()
        .filter(|t| drawer.contains(t.suit))
        .collect()
}

/// Repeat each drawer’s tiles for GPU stress testing (`repeat` 1 = natural wall count).
pub fn tiles_for_drawer_with_repeat(
    tiles: &[Tile],
    drawer: SuitDrawer,
    repeat: usize,
) -> Vec<Tile> {
    let base = tiles_for_drawer(tiles, drawer);
    repeat_tiles(&base, repeat, (drawer as u32) * 10_000)
}

/// Repeat an entire tile list for flat stress-lab grids (`repeat` 1 = no duplication).
pub fn wall_tiles_with_repeat(tiles: &[Tile], repeat: usize) -> Vec<Tile> {
    repeat_tiles(tiles, repeat, 0)
}

fn repeat_tiles(tiles: &[Tile], repeat: usize, id_salt: u32) -> Vec<Tile> {
    if repeat <= 1 || tiles.is_empty() {
        return tiles.to_vec();
    }
    const STRESS_TILE_ID_BASE: u32 = 500_000;
    let mut out = Vec::with_capacity(tiles.len() * repeat);
    for r in 0..repeat {
        for (i, &t) in tiles.iter().enumerate() {
            let mut copy = t;
            copy.id = STRESS_TILE_ID_BASE
                + id_salt
                + (r as u32) * tiles.len() as u32
                + i as u32;
            out.push(copy);
        }
    }
    out
}

pub fn compute_tile_picker_layout<A: Copy>(
    w: f32,
    h: f32,
    config: TilePickerConfig<'_, A>,
) -> TilePickerLayout<A> {
    let scale = metrics::scene_scale(w, h);
    let margin = (14.0 * scale).max(8.0);
    let title_font = crate::render::theme::typography::size(
        crate::render::theme::typography::H20,
        h,
    );
    let body_font = crate::render::theme::typography::size(
        crate::render::theme::typography::H42,
        h,
    );
    let hint_reserve = screen_footer_reserve(w, h);
    let footer_y = h - hint_reserve - (36.0 * scale).max(28.0) - margin;
    let band = tile_band(
        w,
        margin,
        title_font + config.content_top_extra,
        footer_y - margin,
    );

    let header_h = (body_font * 1.35).max(28.0);
    let drawer_gap = (6.0 * scale).max(4.0);
    let expanded_count = config.expanded.iter().filter(|&&e| e).count();
    let headers_total = 5.0 * header_h + 4.0 * drawer_gap;
    let body_budget = (band[3] - headers_total).max(0.0);
    let body_per_expanded = if expanded_count > 0 {
        body_budget / expanded_count as f32
    } else {
        0.0
    };

    let mut drawers = [DrawerLayout {
        drawer: SuitDrawer::Manzu,
        header: [0.0; 4],
        expanded: false,
    }; 5];
    let mut flat_items = Vec::new();
    let mut placements = Vec::new();
    let mut pick_tile_ids = Vec::new();
    let mut pick_tile_rects = Vec::new();
    let mut pick_cursor = 0usize;
    let mut y = band[1];

    for (i, drawer) in SuitDrawer::ALL.iter().enumerate() {
        let is_open = config.expanded[i];
        let header = [band[0], y, band[2], header_h];
        drawers[i] = DrawerLayout {
            drawer: *drawer,
            header,
            expanded: is_open,
        };
        if let Some(drawer_action) = config.drawer_action {
            flat_items.push(FlatItem::new(
                drawer.flat_id(),
                header,
                drawer_action(*drawer),
            ));
        }
        y += header_h;

        if is_open && body_per_expanded > 1.0 {
            let body = [band[0], y, band[2], body_per_expanded];
            let drawer_tiles =
                tiles_for_drawer_with_repeat(config.tiles, *drawer, config.tile_repeat);
            let plan = plan_tile_grid(drawer_tiles.len(), body, config.face_aspect);
            let positions = tile_grid_positions(drawer_tiles.len(), body, plan);
            for (tile, (cx, cy, size_px)) in drawer_tiles.iter().zip(positions) {
                let pick_id = pick_cursor;
                let hl = tile_highlight_for(
                    tile.id,
                    config.selected_ids,
                    config.player_claim_ids,
                    config.house_claim_ids,
                );
                let (selected, hovered, outline, glow, glow_color, brightness, outline_sel) =
                    placement_style(
                        hl,
                        config.hovered_pick == Some(pick_id),
                        config.dim_unmarked,
                        config.selection_outline_sel,
                    );
                placements.push(ShowcaseTilePlacement {
                    tile: *tile,
                    center_pos: [cx, cy, 0.0],
                    rotation: TILE_ROTATION,
                    scale: 1.0,
                    size_px,
                    brightness,
                    opacity: 1.0,
                    selected,
                    hovered,
                    outline,
                    glow,
                    glow_color,
                    outline_sel,
                    pick_id: config.pickable.then_some(pick_id),
                    overlay_rect_group: None,
                });
                pick_tile_ids.push(tile.id);
                let slot_h = size_px * config.face_aspect;
                pick_tile_rects.push([
                    cx - size_px * 0.5,
                    cy - slot_h * 0.5,
                    size_px,
                    slot_h,
                ]);
                pick_cursor += 1;
            }
            y += body_per_expanded;
        }
        y += drawer_gap;
    }

    for &(action, rect) in config.footer_actions {
        flat_items.push(FlatItem::new(
            FocusId(0xD100 + flat_items.len() as u32),
            rect,
            action,
        ));
    }

    TilePickerLayout {
        drawers,
        flat_items,
        placements,
        pick_tile_ids,
        pick_tile_rects,
        visible_count: pick_cursor,
        expanded_drawers: expanded_count,
    }
}

pub const STRESS_LAB_GRID_COLS: usize = 12;
pub const STRESS_LAB_SCROLL_LINES_PX: f32 = 48.0;

pub struct FlatTileStressConfig<'a, A: Copy> {
    pub tiles: &'a [Tile],
    pub face_aspect: f32,
    pub scroll_y: f32,
    pub pickable: bool,
    pub hovered_pick: Option<usize>,
    pub footer_actions: &'a [(A, [f32; 4])],
}

pub struct FlatTileStressLayout<A: Copy> {
    pub flat_items: Vec<FlatItem<A>>,
    pub placements: Vec<ShowcaseTilePlacement>,
    pub pick_tile_ids: Vec<u32>,
    pub pick_tile_rects: Vec<[f32; 4]>,
    pub visible_count: usize,
    pub tile_count: usize,
    pub scroll: TilePickerScrollMeta,
}

pub fn compute_flat_tile_stress_layout<A: Copy>(
    w: f32,
    h: f32,
    config: FlatTileStressConfig<'_, A>,
) -> FlatTileStressLayout<A> {
    let scale = metrics::scene_scale(w, h);
    let margin = (14.0 * scale).max(8.0);
    let title_font = crate::render::theme::typography::size(
        crate::render::theme::typography::H20,
        h,
    );
    let hint_reserve = screen_footer_reserve(w, h);
    let footer_y = h - hint_reserve - (36.0 * scale).max(28.0) - margin;
    let viewport = tile_band(w, margin, title_font * 3.2, footer_y - margin);
    let cols = STRESS_LAB_GRID_COLS.max(1);
    let col_gaps = (cols.saturating_sub(1)) as f32 * GRID_GAP_FRAC;
    let tile_w = (viewport[2] / (cols as f32 + col_gaps)).max(1.0);
    let gap = tile_w * GRID_GAP_FRAC;
    let tile_h = tile_w * config.face_aspect.max(1.0);
    let rows = config.tiles.len().div_ceil(cols);
    let content_height = rows as f32 * tile_h + rows.saturating_sub(1) as f32 * gap;
    let max_scroll_y = (content_height - viewport[3]).max(0.0);
    let scroll_y = config.scroll_y.clamp(0.0, max_scroll_y);

    let mut flat_items = Vec::new();
    let mut placements = Vec::new();
    let mut pick_tile_ids = Vec::new();
    let mut pick_tile_rects = Vec::new();
    let mut pick_cursor = 0usize;

    for (i, tile) in config.tiles.iter().enumerate() {
        let row = i / cols;
        let col = i % cols;
        let cx = viewport[0] + col as f32 * (tile_w + gap) + tile_w * 0.5;
        let cy = viewport[1] + row as f32 * (tile_h + gap) + tile_h * 0.5 - scroll_y;
        let screen_rect = [cx - tile_w * 0.5, cy - tile_h * 0.5, tile_w, tile_h];
        let pick_id = pick_cursor;
        pick_tile_ids.push(tile.id);
        pick_tile_rects.push(screen_rect);

        if rects_overlap(screen_rect, viewport) {
            let (selected, hovered, outline, glow, glow_color, brightness, outline_sel) =
                placement_style(
                    TileHighlight::None,
                    config.hovered_pick == Some(pick_id),
                    false,
                    None,
                );
            placements.push(ShowcaseTilePlacement {
                tile: *tile,
                center_pos: [cx, cy, 0.0],
                rotation: TILE_ROTATION,
                scale: 1.0,
                size_px: tile_w,
                brightness,
                opacity: 1.0,
                selected,
                hovered,
                outline,
                glow,
                glow_color,
                outline_sel,
                pick_id: config.pickable.then_some(pick_id),
                overlay_rect_group: None,
            });
        }
        pick_cursor += 1;
    }

    for &(action, rect) in config.footer_actions {
        flat_items.push(FlatItem::new(
            FocusId(0xD100 + flat_items.len() as u32),
            rect,
            action,
        ));
    }

    FlatTileStressLayout {
        flat_items,
        placements,
        pick_tile_ids,
        pick_tile_rects,
        visible_count: pick_cursor,
        tile_count: config.tiles.len(),
        scroll: TilePickerScrollMeta {
            viewport,
            scroll_y,
            max_scroll_y,
            content_height,
        },
    }
}

pub const SCROLLABLE_GRID_COLS: usize = 14;
const GROUPED_LABEL_COL_FRAC: f32 = 0.09;

/// Upper-left Back and matching chrome band (same proportions as Wall Ledger).
pub fn picker_header_chrome(window_w: f32, window_h: f32) -> ([f32; 4], f32) {
    let chrome = HeaderChromeMetrics::from_window(window_w, window_h);
    (chrome.back_rect_left(), chrome.chrome_bottom())
}

/// Upper-right confirm slot aligned with [`picker_header_chrome`].
fn picker_scrollbar_gutter(scale: f32) -> f32 {
    let track_w = (8.0 * scale).max(6.0);
    track_w + (12.0 * scale).max(8.0)
}

pub fn tile_picker_scrollbar(
    viewport: [f32; 4],
    scale: f32,
    content_height: f32,
    scroll_y: f32,
    max_scroll_y: f32,
) -> Option<TilePickerScrollbar> {
    if max_scroll_y <= 0.0 {
        return None;
    }
    let track_w = (8.0 * scale).max(6.0);
    let track_pad = (6.0 * scale).max(4.0);
    let track_x = viewport[0] + viewport[2] - track_w - track_pad;
    let track = [track_x, viewport[1], track_w, viewport[3]];
    let thumb_h = (track[3] * (viewport[3] / content_height.max(1.0)))
        .clamp((18.0 * scale).max(14.0), track[3]);
    let thumb_travel = (track[3] - thumb_h).max(0.0);
    let thumb_t = if max_scroll_y > 0.0 {
        scroll_y / max_scroll_y
    } else {
        0.0
    };
    let thumb_y = track[1] + thumb_travel * thumb_t;
    let thumb = [track_x, thumb_y, track_w, thumb_h];
    let hit_pad_x = (10.0 * scale).max(8.0);
    Some(TilePickerScrollbar {
        track,
        thumb,
        hit_track: [
            track_x - hit_pad_x,
            track[1],
            track_w + hit_pad_x * 2.0,
            track[3],
        ],
        thumb_travel,
    })
}

pub fn tile_picker_scroll_y_from_cursor(
    my: f32,
    grab_y: f32,
    sb: &TilePickerScrollbar,
    max_scroll_y: f32,
) -> f32 {
    if sb.thumb_travel <= 0.0 {
        return 0.0;
    }
    let thumb_top = (my - grab_y - sb.track[1]).clamp(0.0, sb.thumb_travel);
    (thumb_top / sb.thumb_travel) * max_scroll_y
}

pub fn push_tile_picker_scrollbar(
    frame: &mut crate::render::draw_cmd::UiFrame,
    sb: &TilePickerScrollbar,
    dragging: bool,
) {
    use crate::render::theme::color;
    use crate::render::wgpu_renderer::GpuInstance;

    frame.quad(GpuInstance {
        rect: sb.track,
        color: color::alpha(color::WALNUT_INK, 0.45),
        user: 0,
    });
    let thumb_alpha = if dragging { 1.0 } else { 0.82 };
    frame.quad(GpuInstance {
        rect: sb.thumb,
        color: color::alpha(color::CHAMPAGNE, thumb_alpha),
        user: 0,
    });
}

pub fn picker_seal_button_rect(window_w: f32, window_h: f32) -> [f32; 4] {
    HeaderChromeMetrics::from_window(window_w, window_h).right_confirm_rect(window_w)
}

fn legacy_scroll_viewport(w: f32, h: f32, content_top_extra: f32) -> [f32; 4] {
    let scale = metrics::scene_scale(w, h);
    let margin = (14.0 * scale).max(8.0);
    let title_font = crate::render::theme::typography::size(
        crate::render::theme::typography::H20,
        h,
    );
    let hint_reserve = screen_footer_reserve(w, h);
    let footer_y = h - hint_reserve - (36.0 * scale).max(28.0) - margin;
    tile_band(w, margin, title_font + content_top_extra, footer_y - margin)
}

pub fn compute_scrollable_tile_picker_layout<A: Copy>(
    w: f32,
    h: f32,
    config: ScrollableTilePickerConfig<'_, A>,
) -> ScrollableTilePickerLayout<A> {
    let scale = metrics::scene_scale(w, h);
    let body_font = crate::render::theme::typography::size(
        crate::render::theme::typography::H42,
        h,
    );
    let viewport = config.viewport.unwrap_or_else(|| legacy_scroll_viewport(w, h, 0.0));
    let scroll_gutter = if config.show_scrollbar {
        picker_scrollbar_gutter(scale)
    } else {
        0.0
    };
    let tile_viewport = [
        viewport[0],
        viewport[1],
        (viewport[2] - scroll_gutter).max(1.0),
        viewport[3],
    ];
    let cols = config.grid_cols.max(1);
    let section_gap = (8.0 * scale).max(5.0);
    let section_header_h = if config.grouped_rows {
        0.0
    } else {
        (body_font * 0.95).max(22.0)
    };
    let label_h_fallback = (body_font * 0.88).max(11.0);

    struct Section {
        drawer: SuitDrawer,
        tiles: Vec<Tile>,
        header_content_y: f32,
        grid_content_y: f32,
        row_h: f32,
    }

    let (label_col_w, grid_x, grid_w) = if config.grouped_rows {
        let label_col_w = tile_viewport[2] * GROUPED_LABEL_COL_FRAC;
        (
            label_col_w,
            tile_viewport[0] + label_col_w,
            tile_viewport[2] - label_col_w,
        )
    } else {
        (0.0, tile_viewport[0], tile_viewport[2])
    };
    let grouped_label_font_px = if config.grouped_rows {
        typography::tier_at_most(label_col_w * 0.92, h)
    } else {
        0.0
    };
    let label_h = if config.grouped_rows {
        grouped_label_font_px * PLAIN_TEXT_LINE_STEP_MUL
    } else {
        label_h_fallback
    };

    let tile_band_w = if config.grouped_rows {
        grid_w
    } else {
        tile_viewport[2]
    };
    let col_gaps = (cols.saturating_sub(1)) as f32 * GRID_GAP_FRAC;
    let tile_w = (tile_band_w / (cols as f32 + col_gaps)).max(1.0);
    let gap = tile_w * GRID_GAP_FRAC;
    let tile_h = tile_w * config.face_aspect.max(1.0);

    let mut sections = Vec::new();
    let mut content_y = 0.0_f32;
    for drawer in SuitDrawer::ALL {
        let tiles = tiles_for_drawer(config.tiles, drawer);
        if tiles.is_empty() {
            continue;
        }
        if !sections.is_empty() {
            content_y += section_gap;
        }
        let header_content_y = content_y;
        if !config.grouped_rows {
            content_y += section_header_h;
        }
        let grid_content_y = content_y;
        let rows = tiles.len().div_ceil(cols);
        let grid_h = rows as f32 * tile_h + rows.saturating_sub(1) as f32 * gap;
        content_y += grid_h;
        sections.push(Section {
            drawer,
            tiles,
            header_content_y,
            grid_content_y,
            row_h: grid_h,
        });
    }

    let content_height = content_y;
    let max_scroll_y = (content_height - tile_viewport[3]).max(0.0);
    let scroll_y = config.scroll_y.clamp(0.0, max_scroll_y);

    let mut flat_items = Vec::new();
    let mut placements = Vec::new();
    let mut pick_tile_ids = Vec::new();
    let mut pick_tile_rects = Vec::new();
    let mut section_headers = Vec::new();
    let mut section_meta = Vec::with_capacity(sections.len());
    let mut pick_cursor = 0usize;

    for section in &sections {
        section_meta.push(TilePickerSectionMeta {
            drawer: section.drawer,
            header_content_y: section.header_content_y,
            first_pick_index: pick_cursor,
            pick_count: section.tiles.len(),
        });

        if config.grouped_rows {
            let screen_y = tile_viewport[1] + section.header_content_y - scroll_y;
            let label_rect = [
                tile_viewport[0],
                screen_y + section.row_h * 0.5 - label_h * 0.5,
                label_col_w * 0.92,
                label_h,
            ];
            if rects_overlap(label_rect, tile_viewport) {
                section_headers.push(TilePickerSectionHeader {
                    drawer: section.drawer,
                    rect: label_rect,
                });
            }
        } else {
            let screen_header = [
                tile_viewport[0],
                tile_viewport[1] + section.header_content_y - scroll_y,
                tile_viewport[2],
                section_header_h,
            ];
            if rects_overlap(screen_header, tile_viewport) {
                section_headers.push(TilePickerSectionHeader {
                    drawer: section.drawer,
                    rect: screen_header,
                });
            }
        }

        for (i, tile) in section.tiles.iter().enumerate() {
            let row = i / cols;
            let col = i % cols;
            let cx = grid_x + col as f32 * (tile_w + gap) + tile_w * 0.5;
            let cy_content =
                section.grid_content_y + row as f32 * (tile_h + gap) + tile_h * 0.5;
            let cy = tile_viewport[1] + cy_content - scroll_y;
            let screen_rect = [cx - tile_w * 0.5, cy - tile_h * 0.5, tile_w, tile_h];

            let pick_id = pick_cursor;
            pick_tile_ids.push(tile.id);
            pick_tile_rects.push(screen_rect);

            if rects_overlap(screen_rect, tile_viewport) {
                let hl = tile_highlight_for(
                    tile.id,
                    config.selected_ids,
                    config.player_claim_ids,
                    config.house_claim_ids,
                );
                let (selected, hovered, outline, glow, glow_color, brightness, outline_sel) =
                    placement_style(
                        hl,
                        config.hovered_pick == Some(pick_id),
                        config.dim_unmarked,
                        config.selection_outline_sel,
                    );
                placements.push(ShowcaseTilePlacement {
                    tile: *tile,
                    center_pos: [cx, cy, 0.0],
                    rotation: TILE_ROTATION,
                    scale: 1.0,
                    size_px: tile_w,
                    brightness,
                    opacity: 1.0,
                    selected,
                    hovered,
                    outline,
                    glow,
                    glow_color,
                    outline_sel,
                    pick_id: config.pickable.then_some(pick_id),
                    overlay_rect_group: None,
                });
            }
            pick_cursor += 1;
        }
    }

    for &(action, rect) in config.chrome_actions {
        flat_items.push(FlatItem::new(
            FocusId(0xD100 + flat_items.len() as u32),
            rect,
            action,
        ));
    }

    ScrollableTilePickerLayout {
        flat_items,
        placements,
        pick_tile_ids,
        pick_tile_rects,
        visible_count: pick_cursor,
        scroll: TilePickerScrollMeta {
            viewport: tile_viewport,
            scroll_y,
            max_scroll_y,
            content_height,
        },
        section_headers,
        sections: section_meta,
        grouped_label_font_px,
    }
}

fn rects_overlap(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[0] + b[2] && a[0] + a[2] > b[0] && a[1] < b[1] + b[3] && a[1] + a[3] > b[1]
}

/// Bool mask over visible pick slots for marquee selection.
pub fn pick_selection_mask(selected_ids: &[u32], pick_tile_ids: &[u32]) -> Vec<bool> {
    pick_tile_ids
        .iter()
        .map(|id| selected_ids.contains(id))
        .collect()
}

pub fn apply_pick_selection_mask(
    selected_ids: &mut Vec<u32>,
    pick_tile_ids: &[u32],
    mask: &[bool],
) {
    selected_ids.clear();
    for (i, &id) in pick_tile_ids.iter().enumerate() {
        if mask.get(i).copied().unwrap_or(false) {
            selected_ids.push(id);
        }
    }
}

fn section_for_slot<'a>(
    slot: usize,
    sections: &'a [TilePickerSectionMeta],
) -> Option<&'a TilePickerSectionMeta> {
    sections.iter().find(|s| {
        slot >= s.first_pick_index && slot < s.first_pick_index + s.pick_count
    })
}

/// Pick slots covered by a 2D grid marquee between `start` and `current`.
///
/// When both endpoints share a section, selection is the axis-aligned
/// rectangle in row-major grid coordinates. Cross-section drags fall back to
/// the inclusive linear index span.
pub fn grid_marquee_swept_slots(
    start: usize,
    current: usize,
    cols: usize,
    sections: &[TilePickerSectionMeta],
) -> Vec<usize> {
    if start == current {
        return vec![start];
    }
    let cols = cols.max(1);
    match (
        section_for_slot(start, sections),
        section_for_slot(current, sections),
    ) {
        (Some(section), Some(other)) if section.first_pick_index == other.first_pick_index => {
            let base = section.first_pick_index;
            let local_start = start - base;
            let local_current = current - base;
            let (r0, c0) = (local_start / cols, local_start % cols);
            let (r1, c1) = (local_current / cols, local_current % cols);
            let r_lo = r0.min(r1);
            let r_hi = r0.max(r1);
            let c_lo = c0.min(c1);
            let c_hi = c0.max(c1);
            let mut out = Vec::new();
            for r in r_lo..=r_hi {
                for c in c_lo..=c_hi {
                    let local = r * cols + c;
                    if local < section.pick_count {
                        out.push(base + local);
                    }
                }
            }
            out
        }
        _ => {
            let lo = start.min(current);
            let hi = start.max(current);
            (lo..=hi).collect()
        }
    }
}

fn placement_style(
    hl: TileHighlight,
    hovered: bool,
    dim_unmarked: bool,
    selection_outline_sel: Option<f32>,
) -> (bool, bool, bool, bool, Option<[f32; 4]>, f32, Option<f32>) {
    match hl {
        TileHighlight::None => (
            false,
            hovered,
            hovered,
            false,
            None,
            if dim_unmarked { 0.90 } else { 1.0 },
            None,
        ),
        TileHighlight::Selected => (
            selection_outline_sel.is_none(),
            hovered,
            true,
            false,
            None,
            1.0,
            selection_outline_sel,
        ),
        TileHighlight::PlayerClaim => (
            true,
            hovered,
            true,
            false,
            None,
            1.0,
            None,
        ),
        TileHighlight::HouseClaim => (
            false,
            hovered,
            true,
            false,
            None,
            1.0,
            Some(3.0),
        ),
    }
}

fn tile_band(w: f32, margin: f32, title_font: f32, content_bottom: f32) -> [f32; 4] {
    let side = (w * 0.04).max(margin);
    let top = margin + title_font * 3.2;
    [side, top, w - side * 2.0, (content_bottom - top).max(1.0)]
}

fn plan_tile_grid(count: usize, band: [f32; 4], face_aspect: f32) -> TileGridPlan {
    let face_aspect = face_aspect.max(1.0);
    let [_, _, bw, bh] = band;
    if count == 0 || bw <= 0.0 || bh <= 0.0 {
        return TileGridPlan {
            cols: 1,
            rows: 0,
            size_px: 0.0,
            gap: 0.0,
            face_aspect,
        };
    }

    let mut best = TileGridPlan {
        cols: 1,
        rows: count,
        size_px: 0.0,
        gap: 0.0,
        face_aspect,
    };

    for cols in 1..=count {
        let rows = count.div_ceil(cols);
        let col_gaps = (cols.saturating_sub(1)) as f32 * GRID_GAP_FRAC;
        let row_gaps = (rows.saturating_sub(1)) as f32 * GRID_GAP_FRAC;
        let size = (bw / (cols as f32 + col_gaps))
            .min(bh / (rows as f32 * face_aspect + row_gaps));
        if size > best.size_px {
            best = TileGridPlan {
                cols,
                rows,
                size_px: size,
                gap: size * GRID_GAP_FRAC,
                face_aspect,
            };
        }
    }

    best
}

fn tile_grid_positions(count: usize, band: [f32; 4], plan: TileGridPlan) -> Vec<(f32, f32, f32)> {
    let [bx, by, bw, bh] = band;
    if count == 0 || plan.size_px <= 0.0 {
        return Vec::new();
    }

    let cols = plan.cols;
    let slot_w = plan.size_px;
    let slot_h = slot_w * plan.face_aspect;
    let gap = plan.gap;
    let grid_w = cols as f32 * slot_w + gap * cols.saturating_sub(1) as f32;
    let rows = count.div_ceil(cols);
    let grid_h = rows as f32 * slot_h + gap * rows.saturating_sub(1) as f32;
    let origin_x = bx + (bw - grid_w) * 0.5;
    let origin_y = by + (bh - grid_h) * 0.5;

    (0..count)
        .map(|i| {
            let col = i % cols;
            let row = i / cols;
            let cx = origin_x + col as f32 * (slot_w + gap) + slot_w * 0.5;
            let cy = origin_y + row as f32 * (slot_h + gap) + slot_h * 0.5;
            (cx, cy, slot_w)
        })
        .collect()
}

pub fn footer_button_rects(w: f32, h: f32, button_count: usize) -> Vec<[f32; 4]> {
    let scale = metrics::scene_scale(w, h);
    let margin = (14.0 * scale).max(8.0);
    let row_h = (36.0 * scale).max(28.0);
    let hint_reserve = screen_footer_reserve(w, h);
    let footer_y = h - hint_reserve - row_h - margin;
    let gap = 8.0_f32;
    let btn_w = ((w - margin * 2.0) - gap * (button_count.saturating_sub(1) as f32)).max(1.0)
        / button_count.max(1) as f32;
    (0..button_count)
        .map(|i| [margin + (btn_w + gap) * i as f32, footer_y, btn_w, row_h])
        .collect()
}

pub struct DecimationRevealLayout<A: Copy> {
    pub placements: Vec<ShowcaseTilePlacement>,
    pub flat_items: Vec<FlatItem<A>>,
    /// Screen-space centers for burn spark particles.
    pub spark_anchors: Vec<[f32; 2]>,
    pub yours_label_y: f32,
    pub house_label_y: f32,
    pub group_x: f32,
    pub group_w: f32,
}

pub fn tile_for_id(display_tiles: &[Tile], id: u32) -> Option<Tile> {
    display_tiles.iter().find(|t| t.id == id).copied()
}

pub fn compute_decimation_reveal_layout<A: Copy>(
    w: f32,
    h: f32,
    face_aspect: f32,
    display_tiles: &[Tile],
    player: &[u32],
    house: &[u32],
    burn_t: f32,
    footer_actions: &[(A, [f32; 4])],
) -> DecimationRevealLayout<A> {
    let scale = metrics::scene_scale(w, h);
    let margin = (14.0 * scale).max(8.0);
    let body_font = crate::render::theme::typography::size(
        crate::render::theme::typography::H42,
        h,
    );
    let title_font = crate::render::theme::typography::size(
        crate::render::theme::typography::H20,
        h,
    );
    let group_gap = body_font * 2.0;

    let max_group_w = w * 0.88;
    let tile_count = player.len().max(1);
    let gap = max_group_w * 0.02;
    let size_px =
        ((max_group_w - gap * (tile_count.saturating_sub(1) as f32)) / tile_count as f32)
            .min(h * 0.11);
    let group_w = size_px * tile_count as f32 + gap * tile_count.saturating_sub(1) as f32;
    let group_x = (w - group_w) * 0.5;

    let content_top = margin + title_font * 3.2;
    let row_h = size_px * face_aspect;
    let yours_y = content_top + body_font * 1.1 + row_h * 0.5;
    let house_y = yours_y + row_h * 0.5 + group_gap + body_font * 0.9 + row_h * 0.5;

    // Decimation uses the undimmed stairway now, so start a little lower to
    // avoid blown-out tile faces while preserving the burn fade.
    let char_brightness = (0.82 - burn_t * 0.74).max(0.08);
    let outline_on = burn_t < 0.92;

    let mut placements = Vec::new();
    let mut spark_anchors = Vec::new();

    for (row, is_player) in [(player, true), (house, false)] {
        for (i, &id) in row.iter().enumerate() {
            let Some(tile) = tile_for_id(display_tiles, id) else {
                continue;
            };
            let cx = group_x + size_px * 0.5 + i as f32 * (size_px + gap);
            let cy = if is_player { yours_y } else { house_y };
            placements.push(ShowcaseTilePlacement {
                tile,
                center_pos: [cx, cy, 0.0],
                rotation: TILE_ROTATION,
                scale: 1.0,
                size_px,
                brightness: char_brightness,
                opacity: 1.0,
                selected: false,
                hovered: false,
                outline: outline_on,
                glow: false,
                glow_color: None,
                outline_sel: if outline_on {
                    Some(if is_player { 2.0 } else { 3.0 })
                } else {
                    None
                },
                pick_id: None,
                overlay_rect_group: None,
            });
            spark_anchors.push([cx, cy - size_px * face_aspect * 0.12]);
        }
    }

    let mut flat_items = Vec::new();
    for &(action, rect) in footer_actions {
        flat_items.push(FlatItem::new(
            FocusId(0xD100 + flat_items.len() as u32),
            rect,
            action,
        ));
    }

    DecimationRevealLayout {
        placements,
        flat_items,
        spark_anchors,
        yours_label_y: yours_y - row_h * 0.5 - body_font * 0.25,
        house_label_y: house_y - row_h * 0.5 - body_font * 0.25,
        group_x,
        group_w,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::deck::build_wall;

    #[test]
    fn drawer_tiles_cover_wall_by_suit() {
        let wall = build_wall();
        assert_eq!(tiles_for_drawer(&wall, SuitDrawer::Manzu).len(), 36);
        assert_eq!(tiles_for_drawer(&wall, SuitDrawer::Flowers).len(), 4);
    }

    #[test]
    fn tile_repeat_multiplies_drawer_count() {
        let wall = build_wall();
        assert_eq!(
            tiles_for_drawer_with_repeat(&wall, SuitDrawer::Flowers, 3).len(),
            12
        );
    }

    #[test]
    fn wall_repeat_multiplies_full_wall() {
        let wall = build_wall();
        assert_eq!(wall_tiles_with_repeat(&wall, 3).len(), wall.len() * 3);
    }

    #[test]
    fn scrollable_layout_sections_cover_all_picks() {
        let wall = build_wall();
        let layout: ScrollableTilePickerLayout<()> = compute_scrollable_tile_picker_layout(
            1280.0,
            720.0,
            ScrollableTilePickerConfig {
                tiles: &wall,
                face_aspect: 1.35,
                scroll_y: 0.0,
                pickable: true,
                dim_unmarked: false,
                hovered_pick: None,
                selected_ids: &[],
                player_claim_ids: None,
                house_claim_ids: None,
                chrome_actions: &[],
                selection_outline_sel: None,
                grid_cols: SCROLLABLE_GRID_COLS,
                viewport: None,
                grouped_rows: false,
                show_scrollbar: false,
            },
        );
        assert_eq!(layout.sections.len(), 5);
        let covered: usize = layout.sections.iter().map(|s| s.pick_count).sum();
        assert_eq!(covered, layout.visible_count);
        assert_eq!(layout.sections[0].first_pick_index, 0);
        assert_eq!(
            layout.sections[1].first_pick_index,
            layout.sections[0].pick_count
        );
    }

    #[test]
    fn grid_marquee_selects_axis_aligned_rectangle() {
        let sections = [TilePickerSectionMeta {
            drawer: SuitDrawer::Manzu,
            header_content_y: 0.0,
            first_pick_index: 0,
            pick_count: 28,
        }];
        let swept = grid_marquee_swept_slots(26, 2, 14, &sections);
        let mut expected = Vec::new();
        for r in 0..=1 {
            for c in 2..=12 {
                expected.push(r * 14 + c);
            }
        }
        assert_eq!(swept, expected);
    }
}
