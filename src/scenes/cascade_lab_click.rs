//! Tagged click ids for [`super::CascadeLabScene`].
//!
//! Wire format: `(category << 16) | payload`. Each interactive category owns a
//! distinct high 16 bits, so table slots, panel buttons, and picker rows cannot
//! collide when new controls are added.

use crate::core::relic::all_relic_defs;

/// Cascade Lab `ButtonAction::Scene` click identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabClick {
    Back,
    Prev,
    Next,
    CashIn,
    ResetScore,
    Apply,
    StructureField,
    Save,
    ResetTuning,
    TogglePanel,
    TabTiming,
    TabTable,
    TabState,
    DiscardsDec,
    DiscardsInc,
    PlaysDec,
    PlaysInc,
    YenDec,
    YenInc,
    ToggleScoredLast,
    CounterDec,
    CounterInc,
    PickerClose,
    PickerClear,
    RelicTraySlot(usize),
    DoraSlot(usize),
    RoundWindSlot(usize),
    StructureMeldSlot(usize),
    StructureAdd,
    Boss,
    PickRelicDef(usize),
    PickDoraRow(usize),
    PickWind(u8),
    PickBossRow(usize),
    PickMeldRow(usize),
}

impl LabClick {
    const TAG_PANEL: u32 = 0xE200;
    const TAG_TRAY: u32 = 0xE210;
    const TAG_DORA: u32 = 0xE220;
    const TAG_WIND: u32 = 0xE230;
    const TAG_MELD: u32 = 0xE240;
    const TAG_BOSS: u32 = 0xE250;
    const TAG_MELD_ADD: u32 = 0xE260;
    const TAG_PICKER: u32 = 0xE270;
    const TAG_PICK_RELIC: u32 = 0xE280;
    const TAG_PICK_DORA: u32 = 0xE290;
    const TAG_PICK_WIND: u32 = 0xE2A0;
    const TAG_PICK_BOSS: u32 = 0xE2B0;
    const TAG_PICK_MELD: u32 = 0xE2C0;

    const PANEL_BACK: u32 = 0;
    const PANEL_PREV: u32 = 1;
    const PANEL_NEXT: u32 = 2;
    const PANEL_CASH_IN: u32 = 3;
    const PANEL_RESET_SCORE: u32 = 4;
    const PANEL_APPLY: u32 = 5;
    const PANEL_STRUCTURE_FIELD: u32 = 6;
    const PANEL_SAVE: u32 = 7;
    const PANEL_RESET_TUNING: u32 = 8;
    const PANEL_TOGGLE: u32 = 9;
    const PANEL_TAB_TIMING: u32 = 10;
    const PANEL_TAB_TABLE: u32 = 11;
    const PANEL_TAB_STATE: u32 = 12;
    const PANEL_DISCARDS_DEC: u32 = 13;
    const PANEL_DISCARDS_INC: u32 = 14;
    const PANEL_PLAYS_DEC: u32 = 15;
    const PANEL_PLAYS_INC: u32 = 16;
    const PANEL_YEN_DEC: u32 = 17;
    const PANEL_YEN_INC: u32 = 18;
    const PANEL_SCORED_LAST: u32 = 19;
    const PANEL_COUNTER_DEC: u32 = 20;
    const PANEL_COUNTER_INC: u32 = 21;
    const PICKER_CLOSE: u32 = 0;
    const PICKER_CLEAR: u32 = 1;

    #[inline]
    const fn encode(tag: u32, payload: u32) -> u32 {
        (tag << 16) | (payload & 0xFFFF)
    }

