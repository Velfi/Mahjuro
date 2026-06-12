//! Relic load/decode for the runtime renderer and the offline RLC2 baker.
//!
//! **Runtime** loads pre-baked `data/relic_baked/<slug>.rlc` only (see [`crate::relic_bake`]).
//! Source PNGs under `textures/relics/` are bake-time inputs for `mahjuro-bake-relics` and are
//! not shipped in release asset packs.

use std::sync::mpsc;
use std::time::Instant;

use crate::gpu_types::DecodedRelicImage;
use crate::relic_dish::build_relic_mesh_from_rgba;
use mahjuro_core::core::relic::{RelicId, all_relic_defs};

fn flat_relief_rgba() -> (Vec<u8>, u32, u32) {
    (vec![128, 128, 128, 255], 1, 1)
}

/// Matches `HEIGHT_ALPHA_LO` / specular derivation in `scripts/generate_relic_art.py`.
const HEIGHT_ALPHA_LO: u8 = 8;
const HEIGHT_ALPHA_HI: u8 = 24;
const SPECULAR_ENAMEL: u8 = 36;
const SPECULAR_METAL: u8 = 255;
const SPECULAR_METAL_THRESHOLD: u8 = 200;
const SPECULAR_RAMP_START: u8 = 168;

fn specular_from_height_luma(luma: u8) -> u8 {
    if luma < HEIGHT_ALPHA_LO {
        return 0;
    }
    if luma >= SPECULAR_METAL_THRESHOLD {
        return SPECULAR_METAL;
    }
    if luma >= SPECULAR_RAMP_START {
        let t = (luma - SPECULAR_RAMP_START) as f32
            / (SPECULAR_METAL_THRESHOLD - SPECULAR_RAMP_START).max(1) as f32;
        return (SPECULAR_ENAMEL as f32 + t * (SPECULAR_METAL - SPECULAR_ENAMEL) as f32).round()
            as u8;
    }
    if luma >= HEIGHT_ALPHA_HI {
        let t =
            (luma - HEIGHT_ALPHA_HI) as f32 / (SPECULAR_RAMP_START - HEIGHT_ALPHA_HI).max(1) as f32;
        return (SPECULAR_ENAMEL as f32 * (0.65 + 0.35 * t)).round() as u8;
    }
    SPECULAR_ENAMEL
}

fn height_luma(pixel: &image::Rgba<u8>) -> u8 {
    let r = pixel[0] as u32;
    let g = pixel[1] as u32;
    let b = pixel[2] as u32;
    ((54 * r + 183 * g + 19 * b) / 256) as u8
}

fn pack_relief_rgba(
    height: &image::RgbaImage,
    specular: Option<&image::GrayImage>,
) -> (Vec<u8>, u32, u32) {
    let (w, h) = height.dimensions();
    let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
    for y in 0..h {
        for x in 0..w {
            let hp = height.get_pixel(x, y);
            let height_l = height_luma(hp);
            let spec_l = specular
                .map(|s| {
                    let mx = ((x as u64) * (s.width() as u64) / (w as u64)) as u32;
                    let my = ((y as u64) * (s.height() as u64) / (h as u64)) as u32;
                    s.get_pixel(
                        mx.min(s.width().saturating_sub(1)),
                        my.min(s.height().saturating_sub(1)),
                    )[0]
                })
                .unwrap_or_else(|| specular_from_height_luma(height_l));
            let i = ((y * w + x) * 4) as usize;
            out[i] = height_l;
            out[i + 1] = spec_l;
            out[i + 2] = height_l;
            out[i + 3] = 255;
        }
    }
    (out, w, h)
}

/// Threshold below which a mask pixel's luminance/alpha reads as "outside the
/// silhouette." Matches the convention used for mesh extraction in
/// [`crate::relic_dish::build_relic_mesh_from_rgba`].
const MASK_SILHOUETTE_THRESHOLD: u8 = 115; // ≈ 0.45 * 255

/// Cut `img`'s alpha channel against a silhouette mask. The silhouette is
/// always encoded in the mask's luma (white = inside, black = outside); the
/// alpha channel, if any, is ignored. Pixels outside the silhouette are set
/// to alpha=0; pixels inside keep the object's original alpha. The mask is
/// nearest-neighbour sampled so dimension mismatches are handled without
/// introducing a PIL/image dependency on resampling.
fn apply_mask_alpha(img: &mut image::RgbaImage, mask: &image::RgbaImage) {
    let (iw, ih) = img.dimensions();
    let (mw, mh) = mask.dimensions();
    if mw == 0 || mh == 0 {
        return;
    }
    for y in 0..ih {
        for x in 0..iw {
            let mx = ((x as u64) * (mw as u64) / (iw as u64)) as u32;
            let my = ((y as u64) * (mh as u64) / (ih as u64)) as u32;
            let mp = mask.get_pixel(mx.min(mw - 1), my.min(mh - 1));
            // rec.709 luminance on RGB, integer-math.
            let luma = (54 * mp[0] as u32 + 183 * mp[1] as u32 + 19 * mp[2] as u32) / 256;
            if (luma as u8) < MASK_SILHOUETTE_THRESHOLD {
                let px = img.get_pixel_mut(x, y);
                px[3] = 0;
            }
        }
    }
}

fn load_relic_mask_rgba(id: RelicId) -> Option<image::RgbaImage> {
    let bytes = mahjuro_assets::asset_path::get(&id.source_mask_path())
        .or_else(|| mahjuro_assets::asset_path::get(&id.render_mask_path()))?;
    image::load_from_memory(&bytes.data)
        .ok()
        .map(|i| i.into_rgba8())
}

