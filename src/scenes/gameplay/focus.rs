use crate::ui::input::UiAction;

/// Bottom-row gameplay buttons. Order in `ALL_BUTTONS` is the keyboard nav order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum GameplayButton {
    /// Commit selected melds into the structure (mirror).
    Play,
    /// Cash in structure for score (wood tablet above mirror).
    Trigger,
    Discard,
}

impl GameplayButton {
    /// Maps a focusable action button to its `UiAction`. Returns `None`
    pub(super) fn ui_action(self) -> Option<UiAction> {
        Some(match self {
            GameplayButton::Play => UiAction::ScoreHand,
            GameplayButton::Trigger => UiAction::TriggerStructure,
            GameplayButton::Discard => UiAction::CommitDiscard,
        })
    }
}

/// Which counter peg row a `FocusTarget::Peg` refers to. The peg block on
/// the table holds two distinct groups: jade pegs counting plays remaining
/// (`Hands`) and amber pegs counting discards remaining (`Discards`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum PegKind {
    Hands,
    Discards,
}

/// Which odometer bank a `FocusTarget::ScoreRoller` refers to. Bank 0 is
/// round score (left); bank 1 is the blind target (right).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ScoreRollerBank {
    Score,
    Target,
}

/// Every gameplay HUD element that the keyboard / controller / cursor can
/// "select". Spatial 2D navigation chooses the next target by nearest-in-
/// direction over the on-screen rect for each variant. Display-only variants
/// (relics, pegs, gold, yaku, dora) are still focusable so the player can
/// read tooltips for them via the keyboard, even though `Confirm` is a
/// no-op for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum FocusTarget {
    HandTile(usize),
    Button(GameplayButton),
    Consumable(usize),
    Relic(usize),
    Peg(PegKind),
    Gold,
    /// One of the bone yaku-progress tablets in the row above the action
    /// bar. Index matches `visible_previews[i]`.
    YakuTablet(usize),
    /// The brass dora indicator stand at the back-right of the table.
    Dora,
    /// The brass ordeal-rule stand parked to the right of dora during ordeal rounds.
    Ordeal,
    /// The brass round-wind stand beside the dora plinth.
    RoundWind,
    /// Optional post-discard undo control (accessibility).
    DiscardUndo,
    /// Yaku journal on the table (`player_yaku_journal` empty).
    Journal,
    /// Guide book on the table (`player_guidebook` empty).
    Guidebook,
    /// Lower-right wall supply HUD (opens Wall Ledger).
    WallHud,
    /// Round-score odometer (left bank on the score frame).
    ScoreRoller(ScoreRollerBank),
}

pub(super) const ALL_BUTTONS: [GameplayButton; 3] = [
    GameplayButton::Discard,
    GameplayButton::Play,
    GameplayButton::Trigger,
];

pub(super) fn rebuild_focus_nav(
    nav: &mut crate::ui::focus_nav::FocusNavState<FocusTarget>,
    focus_rects: &[(FocusTarget, [f32; 4])],
    paused: bool,
) {
    use crate::ui::focus_nav::FocusScope;

    nav.begin_frame();
    nav.clear_edges();
    if paused {
        nav.set_scope(Some(FocusScope::Modal));
    } else {
        nav.set_scope(None);
        for &(target, rect) in focus_rects {
            nav.add(target, rect);
        }
        for (from, dir, to) in gameplay_nav_edges(focus_rects) {
            nav.edge(from, dir, to);
        }
    }
    nav.end_frame();
}

