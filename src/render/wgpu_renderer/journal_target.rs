//! Journal-page render-to-texture support.
//!
//! Owns the CPU-side rasterizer that paints content into
//! `WgpuRenderer::journal_page_texture`, the offscreen RGBA8 sRGB target
//! the shop's open-book mesh samples as its page-spread albedo (via the
//! leather shader's `uv.x > 3.5` page-content sentinel).
//!
//! ## Why CPU rasterization
//!
//! A GPU side render pipeline (rendering into the journal target as a
//! second offscreen pass with its own depth, instance pools, and
//! globals bind group) was the original plan, but it requires hoisting
//! a lot of `WgpuRenderer::render()`'s inline closures (`make_text_draw`,
//! per-frame buffer allocation, op dispatch) into reusable helpers. That
//! refactor is high-risk for the gameplay rendering path.
//!
//! At 1024×1024 RGBA, CPU rasterization is ~4 MB per frame uploaded via
//! `queue.write_texture` — well under any plausible bandwidth budget on
//! modern hardware. The visual result on the open book is identical to
//! a GPU side path. We trade some CPU time per frame for vastly simpler
//! code and zero risk to the main renderer.

use super::WgpuRenderer;

pub(super) const JOURNAL_TARGET_SIZE: u32 = 1024;

/// Background colour of the page surface (warm vellum). Linear sRGB.
const VELLUM: [u8; 3] = [0xee, 0xe1, 0xc4];
/// Darker band along the spine gutter (centre vertical strip).
const GUTTER: [u8; 3] = [0xc4, 0xa9, 0x7a];
/// Yaku card chrome.
const CARD_BG: [u8; 3] = [0xfb, 0xf5, 0xe4];
const CARD_BORDER: [u8; 3] = [0x6b, 0x4a, 0x28];
/// Title ink — deep oxblood so the journal title pops against vellum.
const TITLE_INK: [u8; 3] = [0x6a, 0x1d, 0x12];
/// Yaku card label ink.
const CARD_INK: [u8; 3] = [0x35, 0x21, 0x10];

impl WgpuRenderer {
    /// Rasterise the yaku-journal page content to CPU memory and upload
    /// it to the journal-page render target. Called once per frame from
    /// `render()`; eventually we'd skip the call when the book is fully
    /// closed, but for now it's cheap enough to always run.
    pub(super) fn upload_journal_page_content(&self) {
        let n = JOURNAL_TARGET_SIZE as usize;
        let mut rgba = vec![0u8; n * n * 4];

        // ── Page background: vellum tint, slightly warmer at the bottom.
        for y in 0..n {
            let v = y as f32 / (n - 1) as f32;
            let warm_r = (VELLUM[0] as f32 * (1.0 - 0.04 * v)) as u8;
            let warm_g = (VELLUM[1] as f32 * (1.0 - 0.06 * v)) as u8;
            let warm_b = (VELLUM[2] as f32 * (1.0 - 0.10 * v)) as u8;
            for x in 0..n {
                let i = (y * n + x) * 4;
                rgba[i] = warm_r;
                rgba[i + 1] = warm_g;
                rgba[i + 2] = warm_b;
                rgba[i + 3] = 0xff;
            }
        }

        // ── Spine gutter: 12-px-wide darkening band down the centre.
        let gutter_x = n / 2;
        for y in 0..n {
            for x in (gutter_x.saturating_sub(6))..(gutter_x + 6).min(n) {
                let i = (y * n + x) * 4;
                rgba[i] = GUTTER[0];
                rgba[i + 1] = GUTTER[1];
                rgba[i + 2] = GUTTER[2];
            }
        }

        // ── Title at the top of the spread.
        if let Some(font) = self.ui_font.as_ref() {
            let title = "Yaku Journal";
            let title_band = crate::render::decal::rasterize_label(font, title, n as u32 - 80, 96);
            blit_tinted(
                &title_band,
                n as u32 - 80,
                96,
                &mut rgba,
                n,
                40,
                28,
                TITLE_INK,
            );
        }

        // ── 13 yaku cards laid out in a 5/4/4 grid.
        let yakus = crate::core::yaku::YakuKind::all();
        let row_counts = [5_usize, 4, 4];
        let card_w_full = (n - 80) as i32;
        // Row layout: top = 5 cards, middle = 4, bottom = 4. Reserve
        // some vertical space at the top for the title.
        let grid_top = 160_i32;
        let grid_h = (n as i32) - grid_top - 60;
        let row_h = grid_h / row_counts.len() as i32;
        let card_h = row_h - 16; // gap between rows

        let mut yaku_idx = 0_usize;
        for (row_i, &count) in row_counts.iter().enumerate() {
            let card_w_with_gap = card_w_full / count as i32;
            let card_w = card_w_with_gap - 12;
            let row_y = grid_top + row_i as i32 * row_h;
            let row_x_origin = 40;
            for col in 0..count {
                let card_x = row_x_origin + col as i32 * card_w_with_gap + 6;
                draw_filled_rect(&mut rgba, n, card_x, row_y, card_w, card_h, CARD_BG);
                draw_rect_outline(&mut rgba, n, card_x, row_y, card_w, card_h, 2, CARD_BORDER);
                if let (Some(font), Some(yaku)) = (self.ui_font.as_ref(), yakus.get(yaku_idx)) {
                    let label = yaku.name();
                    let band = crate::render::decal::rasterize_label(
                        font,
                        label,
                        (card_w - 16) as u32,
                        (card_h - 16) as u32,
                    );
                    blit_tinted(
                        &band,
                        (card_w - 16) as u32,
                        (card_h - 16) as u32,
                        &mut rgba,
                        n,
                        card_x + 8,
                        row_y + 8,
                        CARD_INK,
                    );
                }
                yaku_idx += 1;
            }
        }

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.journal_page_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(JOURNAL_TARGET_SIZE * 4),
                rows_per_image: Some(JOURNAL_TARGET_SIZE),
            },
            wgpu::Extent3d {
                width: JOURNAL_TARGET_SIZE,
                height: JOURNAL_TARGET_SIZE,
                depth_or_array_layers: 1,
            },
        );
    }
}

