#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GameOverReason {
    OutOfPlays,
    NoActionsRemaining,
}

impl GameOverReason {
    pub fn loss_summary(self) -> &'static str {
        match self {
            Self::OutOfPlays => "no more plays and missed target",
            Self::NoActionsRemaining => "plays left but no valid melds and no discards left",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GameOverReason;

    #[test]
    fn loss_summaries_name_exact_failure_cause() {
        assert_eq!(
            GameOverReason::OutOfPlays.loss_summary(),
            "no more plays and missed target"
        );
        assert_eq!(
            GameOverReason::NoActionsRemaining.loss_summary(),
            "plays left but no valid melds and no discards left"
        );
    }
}
