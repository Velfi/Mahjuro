//! Shop spatial focus navigation — stock rows vs HUD chrome use separate candidate
//! pools so Restock / Leave cannot steal vertical moves between shelf and inventory.

use super::pick_ids::PICK_JOURNAL_BOOK;
use super::shared::ShopFocus;
use crate::ui::focus_nav::{FocusDir, FocusNavState, rect_center};

#[inline]
pub(in crate::scenes::shop) fn shop_focus_is_chrome(f: ShopFocus) -> bool {
    matches!(
        f,
        ShopFocus::Dish(_) | ShopFocus::Restock | ShopFocus::NextRound | ShopFocus::WallHud
    )
}

#[inline]
fn shop_is_right_rail_plaque(f: ShopFocus) -> bool {
    matches!(
        f,
        ShopFocus::NextRound | ShopFocus::Restock | ShopFocus::Dish(PICK_JOURNAL_BOOK)
    )
}

/// Rightmost for-sale slot on the upper shelf row (not the lower inventory bar).
fn rightmost_shelf_stock(stock: &[(ShopFocus, [f32; 4])]) -> Option<ShopFocus> {
    let min_row_y = stock
        .iter()
        .map(|(_, r)| rect_center(*r).1)
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?;
    let row_snap = stock
        .iter()
        .map(|(_, r)| r[3])
        .fold(0.0_f32, f32::max)
        .max(48.0)
        * 0.75;
    stock
        .iter()
        .filter(|(_, r)| (rect_center(*r).1 - min_row_y).abs() <= row_snap)
        .max_by(|(_, a), (_, b)| {
            rect_center(*a)
                .0
                .partial_cmp(&rect_center(*b).0)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(f, _)| *f)
}

fn shop_stock_rects(rects: &[(ShopFocus, [f32; 4])]) -> Vec<(ShopFocus, [f32; 4])> {
    rects
        .iter()
        .copied()
        .filter(|(f, _)| !shop_focus_is_chrome(*f))
        .collect()
}

fn shop_chrome_rects(rects: &[(ShopFocus, [f32; 4])]) -> Vec<(ShopFocus, [f32; 4])> {
    rects
        .iter()
        .copied()
        .filter(|(f, _)| shop_focus_is_chrome(*f))
        .collect()
}

fn rect_for_focus(rects: &[(ShopFocus, [f32; 4])], focus: ShopFocus) -> Option<[f32; 4]> {
    rects
        .iter()
        .find_map(|(f, r)| (*f == focus).then_some(*r))
}

fn shop_pool_nav_pick(
    nav: &mut FocusNavState<ShopFocus>,
    candidates: &[(ShopFocus, [f32; 4])],
    from: ShopFocus,
    dir: FocusDir,
) -> Option<ShopFocus> {
    nav.load_candidates(candidates, &[]);
    nav.pick(from, dir)
}

fn shop_pool_pick_from_rect(
    nav: &mut FocusNavState<ShopFocus>,
    candidates: &[(ShopFocus, [f32; 4])],
    from_rect: [f32; 4],
    dir: FocusDir,
) -> Option<ShopFocus> {
    nav.load_candidates(candidates, &[]);
    nav.pick_from_rect(from_rect, dir)
}

fn shop_enter_chrome_from_stock(
    nav: &mut FocusNavState<ShopFocus>,
    all_rects: &[(ShopFocus, [f32; 4])],
    from: ShopFocus,
    dir: FocusDir,
) -> Option<ShopFocus> {
    let from_rect = rect_for_focus(all_rects, from)?;
    let chrome = shop_chrome_rects(all_rects);
    if chrome.is_empty() {
        return None;
    }
    shop_pool_pick_from_rect(nav, &chrome, from_rect, dir)
}

fn shop_enter_stock_from_chrome(
    nav: &mut FocusNavState<ShopFocus>,
    all_rects: &[(ShopFocus, [f32; 4])],
    from: ShopFocus,
    dir: FocusDir,
) -> Option<ShopFocus> {
    let from_rect = rect_for_focus(all_rects, from)?;
    let stock = shop_stock_rects(all_rects);
    if stock.is_empty() {
        return None;
    }
    shop_pool_pick_from_rect(nav, &stock, from_rect, dir)
}

/// Directional pick for the storeroom — stock rows and HUD chrome use separate
/// candidate pools so Restock / Leave cannot steal vertical moves between shelf and inventory.
pub(in crate::scenes::shop) fn shop_directional_pick(
    nav: &mut FocusNavState<ShopFocus>,
    all_rects: &[(ShopFocus, [f32; 4])],
    cur: ShopFocus,
    dir: FocusDir,
) -> Option<ShopFocus> {
    if shop_focus_is_chrome(cur) {
        if dir == FocusDir::Left && shop_is_right_rail_plaque(cur) {
            return rightmost_shelf_stock(&shop_stock_rects(all_rects));
        }
        let chrome = shop_chrome_rects(all_rects);
        shop_pool_nav_pick(nav, &chrome, cur, dir)
            .or_else(|| shop_enter_stock_from_chrome(nav, all_rects, cur, dir))
    } else {
        let stock = shop_stock_rects(all_rects);
        shop_pool_nav_pick(nav, &stock, cur, dir)
            .or_else(|| shop_enter_chrome_from_stock(nav, all_rects, cur, dir))
    }
}

/// Inspect overlay cycles inspectable stock only (no HUD chrome).
pub(in crate::scenes::shop) fn shop_inspect_nav_pick(
    nav: &mut FocusNavState<ShopFocus>,
    inspect_rects: &[(ShopFocus, [f32; 4])],
    cur: ShopFocus,
    dir: FocusDir,
) -> Option<ShopFocus> {
    shop_pool_nav_pick(nav, inspect_rects, cur, dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::pick_ids::PICK_JOURNAL_BOOK;
    use super::super::shared::shop_focus_inspectable;
    use crate::core::relic::RelicId;
    use crate::game::game_mode::GameMode;
    use crate::game::run::RunState;
    use crate::ui::focus_nav::rect_center;

    fn projected_rects(scene: &super::super::ShopScene, run: &RunState) -> Vec<(ShopFocus, [f32; 4])> {
        super::super::view::projected_shop_focus_rects(scene, 1920.0, 1080.0, run)
    }

    fn first_for_sale_relic(rects: &[(ShopFocus, [f32; 4])]) -> ShopFocus {
        rects
            .iter()
            .find_map(|(f, _)| matches!(f, ShopFocus::Relic(i) if *i == 0).then_some(*f))
            .expect("for-sale relic")
    }

    fn shelf_relic_aligned_with_owned(
        rects: &[(ShopFocus, [f32; 4])],
        n_for_sale: usize,
    ) -> Option<(ShopFocus, ShopFocus)> {
        let mut owned: Vec<_> = rects
            .iter()
            .filter(|(f, _)| matches!(f, ShopFocus::Relic(i) if *i >= n_for_sale))
            .collect();
        let mut for_sale: Vec<_> = rects
            .iter()
            .filter(|(f, _)| matches!(f, ShopFocus::Relic(i) if *i < n_for_sale))
            .collect();
        if owned.is_empty() || for_sale.is_empty() {
            return None;
        }
        owned.sort_by(|(_, a), (_, b)| {
            rect_center(*a)
                .0
                .partial_cmp(&rect_center(*b).0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let (owned_focus, owned_rect) = owned[0];
        let (ox, _) = rect_center(*owned_rect);
        for_sale.sort_by(|(_, a), (_, b)| {
            let ax = rect_center(*a).0;
            let bx = rect_center(*b).0;
            (ax - ox)
                .abs()
                .partial_cmp(&(bx - ox).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Some((for_sale[0].0, *owned_focus))
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
    fn shelf_down_reaches_inventory_not_restock() {
        let mut run = RunState::new(GameMode::standard());
        run.relics.active = vec![RelicId::PairPower, RelicId::TripletBoost];
        run.recompute_capacities();
        let scene = super::super::ShopScene::new(
            &mut run,
            &crate::core::progression::PlayerProgress::new(),
        );
        let rects = projected_rects(&scene, &run);
        let n_for_sale = scene.items.len();
        let Some((shelf, _owned)) = shelf_relic_aligned_with_owned(&rects, n_for_sale) else {
            return;
        };
        let mut nav = FocusNavState::new();
        let next = shop_directional_pick(&mut nav, &rects, shelf, FocusDir::Down);
        assert!(
            matches!(next, Some(ShopFocus::Relic(i)) if i >= n_for_sale),
            "Down from shelf should reach inventory row, got {next:?}"
        );
        assert!(
            !matches!(next, Some(f) if shop_focus_is_chrome(f)),
            "Down from shelf must not pick Restock / Leave chrome"
        );
    }

    fn first_owned_relic(rects: &[(ShopFocus, [f32; 4])], n_for_sale: usize) -> Option<ShopFocus> {
        rects
            .iter()
            .find_map(|(f, _)| matches!(f, ShopFocus::Relic(i) if *i >= n_for_sale).then_some(*f))
    }

    #[test]
    fn inventory_up_reaches_shelf_not_restock() {
        let mut run = RunState::new(GameMode::standard());
        run.relics.active = vec![RelicId::PairPower];
        run.recompute_capacities();
        let scene = super::super::ShopScene::new(
            &mut run,
            &crate::core::progression::PlayerProgress::new(),
        );
        let rects = projected_rects(&scene, &run);
        let n_for_sale = scene.items.len();
        let owned = first_owned_relic(&rects, n_for_sale).expect("owned relic row");
        let mut nav = FocusNavState::new();
        let next = shop_directional_pick(&mut nav, &rects, owned, FocusDir::Up);
        assert!(
            matches!(next, Some(ShopFocus::Relic(i)) if i < n_for_sale),
            "Up from inventory should reach for-sale shelf, got {next:?}"
        );
    }

    #[test]
    fn shelf_horizontal_stays_on_shelf() {
        let mut run = RunState::new(GameMode::standard());
        let scene = super::super::ShopScene::new(
            &mut run,
            &crate::core::progression::PlayerProgress::new(),
        );
        let rects = projected_rects(&scene, &run);
        if scene.items.len() < 2 {
            return;
        }
        let start = first_for_sale_relic(&rects);
        let mut nav = FocusNavState::new();
        let next = shop_directional_pick(&mut nav, &rects, start, FocusDir::Right);
        assert!(
            matches!(next, Some(ShopFocus::Relic(1))),
            "Right from first shelf relic should stay on shelf, got {next:?}"
        );
    }

    #[test]
    fn left_from_rail_reaches_rightmost_shelf_item() {
        let mut run = RunState::new(GameMode::standard());
        let scene = super::super::ShopScene::new(
            &mut run,
            &crate::core::progression::PlayerProgress::new(),
        );
        let rects = projected_rects(&scene, &run);
        let stock = shop_stock_rects(&rects);
        let expected = rightmost_shelf_stock(&stock).expect("shelf stock");
        let mut nav = FocusNavState::new();
        for plaque in [
            ShopFocus::NextRound,
            ShopFocus::Restock,
            ShopFocus::Dish(PICK_JOURNAL_BOOK),
        ] {
            let next = shop_directional_pick(&mut nav, &rects, plaque, FocusDir::Left);
            assert_eq!(
                next,
                Some(expected),
                "Left from {plaque:?} should reach rightmost shelf item"
            );
        }
        assert_ne!(
            shop_directional_pick(&mut nav, &rects, ShopFocus::NextRound, FocusDir::Left),
            Some(ShopFocus::Restock),
            "Left from PLAY must not step down to Restock"
        );
    }

    #[test]
    fn chrome_rail_follows_screen_y_via_spatial_nav() {
        let mut run = RunState::new(GameMode::standard());
        let scene = super::super::ShopScene::new(
            &mut run,
            &crate::core::progression::PlayerProgress::new(),
        );
        let rects = projected_rects(&scene, &run);
        let mut rail = right_rail_plaques(&rects);
        assert_eq!(rail.len(), 3, "expected three right-rail plaques, got {rail:?}");
        rail.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut nav = FocusNavState::new();
        let top = rail[0].0;
        let mid = rail[1].0;
        let bottom = rail[2].0;
        let top_y = rail[0].1;
        let mid_y = rail[1].1;
        let bottom_y = rail[2].1;

        let down_from_top = shop_directional_pick(&mut nav, &rects, top, FocusDir::Down);
        assert_eq!(
            down_from_top,
            Some(mid),
            "spatial Down from top plaque should reach the next plaque below"
        );
        assert!(mid_y > top_y);

        let down_from_mid = shop_directional_pick(&mut nav, &rects, mid, FocusDir::Down);
        assert_eq!(
            down_from_mid,
            Some(bottom),
            "spatial Down from middle plaque should reach the lowest plaque"
        );
        assert!(bottom_y > mid_y);

        assert_eq!(
            shop_directional_pick(&mut nav, &rects, bottom, FocusDir::Up),
            Some(mid),
            "spatial Up from bottom plaque should reach middle"
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