fn draw_filled_rect(rgba: &mut [u8], stride: usize, x: i32, y: i32, w: i32, h: i32, rgb: [u8; 3]) {
    let n = stride as i32;
    for py in y.max(0)..(y + h).min(n) {
        for px in x.max(0)..(x + w).min(n) {
            let i = (py as usize * stride + px as usize) * 4;
            rgba[i] = rgb[0];
            rgba[i + 1] = rgb[1];
            rgba[i + 2] = rgb[2];
            rgba[i + 3] = 0xff;
        }
    }
}

fn draw_rect_outline(
    rgba: &mut [u8],
    stride: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    thick: i32,
    rgb: [u8; 3],
) {
    draw_filled_rect(rgba, stride, x, y, w, thick, rgb);
    draw_filled_rect(rgba, stride, x, y + h - thick, w, thick, rgb);
    draw_filled_rect(rgba, stride, x, y, thick, h, rgb);
    draw_filled_rect(rgba, stride, x + w - thick, y, thick, h, rgb);
}

/// Blit an RGBA8 source band onto `dst` at `(dx, dy)`, treating the
/// source's R channel as a coverage mask and tinting it with `rgb`.
/// Used to lay rasterised text glyphs onto the page in a chosen ink
/// colour.
fn blit_tinted(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst: &mut [u8],
    dst_stride: usize,
    dx: i32,
    dy: i32,
    rgb: [u8; 3],
) {
    let stride_n = dst_stride as i32;
    for sy in 0..src_h as i32 {
        let py = dy + sy;
        if py < 0 || py >= stride_n {
            continue;
        }
        for sx in 0..src_w as i32 {
            let px = dx + sx;
            if px < 0 || px >= stride_n {
                continue;
            }
            let si = (sy as usize * src_w as usize + sx as usize) * 4;
            // fontdue rasterisers store coverage in all channels; just
            // read R as the alpha mask.
            let cov = src[si] as u16;
            if cov == 0 {
                continue;
            }
            let di = (py as usize * dst_stride + px as usize) * 4;
            // Premultiplied blend: dst = lerp(dst, ink, cov / 255).
            for c in 0..3 {
                let bg = dst[di + c] as u16;
                let ink = rgb[c] as u16;
                dst[di + c] = ((bg * (255 - cov) + ink * cov) / 255) as u8;
            }
        }
    }
}
