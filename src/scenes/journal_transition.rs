//! Shared Yaku Journal click-to-open animation (shop counter + gameplay HUD).
//!
//! ## Book scale (mesh local **X** × **Z** visible face)
//!
//! [`Object3d`](crate::render::draw_cmd::Object3d) extents scale the unit book mesh non-uniformly.
//! The cover face should keep a **fixed** height÷width near **1.5** (typical US trade hardcover
//! 6"×9" ≈ 1.50). Previously height used `h * 0.138` while width used `w * 0.065`, which made the
//! silhouette depend on monitor aspect ratio (ultrawide could look wider than tall).

/// Horizontal span of the closed cover as a fraction of window width (mesh local X, fore-edge).
pub const BOOK_FACE_WIDTH_FRAC: f32 = 0.042;
/// Visible cover height ÷ width — upright portrait book (trade 6×9" class).
pub const BOOK_FACE_HEIGHT_OVER_WIDTH: f32 = 1.52;
/// Spine thickness along mesh local Y (depth), in mm — slim journal (thinner than a thick octavo).
pub const BOOK_SPINE_THICKNESS_MM: f32 = 7.0;

/// `(face_width, face_height)` in layout pixel units, scaled by `zoom`.
#[inline]
pub fn book_cover_face_extents_xy(window_w: f32, zoom: f32) -> (f32, f32) {
    let face_w = window_w * BOOK_FACE_WIDTH_FRAC * zoom;
    let face_h = face_w * BOOK_FACE_HEIGHT_OVER_WIDTH;
    (face_w, face_h)
}

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

/// Routes [`crate::render::draw_cmd::Object3dKind::Book`] picks through `aux_dish_rects` /
/// `last_primitive_pick_models` (shop counter + gameplay action bar).
pub const YAKU_JOURNAL_BOOK_PICK_ID: u32 = 3;