fn decode_relic_albedo_masked(id: RelicId) -> Option<(image::RgbaImage, Option<image::RgbaImage>)> {
    let mut img = load_relic_albedo_rgba(&id.render_texture_path(), &id.source_object_path())?;
    let mask = load_relic_mask_rgba(id);
    if let Some(ref m) = mask {
        apply_mask_alpha(&mut img, m);
    }
    Some((img, mask))
}

/// Mask-cut relic albedo for 2D [`ImageQuad`](crate::draw_cmd::ImageQuad) icons.
pub(crate) fn decode_relic_icon_rgba(id: RelicId) -> Option<(Vec<u8>, u32, u32)> {
    let msg = if crate::offline_bakes::committed_offline_bakes_required() {
        crate::relic_bake::load_baked_relic(id).unwrap_or_else(|e| panic!("{e:#}"))
    } else {
        crate::relic_bake::load_baked_relic(id).ok()?
    };
    Some((msg.rgba, msg.width, msg.height))
}

fn load_relic_albedo_rgba(primary: &str, object: &str) -> Option<image::RgbaImage> {
    let bytes = mahjuro_assets::asset_path::get(primary)
        .map(|f| f.data.to_vec())
        .or_else(|| mahjuro_assets::asset_path::get(object).map(|f| f.data.to_vec()))?;
    match image::load_from_memory(&bytes) {
        Ok(i) => Some(i.into_rgba8()),
        Err(e) => {
            log::warn!("failed to decode relic icon {primary} / {object}: {e}");
            None
        }
    }
}

/// Decode one relic from source PNGs. Used by `mahjuro-bake-relics` only — not at runtime.
pub fn decode_relic_assets(
    id: RelicId,
    name: &'static str,
) -> Option<(DecodedRelicImage, std::time::Duration)> {
    let (img, mask) = decode_relic_albedo_masked(id)?;
    let (w, h) = img.dimensions();
    let mesh_rgba = mask.map(|rgba| {
        let (mw, mh) = rgba.dimensions();
        (rgba.into_raw(), mw, mh)
    });

    let relief_path = id.source_heightmap_path();
    let specular_path = id.source_specular_path();
    let (relief_rgba, relief_width, relief_height) =
        if let Some(file) = mahjuro_assets::asset_path::get(&relief_path) {
            match image::load_from_memory(&file.data) {
                Ok(himg) => {
                    let height = himg.into_rgba8();
                    let specular =
                        mahjuro_assets::asset_path::get(&specular_path).and_then(|spec_file| {
                            match image::load_from_memory(&spec_file.data) {
                                Ok(sim) => Some(sim.into_luma8()),
                                Err(e) => {
                                    log::warn!(
                                        "failed to decode relic specular map {specular_path}: {e}"
                                    );
                                    None
                                }
                            }
                        });
                    pack_relief_rgba(&height, specular.as_ref())
                }
                Err(e) => {
                    log::warn!("failed to decode relic heightmap {relief_path}: {e}");
                    flat_relief_rgba()
                }
            }
        } else {
            flat_relief_rgba()
        };

    let mesh_source = mesh_rgba
        .as_ref()
        .map(|(rgba, mw, mh)| (rgba.as_slice(), *mw, *mh))
        .unwrap_or((img.as_raw(), w, h));
    let mesh_source_label = if mesh_rgba.is_some() {
        id.source_mask_path()
    } else {
        format!("{} (alpha fallback)", id.render_texture_path())
    };
    let t_mesh = Instant::now();
    let mesh_cpu = build_relic_mesh_from_rgba(
        mesh_source.0,
        mesh_source.1,
        mesh_source.2,
        &mesh_source_label,
    );
    let mesh_build = t_mesh.elapsed();

    Some((
        DecodedRelicImage {
            id,
            name,
            rgba: img.into_raw(),
            width: w,
            height: h,
            relief_rgba,
            relief_width,
            relief_height,
            mesh_cpu,
            albedo_bc7: None,
            relief_bc7: None,
        },
        mesh_build,
    ))
}

/// Background thread: load every relic once, send [`DecodedRelicImage`] to the renderer.
pub(crate) fn spawn_relic_loader() -> mpsc::Receiver<DecodedRelicImage> {
    spawn_relic_bake_loader()
}

fn spawn_relic_bake_loader() -> mpsc::Receiver<DecodedRelicImage> {
    let (tx, rx) = mpsc::channel();
    crate::loader_pool::submit_relic_batch(move || {
        let t_thread = Instant::now();
        let mut decoded = 0usize;
        for d in all_relic_defs() {
            let path = crate::relic_bake::baked_relic_asset_path(d.id);
            match crate::relic_bake::load_baked_relic_uncached(d.id) {
                Ok(msg) => {
                    decoded += 1;
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    if crate::offline_bakes::committed_offline_bakes_required() {
                        panic!("baked relic {path}: {e:#}");
                    }
                    log::error!("baked relic {path}: {e:#}");
                }
            }
        }
        let thread_total = t_thread.elapsed();
        crate::startup_profile::record("relic.decode_thread", thread_total);
        crate::startup_profile::record("relic.decode_cpu", thread_total);
        crate::startup_profile::record("relic.mesh_build_thread", std::time::Duration::ZERO);
        log::debug!("relic-loader (RLC2): loaded {decoded} baked relics in {thread_total:?}",);
    });
    rx
}
