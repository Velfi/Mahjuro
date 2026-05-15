use super::*;
use crate::render::draw_cmd::UiFrame;

impl ShopScene {
    pub(super) fn push_shop_particle_quads(&self, frame: &mut UiFrame) {
        for (rect, color) in self.particles.instances() {
            frame.quad(GpuInstance { rect, color, user: 0});
        }
    }

    pub(super) fn push_shop_score_popup_labels(&self, frame: &mut UiFrame, w: f32, h: f32) {
        let popup_scale = w.min(h) / 1080.0;
        let now_celeb = Instant::now();
        for lbl in self
            .score_popups
            .overlay_text_labels(now_celeb, w, h, popup_scale)
        {
            frame.text(lbl);
        }
    }
}
