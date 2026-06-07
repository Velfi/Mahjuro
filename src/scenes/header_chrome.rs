//! Shared top-header chrome metrics for scene nav rows.
//!
//! Keeps Back/confirm sizing and vertical band spacing consistent anywhere we
//! use the same title-bar pattern (guide, journal, tile-pickers, etc.).

#[derive(Clone, Copy, Debug)]
pub(crate) struct HeaderChromeMetrics {
    pub(crate) ui: f32,
    pub(crate) margin: f32,
    pub(crate) button_h: f32,
}

impl HeaderChromeMetrics {
    pub(crate) fn from_window(window_w: f32, window_h: f32) -> Self {
        let ui = (window_w / 1920.0).min(window_h / 1080.0).clamp(0.55, 1.35);
        let margin = 48.0 * ui;
        let button_h = (window_h * 0.062).clamp(52.0, 86.0);
        Self {
            ui,
            margin,
            button_h,
        }
    }

    pub(crate) fn back_rect_left(self) -> [f32; 4] {
        let back_w = (126.0 * (self.margin / 48.0)).clamp(102.0, 160.0);
        [self.margin, self.margin, back_w, self.button_h]
    }

    pub(crate) fn right_confirm_rect(self, window_w: f32) -> [f32; 4] {
        let seal_w = (214.0 * (self.margin / 48.0)).clamp(172.0, 256.0);
        [
            window_w - self.margin - seal_w,
            self.margin,
            seal_w,
            self.button_h,
        ]
    }

    pub(crate) fn chrome_bottom(self) -> f32 {
        self.margin + self.button_h + 12.0 * self.ui
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HeaderTitleLayout {
    pub(crate) copy_x: f32,
    pub(crate) title_y: f32,
    pub(crate) subtitle_y: f32,
}

impl HeaderTitleLayout {
    /// Title/subtitle lines vertically centered against the nav-row controls.
    pub(crate) fn nav_row_aligned(
        back_rect: [f32; 4],
        preferred_x: f32,
        copy_gap: f32,
        title_font: f32,
        jr: f32,
    ) -> Self {
        let nav_center_y = back_rect[1] + back_rect[3] * 0.5;
        let title_y = nav_center_y - title_font * 0.58;
        let subtitle_y = title_y + title_font * 0.92 + 2.0 * jr;
        let copy_x = preferred_x.max(back_rect[0] + back_rect[2] + copy_gap);
        Self {
            copy_x,
            title_y,
            subtitle_y,
        }
    }

    /// Top of the body panel below a nav-row subtitle.
    pub(crate) fn body_top_below_subtitle(self, subtitle_font: f32, jr: f32) -> f32 {
        self.subtitle_y + subtitle_font * 1.25 + 12.0 * jr
    }
}