    /// Scene button id for the main-loop hit dispatcher.
    pub fn id(self) -> u32 {
        match self {
            Self::Back => Self::encode(Self::TAG_PANEL, Self::PANEL_BACK),
            Self::Prev => Self::encode(Self::TAG_PANEL, Self::PANEL_PREV),
            Self::Next => Self::encode(Self::TAG_PANEL, Self::PANEL_NEXT),
            Self::CashIn => Self::encode(Self::TAG_PANEL, Self::PANEL_CASH_IN),
            Self::ResetScore => Self::encode(Self::TAG_PANEL, Self::PANEL_RESET_SCORE),
            Self::Apply => Self::encode(Self::TAG_PANEL, Self::PANEL_APPLY),
            Self::StructureField => Self::encode(Self::TAG_PANEL, Self::PANEL_STRUCTURE_FIELD),
            Self::Save => Self::encode(Self::TAG_PANEL, Self::PANEL_SAVE),
            Self::ResetTuning => Self::encode(Self::TAG_PANEL, Self::PANEL_RESET_TUNING),
            Self::TogglePanel => Self::encode(Self::TAG_PANEL, Self::PANEL_TOGGLE),
            Self::TabTiming => Self::encode(Self::TAG_PANEL, Self::PANEL_TAB_TIMING),
            Self::TabTable => Self::encode(Self::TAG_PANEL, Self::PANEL_TAB_TABLE),
            Self::TabState => Self::encode(Self::TAG_PANEL, Self::PANEL_TAB_STATE),
            Self::DiscardsDec => Self::encode(Self::TAG_PANEL, Self::PANEL_DISCARDS_DEC),
            Self::DiscardsInc => Self::encode(Self::TAG_PANEL, Self::PANEL_DISCARDS_INC),
            Self::PlaysDec => Self::encode(Self::TAG_PANEL, Self::PANEL_PLAYS_DEC),
            Self::PlaysInc => Self::encode(Self::TAG_PANEL, Self::PANEL_PLAYS_INC),
            Self::YenDec => Self::encode(Self::TAG_PANEL, Self::PANEL_YEN_DEC),
            Self::YenInc => Self::encode(Self::TAG_PANEL, Self::PANEL_YEN_INC),
            Self::ToggleScoredLast => Self::encode(Self::TAG_PANEL, Self::PANEL_SCORED_LAST),
            Self::CounterDec => Self::encode(Self::TAG_PANEL, Self::PANEL_COUNTER_DEC),
            Self::CounterInc => Self::encode(Self::TAG_PANEL, Self::PANEL_COUNTER_INC),
            Self::PickerClose => Self::encode(Self::TAG_PICKER, Self::PICKER_CLOSE),
            Self::PickerClear => Self::encode(Self::TAG_PICKER, Self::PICKER_CLEAR),
            Self::RelicTraySlot(i) => Self::encode(Self::TAG_TRAY, i as u32),
            Self::DoraSlot(i) => Self::encode(Self::TAG_DORA, i as u32),
            Self::RoundWindSlot(i) => Self::encode(Self::TAG_WIND, i as u32),
            Self::StructureMeldSlot(i) => Self::encode(Self::TAG_MELD, i as u32),
            Self::StructureAdd => Self::encode(Self::TAG_MELD_ADD, 0),
            Self::Boss => Self::encode(Self::TAG_BOSS, 0),
            Self::PickRelicDef(i) => Self::encode(Self::TAG_PICK_RELIC, i as u32),
            Self::PickDoraRow(i) => Self::encode(Self::TAG_PICK_DORA, i as u32),
            Self::PickWind(rank) => Self::encode(Self::TAG_PICK_WIND, rank as u32),
            Self::PickBossRow(i) => Self::encode(Self::TAG_PICK_BOSS, i as u32),
            Self::PickMeldRow(i) => Self::encode(Self::TAG_PICK_MELD, i as u32),
        }
    }

