//! Relic PNG decode for the runtime renderer (embedded assets → [`DecodedRelicImage`]).
//!
//! File naming lives on [`RelicId`](crate::core::relic::RelicId):
//! - **Albedo (color):** `textures/relics/<slug>.png`, else `textures/relics/source/<slug>_object.png`
//! - **Mesh silhouette (optional):** `source/<slug>_mask.png`, else `<slug>_mask.png` next to albedo  
//!   Used only for extruded cap geometry; if missing, the albedo image is used (alpha / luma in
//!   [`crate::render::relic_dish::build_relic_mesh_from_rgba`]).
//! - **Relief (linear height):** `source/<slug>_height.png`, or a 1×1 mid-gray placeholder

use std::sync::mpsc;
use std::time::Instant;

use crate::core::relic::{RelicId, all_relic_defs};
use crate::render::gpu_types::DecodedRelicImage;

fn flat_relief_rgba() -> (Vec<u8>, u32, u32) {
    (vec![128, 128, 128, 255], 1, 1)
}

/// Decode one relic from embedded assets. Returns `None` if neither albedo path exists.
pub(crate) fn decode_relic_assets(id: RelicId, name: &'static str) -> Option<DecodedRelicImage> {
    let primary = id.render_texture_path();
    let bytes = crate::asset_path::get(&primary)
        .map(|f| f.data.to_vec())
        .or_else(|| crate::asset_path::get(&id.source_object_path()).map(|f| f.data.to_vec()))?;

    let img = match image::load_from_memory(&bytes) {
        Ok(i) => i.into_rgba8(),
        Err(e) => {
            log::warn!("failed to decode relic art for {:?}: {e}", id);
            return None;
        }
    };
    let (w, h) = img.dimensions();

    let mesh_bytes = crate::asset_path::get(&id.source_mask_path())
        .or_else(|| crate::asset_path::get(&id.render_mask_path()))
        .map(|f| f.data.to_vec());
    let (mesh_rgba, mesh_width, mesh_height) = match mesh_bytes {
        Some(mesh_bytes) => match image::load_from_memory(&mesh_bytes) {
            Ok(img) => {
                let rgba = img.into_rgba8();
                let (mw, mh) = rgba.dimensions();
                (Some(rgba.into_raw()), mw, mh)
            }
            Err(e) => {
                log::warn!("failed to decode relic silhouette for {:?}: {e}", id);
                (None, 0, 0)
            }
        },
        None => (None, 0, 0),
    };

    let relief_path = id.source_heightmap_path();
    let (relief_rgba, relief_width, relief_height) =
        if let Some(file) = crate::asset_path::get(&relief_path) {
            match image::load_from_memory(&file.data) {
                Ok(himg) => {
                    let rgba = himg.into_rgba8();
                    let (rw, rh) = rgba.dimensions();
                    (rgba.into_raw(), rw, rh)
                }
                Err(e) => {
                    log::warn!("failed to decode relic heightmap {relief_path}: {e}");
                    flat_relief_rgba()
                }
            }
        } else {
            flat_relief_rgba()
        };

    Some(DecodedRelicImage {
        id,
        name,
        rgba: img.into_raw(),
        width: w,
        height: h,
        mesh_rgba,
        mesh_width,
        mesh_height,
        relief_rgba,
        relief_width,
        relief_height,
    })
}

/// Background thread: decode every relic once, send [`DecodedRelicImage`] to the renderer.
pub(crate) fn spawn_relic_loader() -> mpsc::Receiver<DecodedRelicImage> {
    let (tx, rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("relic-loader".into())
        .spawn(move || {
            let t_thread = Instant::now();
            let mut decoded = 0usize;
            let mut decode_time = std::time::Duration::ZERO;
            for d in all_relic_defs() {
                let t0 = Instant::now();
                let Some(msg) = decode_relic_assets(d.id, d.name) else {
                    log::warn!(
                        "relic art not found in embedded assets: {} or {}",
                        d.id.render_texture_path(),
                        d.id.source_object_path(),
                    );
                    continue;
                };
                decode_time += t0.elapsed();
                decoded += 1;
                if tx.send(msg).is_err() {
                    break;
                }
            }
            log::info!(
                "relic-loader thread finished: decoded {decoded} images in {decode_time:?} (thread total {:?})",
                t_thread.elapsed(),
            );
        })
        .expect("failed to spawn relic-loader thread");

    rx
}
