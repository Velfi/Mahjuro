//! Shop focus helpers — chrome button registration and spatial nav over projected rects.

use super::pick_ids::PICK_JOURNAL_BOOK;
use super::shared::ShopFocus;
use crate::ui::focus_nav::{FocusDir, FocusNavState, rect_center};
use crate::ui::widget_tree::{FlatItem, TreeState};

/// For-sale vs owned indices on the flat `Relic` / `Ribbon` / `Talisman` lists.
pub(in crate::scenes::shop) struct ShopNavStockBounds {
    pub n_sale_relics: usize,
    pub n_sale_zodiacs: usize,
    pub n_sale_talismans: usize,
}

impl ShopNavStockBounds {
    pub(in crate::scenes::shop) fn from_scene(scene: &super::ShopScene) -> Self {
        Self {
            n_sale_relics: scene.items.len(),
            n_sale_zodiacs: scene.zodiac_items.len(),
            n_sale_talismans: scene.talisman_items.len(),
        }
    }
}

fn shop_chrome_focus(f: ShopFocus) -> bool {
    matches!(
        f,
        ShopFocus::Dish(_) | ShopFocus::Restock | ShopFocus::NextRound | ShopFocus::WallHud
    )
}

fn shop_sale_stock(f: ShopFocus, bounds: &ShopNavStockBounds) -> bool {
    match f {
        ShopFocus::Relic(i) => i < bounds.n_sale_relics,
        ShopFocus::Ribbon(i) => i < bounds.n_sale_zodiacs,
        ShopFocus::Talisman(i) => i < bounds.n_sale_talismans,
        ShopFocus::Pack(_) => true,
        _ => false,
    }
}

fn shop_player_stock(f: ShopFocus, bounds: &ShopNavStockBounds) -> bool {
    match f {
        ShopFocus::Relic(i) => i >= bounds.n_sale_relics,
        ShopFocus::Ribbon(i) => i >= bounds.n_sale_zodiacs,
        ShopFocus::Talisman(i) => i >= bounds.n_sale_talismans,
        _ => false,
    }
}

fn target_rect(focus_rects: &[(ShopFocus, [f32; 4])], target: ShopFocus) -> Option<[f32; 4]> {
    focus_rects
        .iter()
        .find(|(f, _)| *f == target)
        .map(|(_, r)| *r)
}

