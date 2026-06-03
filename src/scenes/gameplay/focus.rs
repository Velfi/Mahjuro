use crate::ui::input::UiAction;

/// Bottom-row gameplay buttons. Order in `ALL_BUTTONS` is the keyboard nav order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PegKind {
    Hands,
    Discards,
}

/// Every gameplay HUD element that the keyboard / controller / cursor can
/// "select". Spatial 2D navigation chooses the next target by nearest-in-
/// direction over the on-screen rect for each variant. Display-only variants
/// (relics, pegs, gold, yaku, dora) are still focusable so the player can
/// read tooltips for them via the keyboard, even though `Confirm` is a
/// no-op for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

pub(super) const ALL_BUTTONS: [GameplayButton; 3] = [
    GameplayButton::Discard,
    GameplayButton::Play,
    GameplayButton::Trigger,
];

/// When focus is on the rightmost relic, RIGHT jumps to the dora plinth
/// instead of falling through to spatial pick (which often lands on pegs
/// or gold below the relic row).
pub(super) fn relic_to_dora_focus(
    current: Option<FocusTarget>,
    dir: crate::ui::focus_nav::FocusDir,
    focus_rects: &[(FocusTarget, [f32; 4])],
) -> Option<FocusTarget> {
    let FocusTarget::Relic(current_idx) = current? else {
        return None;
    };
    if !matches!(dir, crate::ui::focus_nav::FocusDir::Right) {
        return None;
    }

    let mut relic_indices = focus_rects.iter().filter_map(|(target, _)| match *target {
        FocusTarget::Relic(i) => Some(i),
        _ => None,
    });
    let last_idx = relic_indices.next_back()?;

    if current_idx != last_idx {
        return None;
    }

    focus_rects
        .iter()
        .find_map(|(target, _)| matches!(target, FocusTarget::Dora).then_some(*target))
}

pub(super) fn wrap_hand_tile_focus(
    current: Option<FocusTarget>,
    dir: crate::ui::focus_nav::FocusDir,
    focus_rects: &[(FocusTarget, [f32; 4])],
) -> Option<FocusTarget> {
    let FocusTarget::HandTile(current_idx) = current? else {
        return None;
    };
    if !matches!(
        dir,
        crate::ui::focus_nav::FocusDir::Left | crate::ui::focus_nav::FocusDir::Right
    ) {
        return None;
    }

    let mut hand_indices = focus_rects.iter().filter_map(|(target, _)| match *target {
        FocusTarget::HandTile(i) => Some(i),
        _ => None,
    });
    let first_idx = hand_indices.next()?;
    let last_idx = hand_indices.next_back().unwrap_or(first_idx);

    match dir {
        crate::ui::focus_nav::FocusDir::Left if current_idx == first_idx => {
            Some(FocusTarget::HandTile(last_idx))
        }
        crate::ui::focus_nav::FocusDir::Right if current_idx == last_idx => {
            Some(FocusTarget::HandTile(first_idx))
        }
        _ => None,
    }
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
    use crate::sfx_id::SfxId;
    use crate::game::event_bus::GameEvent;
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
        FocusTarget::DiscardUndo | FocusTarget::Journal | FocusTarget::Guidebook | FocusTarget::WallHud => {
            Some(FocusKind::Button)
        }
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::focus_nav::FocusDir;

    #[test]
    fn relic_to_dora_only_from_last_relic_right() {
        let rects = [
            (FocusTarget::Relic(0), [0.0, 0.0, 10.0, 10.0]),
            (FocusTarget::Relic(1), [20.0, 0.0, 10.0, 10.0]),
            (FocusTarget::Dora, [200.0, 0.0, 10.0, 10.0]),
        ];
        assert_eq!(
            relic_to_dora_focus(Some(FocusTarget::Relic(0)), FocusDir::Right, &rects),
            None
        );
        assert_eq!(
            relic_to_dora_focus(Some(FocusTarget::Relic(1)), FocusDir::Right, &rects),
            Some(FocusTarget::Dora)
        );
        assert_eq!(
            relic_to_dora_focus(Some(FocusTarget::Relic(1)), FocusDir::Left, &rects),
            None
        );
    }
}
