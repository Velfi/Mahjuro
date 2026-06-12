//! Shop focus helpers — chrome button registration and spatial nav over projected rects.

use super::shared::ShopFocus;
use crate::ui::focus_nav::{FocusDir, FocusNavState};
use crate::ui::widget_tree::{FlatItem, TreeState};

fn shop_chrome_focus(f: ShopFocus) -> bool {
    matches!(
        f,
        ShopFocus::Dish(_) | ShopFocus::Restock | ShopFocus::NextRound | ShopFocus::WallHud
    )
}

/// Plain spatial pick over all focus rects (stock + HUD chrome).
pub(in crate::scenes::shop) fn shop_directional_pick(
    nav: &mut FocusNavState<ShopFocus>,
    all_rects: &[(ShopFocus, [f32; 4])],
    cur: ShopFocus,
    dir: FocusDir,
) -> Option<ShopFocus> {
    nav.load_candidates(all_rects, &[]);
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
    use super::super::pick_ids::PICK_JOURNAL_BOOK;
    use super::super::shared::shop_focus_inspectable;
    use super::*;
    use crate::game::game_mode::GameMode;
    use crate::game::run::RunState;
    use crate::ui::focus_nav::rect_center;

    fn projected_rects(
        scene: &super::super::ShopScene,
        run: &RunState,
    ) -> Vec<(ShopFocus, [f32; 4])> {
        super::super::view::projected_shop_focus_rects(scene, 1920.0, 1080.0, run)
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