pub(super) fn gameplay_nav_edges(
    focus_rects: &[(FocusTarget, [f32; 4])],
) -> Vec<(FocusTarget, crate::ui::focus_nav::FocusDir, FocusTarget)> {
    use crate::ui::focus_nav::FocusDir;

    let mut edges = Vec::new();

    let mut relic_indices = focus_rects.iter().filter_map(|(target, _)| match *target {
        FocusTarget::Relic(i) => Some(i),
        _ => None,
    });
    if let Some(last_idx) = relic_indices.next_back()
        && focus_rects
            .iter()
            .any(|(t, _)| matches!(t, FocusTarget::Dora))
    {
        edges.push((
            FocusTarget::Relic(last_idx),
            FocusDir::Right,
            FocusTarget::Dora,
        ));
    }

    let score_roller = focus_rects.iter().any(|(t, _)| {
        matches!(t, FocusTarget::ScoreRoller(ScoreRollerBank::Score))
    });
    let target_roller = focus_rects.iter().any(|(t, _)| {
        matches!(t, FocusTarget::ScoreRoller(ScoreRollerBank::Target))
    });
    if score_roller && target_roller {
        edges.push((
            FocusTarget::ScoreRoller(ScoreRollerBank::Score),
            FocusDir::Right,
            FocusTarget::ScoreRoller(ScoreRollerBank::Target),
        ));
    }

    let discard_present = focus_rects
        .iter()
        .any(|(t, _)| matches!(t, FocusTarget::Button(GameplayButton::Discard)));
    if discard_present {
        for i in 0..=1 {
            if focus_rects
                .iter()
                .any(|(t, _)| matches!(t, FocusTarget::HandTile(idx) if *idx == i))
            {
                edges.push((
                    FocusTarget::HandTile(i),
                    FocusDir::Down,
                    FocusTarget::Button(GameplayButton::Discard),
                ));
            }
        }
    }

    let mut hand_entries: Vec<(FocusTarget, [f32; 4])> = focus_rects
        .iter()
        .filter(|(t, _)| matches!(t, FocusTarget::HandTile(_)))
        .copied()
        .collect();
    hand_entries.sort_by(|(_, a), (_, b)| {
        a[0]
            .partial_cmp(&b[0])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if let (Some(&(first, _)), Some(&(last, _))) = (hand_entries.first(), hand_entries.last())
        && first != last
    {
        edges.push((first, FocusDir::Left, last));
        edges.push((last, FocusDir::Right, first));
    }
    if let Some(&(last_hand, _)) = hand_entries.last() {
        // Explicit QoL hop: from the rightmost hand tile, Up should first hit
        // cash-in when that control is visible; otherwise jump to the yen pile.
        let cash_in = FocusTarget::Button(GameplayButton::Trigger);
        if focus_rects.iter().any(|(t, _)| *t == cash_in) {
            edges.push((last_hand, FocusDir::Up, cash_in));
        } else if focus_rects
            .iter()
            .any(|(t, _)| matches!(t, FocusTarget::Gold))
        {
            edges.push((last_hand, FocusDir::Up, FocusTarget::Gold));
        }
    }

    edges
}

/// Pick the focus to adopt after using the consumable at `used_idx`. The
/// consumable list shifted left, so the next consumable now lives at
/// `used_idx`. If the row is now empty, fall back to the first hand tile
/// in the focus graph.
pub(super) fn focus_after_consumable_use(
    used_idx: usize,
    remaining_consumables: usize,
    hand_len: usize,
    focus_rects: &[(FocusTarget, [f32; 4])],
) -> Option<FocusTarget> {
    if remaining_consumables > 0 {
        let next = used_idx.min(remaining_consumables - 1);
        return Some(FocusTarget::Consumable(next));
    }
    default_hand_tile_focus(hand_len, focus_rects)
}

/// First hand tile in the focus graph, or slot 0 when the graph is not
/// built yet (first frame after deal).
pub(super) fn default_hand_tile_focus(
    hand_len: usize,
    focus_rects: &[(FocusTarget, [f32; 4])],
) -> Option<FocusTarget> {
    if hand_len == 0 {
        return None;
    }
    focus_rects
        .iter()
        .find_map(|(t, _)| matches!(t, FocusTarget::HandTile(_)).then_some(*t))
        .or(Some(FocusTarget::HandTile(0)))
}

pub(super) fn play_select_sfx(
    bus: &mut crate::game::event_bus::EventBus,
    added: u32,
    removed: u32,
) {
    use crate::game::event_bus::GameEvent;
    use crate::sfx_id::SfxId;
    if added > 0 {
        bus.push(GameEvent::UiSound(SfxId::TileSelect));
    }
    if removed > 0 {
        bus.push(GameEvent::UiSound(SfxId::TileDeselect));
    }
}

/// Coarse kind of a `FocusTarget`, collapsing payload-carrying variants so
/// scrolling through tiles of the same kind doesn't re-trigger the sound.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FocusKind {
    HandTile,
    Button,
    Consumable,
    Relic,
    Peg,
    Gold,
    YakuTablet,
    Dora,
    Ordeal,
    RoundWind,
    ScoreRoller,
}

pub(super) fn focus_kind(f: Option<FocusTarget>) -> Option<FocusKind> {
    match f? {
        FocusTarget::HandTile(_) => Some(FocusKind::HandTile),
        FocusTarget::Button(_) => Some(FocusKind::Button),
        FocusTarget::Consumable(_) => Some(FocusKind::Consumable),
        FocusTarget::Relic(_) => Some(FocusKind::Relic),
        FocusTarget::Peg(_) => Some(FocusKind::Peg),
        FocusTarget::Gold => Some(FocusKind::Gold),
        FocusTarget::YakuTablet(_) => Some(FocusKind::YakuTablet),
        FocusTarget::Dora => Some(FocusKind::Dora),
        FocusTarget::Ordeal => Some(FocusKind::Ordeal),
        FocusTarget::RoundWind => Some(FocusKind::RoundWind),
        FocusTarget::ScoreRoller(_) => Some(FocusKind::ScoreRoller),
        FocusTarget::DiscardUndo
        | FocusTarget::Journal
        | FocusTarget::Guidebook
        | FocusTarget::WallHud => Some(FocusKind::Button),
    }
}

pub(super) fn focus_kind_sfx(k: FocusKind) -> Option<crate::sfx_id::SfxId> {
    use crate::sfx_id::SfxId;
    Some(match k {
        FocusKind::HandTile => SfxId::FocusHandTile,
        FocusKind::Button => SfxId::FocusButton,
        FocusKind::Consumable => SfxId::FocusConsumable,
        FocusKind::Relic => SfxId::FocusRelic,
        FocusKind::Peg => SfxId::FocusPeg,
        FocusKind::Gold => SfxId::FocusGold,
        FocusKind::YakuTablet => SfxId::FocusYakuTablet,
        FocusKind::Dora => SfxId::FocusDora,
        FocusKind::Ordeal => SfxId::FocusDora,
        FocusKind::RoundWind => SfxId::FocusDora,
        FocusKind::ScoreRoller => SfxId::FocusGold,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::focus_nav::{FocusDir, FocusNavState};

    /// Bottom gameplay strip, left → right (inferred from screen geometry).
    const BOTTOM_ROW_FOCUS_ORDER_LTR: [FocusTarget; 6] = [
        FocusTarget::Button(GameplayButton::Discard),
        FocusTarget::Peg(PegKind::Discards),
        FocusTarget::Guidebook,
        FocusTarget::Journal,
        FocusTarget::Peg(PegKind::Hands),
        FocusTarget::Button(GameplayButton::Play),
    ];

    /// Same strip, right → left.
    const BOTTOM_ROW_FOCUS_ORDER_RTL: [FocusTarget; 6] = [
        FocusTarget::Button(GameplayButton::Play),
        FocusTarget::Peg(PegKind::Hands),
        FocusTarget::Journal,
        FocusTarget::Guidebook,
        FocusTarget::Peg(PegKind::Discards),
        FocusTarget::Button(GameplayButton::Discard),
    ];

    fn load_nav(rects: &[(FocusTarget, [f32; 4])]) -> FocusNavState<FocusTarget> {
        let mut nav = FocusNavState::new();
        nav.begin_frame();
        for &(target, rect) in rects {
            nav.add(target, rect);
        }
        for (from, dir, to) in gameplay_nav_edges(rects) {
            nav.edge(from, dir, to);
        }
        nav.end_frame();
        nav
    }

    fn load_nav_inferred_only(rects: &[(FocusTarget, [f32; 4])]) -> FocusNavState<FocusTarget> {
        let mut nav = FocusNavState::new();
        nav.begin_frame();
        for &(target, rect) in rects {
            nav.add(target, rect);
        }
        nav.end_frame();
        nav
    }

    fn live_gameplay_focus_rects(hand_len: usize) -> Vec<(FocusTarget, [f32; 4])> {
        crate::scenes::gameplay::glb_anchors::projected_gameplay_focus_rects_for_tests(hand_len)
    }

    fn assert_hand_lr_inference(nav: &mut FocusNavState<FocusTarget>, hand_len: usize) {
        for i in 0..hand_len {
            for dir in [FocusDir::Left, FocusDir::Right] {
                if let Some(next) = nav.pick(FocusTarget::HandTile(i), dir) {
                    assert!(
                        matches!(next, FocusTarget::HandTile(_)),
                        "HandTile({i}) {dir:?} left the hand row: {next:?}"
                    );
                }
            }
        }
        for i in 0..hand_len.saturating_sub(1) {
            assert_eq!(
                nav.pick(FocusTarget::HandTile(i), FocusDir::Right),
                Some(FocusTarget::HandTile(i + 1)),
                "HandTile({i}) Right"
            );
        }
        for i in 1..hand_len {
            assert_eq!(
                nav.pick(FocusTarget::HandTile(i), FocusDir::Left),
                Some(FocusTarget::HandTile(i - 1)),
                "HandTile({i}) Left"
            );
        }
        if hand_len >= 2 {
            assert_eq!(
                nav.pick(FocusTarget::HandTile(0), FocusDir::Left),
                Some(FocusTarget::HandTile(hand_len - 1)),
                "HandTile(0) Left wraps"
            );
            assert_eq!(
                nav.pick(FocusTarget::HandTile(hand_len - 1), FocusDir::Right),
                Some(FocusTarget::HandTile(0)),
                "HandTile({}) Right wraps",
                hand_len - 1
            );
        }
    }

    /// Gameplay HUD layout: bottom strip plus hand, relics, dora, etc. as distractors.
    /// Prefer [`live_gameplay_focus_rects`] for geometry that matches projected GLB layout.
    fn sample_gameplay_focus_rects(include_journal: bool) -> Vec<(FocusTarget, [f32; 4])> {
        let mut rects = vec![
            (FocusTarget::Consumable(0), [40.0, 890.0, 50.0, 60.0]),
            (FocusTarget::DiscardUndo, [55.0, 820.0, 80.0, 36.0]),
            (FocusTarget::Button(GameplayButton::Discard), [100.0, 900.0, 120.0, 80.0]),
            (FocusTarget::Peg(PegKind::Discards), [280.0, 880.0, 40.0, 100.0]),
            (FocusTarget::Guidebook, [400.0, 870.0, 80.0, 90.0]),
        ];
        if include_journal {
            rects.push((FocusTarget::Journal, [520.0, 870.0, 80.0, 90.0]));
        }
        rects.extend([
            (FocusTarget::Peg(PegKind::Hands), [720.0, 880.0, 40.0, 100.0]),
            (FocusTarget::Button(GameplayButton::Play), [860.0, 900.0, 120.0, 80.0]),
            (FocusTarget::Button(GameplayButton::Trigger), [1100.0, 900.0, 100.0, 70.0]),
        ]);
        for i in 0..14 {
            rects.push((
                FocusTarget::HandTile(i),
                [300.0 + i as f32 * 62.0, 520.0, 58.0, 90.0],
            ));
        }
        for i in 0..3 {
            rects.push((
                FocusTarget::Relic(i),
                [120.0 + i as f32 * 72.0, 680.0, 64.0, 64.0],
            ));
        }
        rects.extend([
            (FocusTarget::YakuTablet(0), [900.0, 650.0, 70.0, 50.0]),
            (FocusTarget::Gold, [1200.0, 760.0, 100.0, 80.0]),
            (FocusTarget::RoundWind, [1380.0, 420.0, 80.0, 100.0]),
            (FocusTarget::Dora, [1500.0, 400.0, 80.0, 100.0]),
            (FocusTarget::WallHud, [1700.0, 950.0, 90.0, 50.0]),
        ]);
        rects
    }

    fn is_bottom_row_target(t: FocusTarget) -> bool {
        matches!(
            t,
            FocusTarget::Button(GameplayButton::Discard | GameplayButton::Play)
                | FocusTarget::Peg(_)
                | FocusTarget::Guidebook
                | FocusTarget::Journal
        )
    }

    fn assert_bottom_row_only(path: &[FocusTarget]) {
        assert!(
            path.iter().all(|&t| is_bottom_row_target(t)),
            "bottom-row walk must not land on hand/dora/relics/etc.: {path:?}"
        );
    }

    /// Repeated `dir` presses from `start` until navigation stops or `limit` steps.
    fn walk_focus_order(
        nav: &mut FocusNavState<FocusTarget>,
        start: FocusTarget,
        dir: FocusDir,
        limit: usize,
    ) -> Vec<FocusTarget> {
        let mut path = vec![start];
        let mut cur = start;
        for _ in 0..limit {
            let Some(next) = nav.pick(cur, dir) else {
                break;
            };
            if path.contains(&next) {
                break;
            }
            path.push(next);
            cur = next;
        }
        path
    }

    fn assert_focus_order(
        nav: &mut FocusNavState<FocusTarget>,
        start: FocusTarget,
        dir: FocusDir,
        expected: &[FocusTarget],
    ) {
        let path = walk_focus_order(nav, start, dir, expected.len().saturating_sub(1));
        assert_bottom_row_only(&path);
        assert_eq!(
            path, expected,
            "focus order from {start:?} via {dir:?}"
        );
    }

    #[test]
    fn bottom_row_focus_order_left_to_right_live_glb() {
        let rects = live_gameplay_focus_rects(14);
        let mut nav = load_nav(&rects);
        assert_focus_order(
            &mut nav,
            FocusTarget::Button(GameplayButton::Discard),
            FocusDir::Right,
            &BOTTOM_ROW_FOCUS_ORDER_LTR,
        );
    }

    #[test]
    fn bottom_row_focus_order_right_to_left_live_glb() {
        let rects = live_gameplay_focus_rects(14);
        let mut nav = load_nav(&rects);
        assert_focus_order(
            &mut nav,
            FocusTarget::Button(GameplayButton::Play),
            FocusDir::Left,
            &BOTTOM_ROW_FOCUS_ORDER_RTL,
        );
    }

    #[test]
    fn hand_row_lr_stays_on_tiles_live_glb() {
        let rects = live_gameplay_focus_rects(14);
        let mut nav = load_nav(&rects);
        assert_hand_lr_inference(&mut nav, 14);
    }

    #[test]
    fn hand_row_lr_on_sample_layout() {
        let rects = sample_gameplay_focus_rects(true);
        let mut nav = load_nav(&rects);
        assert_hand_lr_inference(&mut nav, 14);
    }

    #[test]
    fn bottom_row_focus_order_left_to_right() {
        let rects = sample_gameplay_focus_rects(true);
        let mut nav = load_nav(&rects);
        assert_focus_order(
            &mut nav,
            FocusTarget::Button(GameplayButton::Discard),
            FocusDir::Right,
            &BOTTOM_ROW_FOCUS_ORDER_LTR,
        );
    }

    #[test]
    fn bottom_row_focus_order_right_to_left() {
        let rects = sample_gameplay_focus_rects(true);
        let mut nav = load_nav(&rects);
        assert_focus_order(
            &mut nav,
            FocusTarget::Button(GameplayButton::Play),
            FocusDir::Left,
            &BOTTOM_ROW_FOCUS_ORDER_RTL,
        );
    }

    #[test]
    fn action_bar_focus_order_omits_missing_journal() {
        let rects = sample_gameplay_focus_rects(false);
        let mut nav = load_nav(&rects);
        let expected = [
            FocusTarget::Button(GameplayButton::Discard),
            FocusTarget::Peg(PegKind::Discards),
            FocusTarget::Guidebook,
            FocusTarget::Peg(PegKind::Hands),
            FocusTarget::Button(GameplayButton::Play),
        ];
        assert_focus_order(
            &mut nav,
            FocusTarget::Button(GameplayButton::Discard),
            FocusDir::Right,
            &expected,
        );
    }

    #[test]
    fn explicit_edges_relic_to_dora_only_from_last_relic_right() {
        let rects = [
            (FocusTarget::Relic(0), [0.0, 0.0, 10.0, 10.0]),
            (FocusTarget::Relic(1), [20.0, 0.0, 10.0, 10.0]),
            (FocusTarget::Dora, [200.0, 0.0, 10.0, 10.0]),
        ];
        let mut nav = load_nav(&rects);
        assert_eq!(
            nav.pick(FocusTarget::Relic(0), FocusDir::Right),
            Some(FocusTarget::Relic(1))
        );
        assert_eq!(
            nav.pick(FocusTarget::Relic(1), FocusDir::Right),
            Some(FocusTarget::Dora)
        );
        assert_eq!(nav.pick(FocusTarget::Relic(1), FocusDir::Left), Some(FocusTarget::Relic(0)));
    }

    #[test]
    fn hand_row_wraps_at_ends() {
        let rects = [
            (FocusTarget::HandTile(0), [0.0, 100.0, 10.0, 10.0]),
            (FocusTarget::HandTile(1), [20.0, 100.0, 10.0, 10.0]),
            (FocusTarget::HandTile(2), [40.0, 100.0, 10.0, 10.0]),
        ];
        let mut nav = load_nav(&rects);
        assert_eq!(
            nav.pick(FocusTarget::HandTile(0), FocusDir::Right),
            Some(FocusTarget::HandTile(1))
        );
        assert_eq!(
            nav.pick(FocusTarget::HandTile(1), FocusDir::Right),
            Some(FocusTarget::HandTile(2))
        );
        assert_eq!(
            nav.pick(FocusTarget::HandTile(0), FocusDir::Left),
            Some(FocusTarget::HandTile(2))
        );
        assert_eq!(
            nav.pick(FocusTarget::HandTile(2), FocusDir::Right),
            Some(FocusTarget::HandTile(0))
        );
    }

    #[test]
    fn play_up_picks_hand_tile_by_play_x_not_stale_column() {
        use crate::ui::focus_nav::rect_center;

        let rects = live_gameplay_focus_rects(14);
        let play_cx = rect_center(
            rects
                .iter()
                .find(|(t, _)| *t == FocusTarget::Button(GameplayButton::Play))
                .map(|(_, r)| *r)
                .expect("play rect"),
        )
        .0;

        fn nearest_hand(rects: &[(FocusTarget, [f32; 4])], anchor_x: f32) -> usize {
            rects
                .iter()
                .filter_map(|(t, r)| match t {
                    FocusTarget::HandTile(i) => Some((*i, (rect_center(*r).0 - anchor_x).abs())),
                    _ => None,
                })
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .map(|(i, _)| i)
                .expect("hand tile")
        }

        let mut nav = load_nav_inferred_only(&rects);
        let fresh = nav
            .pick(FocusTarget::Button(GameplayButton::Play), FocusDir::Up)
            .expect("Up from Play (fresh memory)");
        assert_eq!(
            fresh,
            FocusTarget::HandTile(nearest_hand(&rects, play_cx)),
            "fresh Up from Play should pick x-nearest hand tile"
        );

        // Prior vertical move pins desired_x to tile 5; L/R along bottom to Play
        // should refresh the column to Play's x.
        let mut nav = load_nav_inferred_only(&rects);
        let _ = nav.pick(FocusTarget::HandTile(5), FocusDir::Down);
        let mut cur = FocusTarget::Button(GameplayButton::Discard);
        for _ in 0..12 {
            if cur == FocusTarget::Button(GameplayButton::Play) {
                break;
            }
            cur = nav
                .pick(cur, FocusDir::Right)
                .expect("walk bottom strip to Play");
        }
        assert_eq!(cur, FocusTarget::Button(GameplayButton::Play));
        let after_walk = nav
            .pick(cur, FocusDir::Up)
            .expect("Up from Play after L/R walk");
        assert_eq!(
            after_walk,
            FocusTarget::HandTile(nearest_hand(&rects, play_cx)),
            "Up from Play after horizontal walk should use Play's x, not stale desired_x from tile 5"
        );
    }

    #[test]
    fn hand_tiles_zero_one_down_to_discard_not_undo() {
        let mut rects = live_gameplay_focus_rects(14);
        let discard_rect = rects
            .iter()
            .find(|(t, _)| *t == FocusTarget::Button(GameplayButton::Discard))
            .map(|(_, r)| *r)
            .expect("discard rect");
        let gap = 6.0;
        let undo_rect = [
            discard_rect[0],
            discard_rect[1] + discard_rect[3] + gap,
            88.0,
            28.0,
        ];
        rects.push((FocusTarget::DiscardUndo, undo_rect));

        let mut nav = load_nav(&rects);
        for i in 0..=1 {
            assert_eq!(
                nav.pick(FocusTarget::HandTile(i), FocusDir::Down),
                Some(FocusTarget::Button(GameplayButton::Discard)),
                "HandTile({i}) Down should reach Discard, not DiscardUndo"
            );
        }
    }

    #[test]
    fn last_hand_tile_up_prefers_cash_in_when_visible() {
        let rects = live_gameplay_focus_rects(14);
        let mut nav = load_nav(&rects);
        assert_eq!(
            nav.pick(FocusTarget::HandTile(13), FocusDir::Up),
            Some(FocusTarget::Button(GameplayButton::Trigger)),
            "HandTile(13) Up should prioritize visible Cash In"
        );
    }

    #[test]
    fn last_hand_tile_up_falls_back_to_gold_when_cash_in_hidden() {
        let mut rects = live_gameplay_focus_rects(14);
        rects.retain(|(t, _)| !matches!(t, FocusTarget::Button(GameplayButton::Trigger)));
        let mut nav = load_nav(&rects);
        assert_eq!(
            nav.pick(FocusTarget::HandTile(13), FocusDir::Up),
            Some(FocusTarget::Gold),
            "HandTile(13) Up should target Gold when Cash In is absent"
        );
    }
}