    pub fn from_id(id: u32) -> Option<Self> {
        let tag = id >> 16;
        let payload = id & 0xFFFF;
        match tag {
            Self::TAG_PANEL => Some(match payload {
                Self::PANEL_BACK => Self::Back,
                Self::PANEL_PREV => Self::Prev,
                Self::PANEL_NEXT => Self::Next,
                Self::PANEL_CASH_IN => Self::CashIn,
                Self::PANEL_RESET_SCORE => Self::ResetScore,
                Self::PANEL_APPLY => Self::Apply,
                Self::PANEL_STRUCTURE_FIELD => Self::StructureField,
                Self::PANEL_SAVE => Self::Save,
                Self::PANEL_RESET_TUNING => Self::ResetTuning,
                Self::PANEL_TOGGLE => Self::TogglePanel,
                Self::PANEL_TAB_TIMING => Self::TabTiming,
                Self::PANEL_TAB_TABLE => Self::TabTable,
                Self::PANEL_TAB_STATE => Self::TabState,
                Self::PANEL_DISCARDS_DEC => Self::DiscardsDec,
                Self::PANEL_DISCARDS_INC => Self::DiscardsInc,
                Self::PANEL_PLAYS_DEC => Self::PlaysDec,
                Self::PANEL_PLAYS_INC => Self::PlaysInc,
                Self::PANEL_YEN_DEC => Self::YenDec,
                Self::PANEL_YEN_INC => Self::YenInc,
                Self::PANEL_SCORED_LAST => Self::ToggleScoredLast,
                Self::PANEL_COUNTER_DEC => Self::CounterDec,
                Self::PANEL_COUNTER_INC => Self::CounterInc,
                _ => return None,
            }),
            Self::TAG_TRAY => Some(Self::RelicTraySlot(payload as usize)),
            Self::TAG_DORA => Some(Self::DoraSlot(payload as usize)),
            Self::TAG_WIND => Some(Self::RoundWindSlot(payload as usize)),
            Self::TAG_MELD => Some(Self::StructureMeldSlot(payload as usize)),
            Self::TAG_MELD_ADD if payload == 0 => Some(Self::StructureAdd),
            Self::TAG_BOSS if payload == 0 => Some(Self::Boss),
            Self::TAG_PICKER => Some(match payload {
                Self::PICKER_CLOSE => Self::PickerClose,
                Self::PICKER_CLEAR => Self::PickerClear,
                _ => return None,
            }),
            Self::TAG_PICK_RELIC => {
                let i = payload as usize;
                all_relic_defs().get(i)?;
                Some(Self::PickRelicDef(i))
            }
            Self::TAG_PICK_DORA => Some(Self::PickDoraRow(payload as usize)),
            Self::TAG_PICK_WIND => {
                let rank = payload as u8;
                (1..=4).contains(&rank).then_some(Self::PickWind(rank))
            }
            Self::TAG_PICK_BOSS => Some(Self::PickBossRow(payload as usize)),
            Self::TAG_PICK_MELD => Some(Self::PickMeldRow(payload as usize)),
            _ => None,
        }
    }

    pub fn is_picker(self) -> bool {
        matches!(
            self,
            Self::PickerClose
                | Self::PickerClear
                | Self::PickRelicDef(_)
                | Self::PickDoraRow(_)
                | Self::PickWind(_)
                | Self::PickBossRow(_)
                | Self::PickMeldRow(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::LabClick;

    const PANEL_CLICKS: &[LabClick] = &[
        LabClick::Back,
        LabClick::Prev,
        LabClick::Next,
        LabClick::CashIn,
        LabClick::ResetScore,
        LabClick::Apply,
        LabClick::StructureField,
        LabClick::Save,
        LabClick::ResetTuning,
        LabClick::TogglePanel,
        LabClick::TabTiming,
        LabClick::TabTable,
        LabClick::TabState,
        LabClick::DiscardsDec,
        LabClick::DiscardsInc,
        LabClick::PlaysDec,
        LabClick::PlaysInc,
        LabClick::YenDec,
        LabClick::YenInc,
        LabClick::ToggleScoredLast,
        LabClick::CounterDec,
        LabClick::CounterInc,
        LabClick::PickerClose,
        LabClick::PickerClear,
        LabClick::RelicTraySlot(0),
        LabClick::DoraSlot(1),
        LabClick::RoundWindSlot(0),
        LabClick::StructureMeldSlot(2),
        LabClick::StructureAdd,
        LabClick::Boss,
        LabClick::PickRelicDef(87),
        LabClick::PickRelicDef(31),
        LabClick::PickDoraRow(5),
        LabClick::PickWind(2),
        LabClick::PickBossRow(20),
        LabClick::PickMeldRow(1),
    ];

    #[test]
    fn round_trip_and_unique_ids() {
        let mut seen = std::collections::HashSet::new();
        for &click in PANEL_CLICKS {
            let id = click.id();
            assert!(
                seen.insert(id),
                "duplicate click id 0x{id:08X} for {click:?}"
            );
            assert_eq!(LabClick::from_id(id), Some(click));
        }
    }

    #[test]
    fn pick_relic_def_idx_is_not_table_wind_slot() {
        let beggars = LabClick::PickRelicDef(87).id();
        let wind = LabClick::RoundWindSlot(0).id();
        assert_ne!(beggars, wind);
        assert!(matches!(
            LabClick::from_id(beggars),
            Some(LabClick::PickRelicDef(87))
        ));
    }
}
