//! Stash focus rects from draw; read them back in update.
//!
//! Scenes that project rects during draw and consume them one frame later use this
//! instead of duplicating `RefCell<Vec<(T,[f32;4])>>` wiring.

use std::cell::RefCell;

/// Draw-stashed rect list for one-frame update passes.
pub struct RectFocusSession<T: Copy + PartialEq> {
    rects: RefCell<Vec<(T, [f32; 4])>>,
}

impl<T: Copy + PartialEq> Default for RectFocusSession<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + PartialEq> RectFocusSession<T> {
    pub fn new() -> Self {
        Self {
            rects: RefCell::new(Vec::new()),
        }
    }

    /// Replace the cached rect list (typically from draw).
    pub fn stash(&self, candidates: Vec<(T, [f32; 4])>) {
        *self.rects.borrow_mut() = candidates;
    }

    /// Clone of the stashed rects for one-frame update passes.
    pub fn rects(&self) -> Vec<(T, [f32; 4])> {
        self.rects.borrow().clone()
    }
}