fn x_nearest_target(
    focus_rects: &[(ShopFocus, [f32; 4])],
    anchor_x: f32,
    pred: impl Fn(ShopFocus) -> bool,
) -> Option<ShopFocus> {
    focus_rects
        .iter()
        .filter(|(f, _)| pred(*f))
        .min_by(|(_, a), (_, b)| {
            (rect_center(*a).0 - anchor_x)
                .abs()
                .partial_cmp(&(rect_center(*b).0 - anchor_x).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(f, _)| *f)
}

fn sorted_row_targets(
    focus_rects: &[(ShopFocus, [f32; 4])],
    pred: impl Fn(ShopFocus) -> bool,
) -> Vec<ShopFocus> {
    let mut row: Vec<(ShopFocus, [f32; 4])> = focus_rects
        .iter()
        .copied()
        .filter(|(f, _)| pred(*f))
        .collect();
    row.sort_by(|(_, a), (_, b)| {
        rect_center(*a)
            .0
            .partial_cmp(&rect_center(*b).0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    row.into_iter().map(|(f, _)| f).collect()
}

fn push_horizontal_chain(edges: &mut Vec<(ShopFocus, FocusDir, ShopFocus)>, row: &[ShopFocus]) {
    for pair in row.windows(2) {
        edges.push((pair[0], FocusDir::Right, pair[1]));
        edges.push((pair[1], FocusDir::Left, pair[0]));
    }
}

/// Explicit links the auto-inferred graph cannot derive from storeroom layout.
pub(in crate::scenes::shop) fn shop_nav_edges(
    focus_rects: &[(ShopFocus, [f32; 4])],
    bounds: &ShopNavStockBounds,
) -> Vec<(ShopFocus, FocusDir, ShopFocus)> {
    let mut edges = Vec::new();

    let sale_row = sorted_row_targets(focus_rects, |f| shop_sale_stock(f, bounds));
    let player_row = sorted_row_targets(focus_rects, |f| shop_player_stock(f, bounds));
    push_horizontal_chain(&mut edges, &sale_row);
    push_horizontal_chain(&mut edges, &player_row);

    for sale in &sale_row {
        let Some(sale_rect) = target_rect(focus_rects, *sale) else {
            continue;
        };
        let anchor_x = rect_center(sale_rect).0;
        if let Some(player) =
            x_nearest_target(focus_rects, anchor_x, |f| shop_player_stock(f, bounds))
        {
            edges.push((*sale, FocusDir::Down, player));
        }
    }

    for player in &player_row {
        let Some(player_rect) = target_rect(focus_rects, *player) else {
            continue;
        };
        let anchor_x = rect_center(player_rect).0;
        if let Some(sale) = x_nearest_target(focus_rects, anchor_x, |f| shop_sale_stock(f, bounds))
        {
            edges.push((*player, FocusDir::Up, sale));
        }
    }

    let journal = ShopFocus::Dish(PICK_JOURNAL_BOOK);
    let restock = ShopFocus::Restock;
    let leave = ShopFocus::NextRound;
    let wall = ShopFocus::WallHud;

    let mut rail: Vec<ShopFocus> = [leave, restock, journal]
        .into_iter()
        .filter(|f| target_rect(focus_rects, *f).is_some())
        .collect();
    rail.sort_by(|a, b| {
        let ya = rect_center(target_rect(focus_rects, *a).unwrap()).1;
        let yb = rect_center(target_rect(focus_rects, *b).unwrap()).1;
        ya.partial_cmp(&yb).unwrap_or(std::cmp::Ordering::Equal)
    });
    for pair in rail.windows(2) {
        edges.push((pair[0], FocusDir::Down, pair[1]));
        edges.push((pair[1], FocusDir::Up, pair[0]));
    }
    if let (Some(last_rail), true) = (rail.last().copied(), target_rect(focus_rects, wall).is_some())
    {
        edges.push((last_rail, FocusDir::Down, wall));
        edges.push((wall, FocusDir::Up, last_rail));
    }

    if target_rect(focus_rects, restock).is_some() {
        if let Some(rightmost_sale) = sale_row.last().copied() {
            edges.push((rightmost_sale, FocusDir::Right, restock));
        }
        if let Some(restock_rect) = target_rect(focus_rects, restock)
            && let Some(sale) = x_nearest_target(focus_rects, rect_center(restock_rect).0, |f| {
                shop_sale_stock(f, bounds)
            })
        {
            edges.push((restock, FocusDir::Left, sale));
        }
    }

    if target_rect(focus_rects, leave).is_some()
        && let Some(leave_rect) = target_rect(focus_rects, leave)
        && let Some(sale) = x_nearest_target(focus_rects, rect_center(leave_rect).0, |f| {
            shop_sale_stock(f, bounds)
        })
    {
        edges.push((leave, FocusDir::Left, sale));
    }

    if target_rect(focus_rects, journal).is_some() {
        if let Some(rightmost_player) = player_row.last().copied() {
            edges.push((rightmost_player, FocusDir::Right, journal));
        }
        if let Some(journal_rect) = target_rect(focus_rects, journal) {
            let anchor_x = rect_center(journal_rect).0;
            if let Some(player) =
                x_nearest_target(focus_rects, anchor_x, |f| shop_player_stock(f, bounds))
            {
                edges.push((journal, FocusDir::Left, player));
            } else if let Some(sale) =
                x_nearest_target(focus_rects, anchor_x, |f| shop_sale_stock(f, bounds))
            {
                edges.push((journal, FocusDir::Left, sale));
            }
        }
    }

    if let Some(wall_rect) = target_rect(focus_rects, wall) {
        let anchor_x = rect_center(wall_rect).0;
        if let Some(player) =
            x_nearest_target(focus_rects, anchor_x, |f| shop_player_stock(f, bounds))
        {
            edges.push((wall, FocusDir::Left, player));
        } else if let Some(sale) =
            x_nearest_target(focus_rects, anchor_x, |f| shop_sale_stock(f, bounds))
        {
            edges.push((wall, FocusDir::Left, sale));
        }
    }

    edges
}

/// Spatial pick over all focus rects with storeroom-specific explicit edges.
pub(in crate::scenes::shop) fn shop_directional_pick(
    nav: &mut FocusNavState<ShopFocus>,
    all_rects: &[(ShopFocus, [f32; 4])],
    cur: ShopFocus,
    dir: FocusDir,
    bounds: &ShopNavStockBounds,
) -> Option<ShopFocus> {
    let edges = shop_nav_edges(all_rects, bounds);
    nav.load_candidates(all_rects, &edges);
    nav.pick(cur, dir)
}

/// Inspect overlay cycles inspectable stock only (no HUD chrome).
pub(in crate::scenes::shop) fn shop_inspect_nav_pick(
    nav: &mut FocusNavState<ShopFocus>,
    inspect_rects: &[(ShopFocus, [f32; 4])],
    cur: ShopFocus,
    dir: FocusDir,
) -> Option<ShopFocus> {
    nav.load_candidates(inspect_rects, &[]);
    nav.pick(cur, dir)
}

pub(in crate::scenes::shop) fn flat_chrome_items(
    focus_rects: &[(ShopFocus, [f32; 4])],
) -> Vec<FlatItem<ShopFocus>> {
    focus_rects
        .iter()
        .filter(|(f, _)| shop_chrome_focus(*f))
        .map(|(f, r)| FlatItem::new(f.chrome_id(), *r, *f))
        .collect()
}

pub(in crate::scenes::shop) fn register_shop_chrome_buttons(
    focus_rects: &[(ShopFocus, [f32; 4])],
    buttons: &mut Vec<crate::scenes::ButtonDef>,
) {
    TreeState::new().register_flat_buttons(&flat_chrome_items(focus_rects), buttons);
}

#[cfg(test)]
mod tests {
    use super::super::shared::shop_focus_inspectable;
    use super::*;
    use crate::core::relic::RelicId;
    use crate::game::game_mode::GameMode;
    use crate::game::run::RunState;

    fn projected_rects(
        scene: &super::super::ShopScene,
        run: &RunState,
    ) -> Vec<(ShopFocus, [f32; 4])> {
        super::super::view::projected_shop_focus_rects(scene, 1920.0, 1080.0, run)
    }

    fn bounds(scene: &super::super::ShopScene) -> ShopNavStockBounds {
        ShopNavStockBounds::from_scene(scene)
    }

    fn right_rail_plaques(rects: &[(ShopFocus, [f32; 4])]) -> Vec<(ShopFocus, f32)> {
        rects
            .iter()
            .filter(|(f, _)| {
                matches!(
                    f,
                    ShopFocus::NextRound | ShopFocus::Restock | ShopFocus::Dish(PICK_JOURNAL_BOOK)
                )
            })
            .map(|(f, r)| (*f, rect_center(*r).1))
            .collect()
    }

    #[test]
    fn chrome_rail_follows_screen_y_via_spatial_nav() {
        let mut run = RunState::new(GameMode::standard());
        let scene = super::super::ShopScene::new(
            &mut run,
            &crate::core::progression::PlayerProgress::new(),
        );
        let rects = projected_rects(&scene, &run);
        let b = bounds(&scene);
        let mut rail = right_rail_plaques(&rects);
        assert_eq!(
            rail.len(),
            3,
            "expected three right-rail plaques, got {rail:?}"
        );
        rail.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut nav = FocusNavState::new();
        let top = rail[0].0;
        let mid = rail[1].0;
        let bottom = rail[2].0;
        let top_y = rail[0].1;
        let mid_y = rail[1].1;
        let bottom_y = rail[2].1;

        let down_from_top = shop_directional_pick(&mut nav, &rects, top, FocusDir::Down, &b);
        assert_eq!(
            down_from_top,
            Some(mid),
            "spatial Down from top plaque should reach the next plaque below"
        );
        assert!(mid_y > top_y);

        let down_from_mid = shop_directional_pick(&mut nav, &rects, mid, FocusDir::Down, &b);
        assert_eq!(
            down_from_mid,
            Some(bottom),
            "spatial Down from middle plaque should reach the lowest plaque"
        );
        assert!(bottom_y > mid_y);

        assert_eq!(
            shop_directional_pick(&mut nav, &rects, bottom, FocusDir::Up, &b),
            Some(mid),
            "spatial Up from bottom plaque should reach middle"
        );
    }

    #[test]
    fn shelf_down_reaches_owned_inventory_not_journal() {
        let mut run = RunState::new(GameMode::standard());
        run.relics.active.push(RelicId::HungryGhost);
        let scene = super::super::ShopScene::new(
            &mut run,
            &crate::core::progression::PlayerProgress::new(),
        );
        let rects = projected_rects(&scene, &run);
        let b = bounds(&scene);
        let n_sale = scene.items.len();
        assert!(
            n_sale > 0,
            "standard shop should stock at least one for-sale relic"
        );
        let sale = ShopFocus::Relic(0);
        let owned = ShopFocus::Relic(n_sale);
        assert!(
            rects.iter().any(|(f, _)| *f == owned),
            "owned relic should have a focus rect"
        );

        let mut nav = FocusNavState::new();
        let next = shop_directional_pick(&mut nav, &rects, sale, FocusDir::Down, &b);
        assert_ne!(
            next,
            Some(ShopFocus::Dish(PICK_JOURNAL_BOOK)),
            "Down from for-sale shelf should not jump to the journal plaque"
        );
        assert_eq!(
            next,
            Some(owned),
            "Down from for-sale shelf should reach the owned relic row below"
        );

        let up = shop_directional_pick(&mut nav, &rects, owned, FocusDir::Up, &b);
        assert_eq!(
            up,
            Some(sale),
            "Up from owned inventory should return to the aligned for-sale shelf slot"
        );
    }

    #[test]
    fn rightmost_sale_reaches_chrome_rail_via_right() {
        let mut run = RunState::new(GameMode::standard());
        let scene = super::super::ShopScene::new(
            &mut run,
            &crate::core::progression::PlayerProgress::new(),
        );
        let rects = projected_rects(&scene, &run);
        let b = bounds(&scene);
        let sale_row = sorted_row_targets(&rects, |f| shop_sale_stock(f, &b));
        let rightmost = sale_row
            .last()
            .copied()
            .expect("shop should stock for-sale items");
        let mut nav = FocusNavState::new();
        assert_eq!(
            shop_directional_pick(&mut nav, &rects, rightmost, FocusDir::Right, &b),
            Some(ShopFocus::Restock),
            "Right from the rightmost shelf slot should enter the chrome rail"
        );
    }

    #[test]
    fn wall_hud_links_to_rail_and_player_row() {
        let mut run = RunState::new(GameMode::standard());
        run.relics.active.push(RelicId::HungryGhost);
        let scene = super::super::ShopScene::new(
            &mut run,
            &crate::core::progression::PlayerProgress::new(),
        );
        let rects = projected_rects(&scene, &run);
        let b = bounds(&scene);
        let n_sale = scene.items.len();
        let owned = ShopFocus::Relic(n_sale);
        let mut nav = FocusNavState::new();

        let up_from_wall =
            shop_directional_pick(&mut nav, &rects, ShopFocus::WallHud, FocusDir::Up, &b);
        assert_eq!(
            up_from_wall,
            Some(ShopFocus::Dish(PICK_JOURNAL_BOOK)),
            "Up from wall HUD should climb the right rail"
        );

        let left_from_wall =
            shop_directional_pick(&mut nav, &rects, ShopFocus::WallHud, FocusDir::Left, &b);
        assert_eq!(
            left_from_wall,
            Some(owned),
            "Left from wall HUD should reach the player inventory row"
        );
    }

    #[test]
    fn inspect_nav_excludes_chrome() {
        let mut run = RunState::new(GameMode::standard());
        let scene = super::super::ShopScene::new(
            &mut run,
            &crate::core::progression::PlayerProgress::new(),
        );
        let rects = projected_rects(&scene, &run);
        let inspect: Vec<_> = rects
            .into_iter()
            .filter(|(f, _)| shop_focus_inspectable(*f))
            .collect();
        if inspect.len() < 2 {
            return;
        }
        let start = inspect[0].0;
        let mut nav = FocusNavState::new();
        let next = shop_inspect_nav_pick(&mut nav, &inspect, start, FocusDir::Right);
        assert!(
            next.map(|f| shop_focus_inspectable(f)).unwrap_or(true),
            "inspect nav should never pick chrome"
        );
    }
}
