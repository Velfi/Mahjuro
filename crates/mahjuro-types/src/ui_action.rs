//! Device-agnostic UI actions (keyboard / gamepad / mouse routing).

/// Logical UI actions (device-agnostic).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiAction {
    FocusNext,
    FocusPrev,
    FocusDown,
    FocusUp,
    Confirm,
    ConfirmRelease,
    Cancel,
    CancelRelease,
    ScoreHand,
    TriggerStructure,
    /// Release edge for the cash-in hold (gamepad trigger / keyboard **T** /
    /// Confirm on the Cash In button). Cancels an in-progress hold-to-cash-in.
    TriggerStructureRelease,
    CommitDiscard,
    InvertSelection,
    UndoDiscard,
    FocusPlayButton,
    FocusDiscardButton,
    NavigateHudNext,
    NavigateHudPrev,
    TabNext,
    TabPrev,
    PageNext,
    PagePrev,
    Pause,
    Help,
    Delete,
    DebugToggleAxes,
    NorthFacePress,
    WestFacePress,
    WestFaceRelease,
}
