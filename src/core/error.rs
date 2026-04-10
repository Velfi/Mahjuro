//! Unified error type for core game logic.
//!
//! Provides a single `GameError` enum that all core modules can propagate
//! through `Result<T, GameError>`. Converts to `anyhow::Error` at boundary
//! layers (persistence, main loop) via the `From` impl that `thiserror`
//! derives automatically.

use std::fmt;

/// Errors originating from core game logic (hand validation, scoring,
/// tile lookup, boss resolution, etc.).
#[allow(dead_code)]
#[derive(Debug)]
pub enum GameError {
    /// A tile selection could not be decomposed into valid sets.
    ValidationFailed { reason: String },
    /// A tile id was expected but not found in the hand or wall.
    TileNotFound { id: u32 },
    /// An operation required resources (gold, plays, discards, consumable
    /// slots) that the player doesn't have.
    InsufficientResources { resource: &'static str },
    /// A game-state invariant was violated (e.g. scoring after game over,
    /// drawing from an empty wall).
    StateViolation { detail: String },
}

impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameError::ValidationFailed { reason } => {
                write!(f, "hand validation failed: {reason}")
            }
            GameError::TileNotFound { id } => {
                write!(f, "tile id {id} not found")
            }
            GameError::InsufficientResources { resource } => {
                write!(f, "insufficient {resource}")
            }
            GameError::StateViolation { detail } => {
                write!(f, "state violation: {detail}")
            }
        }
    }
}

impl std::error::Error for GameError {}

/// Convenience alias used by core game functions.
#[allow(dead_code)]
pub type GameResult<T> = Result<T, GameError>;
