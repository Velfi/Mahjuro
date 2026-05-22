//! Tile-pack opening celebration — phase timing and per-tile progress.

use std::time::Instant;

use crate::core::tile::Tile;
use crate::core::tile_pack::TilePackKind;

/// Pack opening phase machine (see `docs/agents/tpos2-art-direction.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackCelebPhase {
    /// Shooting-star wipe + title fade-in.
    Arrival,
    /// Pack hero: seal glow, breathe, wait for confirm.
    Anticipation,
    /// Short “break seal” punch (~0.55 s); no input.
    Unseal,
    /// Tiles arc out and settle into the reveal row.
    Deal,
}

/// State for [`crate::scenes::showcase::tile_pack::TilePackPresenter`].
pub struct PackCelebration {
    pub tiles: Vec<Tile>,
    pub pack_name: &'static str,
    pub pack_kind: TilePackKind,
    pub phase: PackCelebPhase,
    pub started_at: Instant,
    pub dismissed: bool,
    pub revealed_count: usize,
}

impl PackCelebration {
    pub const UNSEAL_SECS: f32 = 0.55;
    pub const DEAL_STAGGER: f32 = 0.14;
    pub const DEAL_TILE_FLY_SECS: f32 = 0.42;
    pub const SETTLE_SECS: f32 = 0.25;
    pub const ARC_SPLIT: f32 = 0.55;
    pub const FAN_HALF_DEG: f32 = 28.0;
    pub const LAST_TILE_GLOW_SECS: f32 = 0.30;

    pub fn new(tiles: Vec<Tile>, pack_name: &'static str, pack_kind: TilePackKind) -> Self {
        Self {
            tiles,
            pack_name,
            pack_kind,
            phase: PackCelebPhase::Arrival,
            started_at: Instant::now(),
            dismissed: false,
            revealed_count: 0,
        }
    }

    pub fn total_duration(&self) -> f32 {
        let n = self.tiles.len().max(1) as f32;
        (n - 1.0) * Self::DEAL_STAGGER + Self::DEAL_TILE_FLY_SECS + Self::SETTLE_SECS
    }

    pub fn elapsed(&self) -> f32 {
        Instant::now()
            .saturating_duration_since(self.started_at)
            .as_secs_f32()
    }

    pub fn fully_settled(&self) -> bool {
        self.phase == PackCelebPhase::Deal && self.elapsed() >= self.total_duration()
    }

    pub fn unseal_t(&self) -> f32 {
        (self.elapsed() / Self::UNSEAL_SECS).clamp(0.0, 1.0)
    }

    /// Per-tile animation progress in Deal: 0 = not started, 1 = landed.
    pub fn tile_progress(&self, idx: usize) -> f32 {
        debug_assert_eq!(self.phase, PackCelebPhase::Deal);
        let t = self.elapsed() - idx as f32 * Self::DEAL_STAGGER;
        (t / Self::DEAL_TILE_FLY_SECS).clamp(0.0, 1.0)
    }

    pub fn screenshot_reveal_settled(
        tiles: Vec<Tile>,
        pack_name: &'static str,
        pack_kind: TilePackKind,
    ) -> Self {
        let mut s = Self::new(tiles, pack_name, pack_kind);
        s.phase = PackCelebPhase::Deal;
        let dur = s.total_duration();
        s.started_at = Instant::now() - std::time::Duration::from_secs_f32(dur + 0.5);
        s.revealed_count = s.tiles.len();
        s
    }
}
