#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GameOverReason {
    OutOfPlays,
    NoActionsRemaining,
}

impl GameOverReason {
    pub fn loss_summary(self) -> &'static str {
        match self {
            Self::OutOfPlays => "No plays remaining",
            Self::NoActionsRemaining => "No legal actions remained",
        }
    }
}
