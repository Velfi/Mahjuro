//! Shared Yaku Journal click-to-open animation (shop counter + gameplay HUD).
//!
//! Book mesh scale constants live in [`mahjuro_render::scene_glue`].

/// Forward opens toward [`crate::scenes::YakuJournalScene`]; reverse plays after the overlay pops.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JournalDirection {
    Opening,
    Closing,
}

#[derive(Clone, Copy)]
pub struct JournalTransition {
    pub start: std::time::Instant,
    pub dir: JournalDirection,
}

impl JournalTransition {
    pub const COVER_OPEN_DUR: f32 = 0.45;
    pub const TOTAL_DUR: f32 = 1.10;

    pub fn elapsed(self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    pub fn open_progress(self) -> f32 {
        let t = self.elapsed();
        match self.dir {
            JournalDirection::Opening => (t / Self::COVER_OPEN_DUR).clamp(0.0, 1.0),
            JournalDirection::Closing => {
                let reverse_t = (Self::TOTAL_DUR - t).max(0.0);
                (reverse_t / Self::COVER_OPEN_DUR).clamp(0.0, 1.0)
            }
        }
    }

    pub fn zoom_progress(self) -> f32 {
        let t = self.elapsed();
        match self.dir {
            JournalDirection::Opening => {
                if t < Self::COVER_OPEN_DUR {
                    0.0
                } else {
                    ((t - Self::COVER_OPEN_DUR) / (Self::TOTAL_DUR - Self::COVER_OPEN_DUR))
                        .clamp(0.0, 1.0)
                }
            }
            JournalDirection::Closing => {
                let reverse_t = (Self::TOTAL_DUR - t).max(0.0);
                if reverse_t < Self::COVER_OPEN_DUR {
                    0.0
                } else {
                    ((reverse_t - Self::COVER_OPEN_DUR) / (Self::TOTAL_DUR - Self::COVER_OPEN_DUR))
                        .clamp(0.0, 1.0)
                }
            }
        }
    }

    pub fn done(self) -> bool {
        self.elapsed() >= Self::TOTAL_DUR
    }
}
