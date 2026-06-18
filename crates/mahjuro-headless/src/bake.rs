//! Offline room GI / shadow bakes.
//!
//! On a successful full-room run with default output dirs, refreshes
//! `.inputs_stamp` for each kind that was baked so `mahjuro`'s `build.rs` won't
//! flag the committed bake as stale next compile.

use std::path::{Path, PathBuf};

use mahjuro_bake_stamp::BakeKind;
use mahjuro_bake_stamp::room_gi::RoomGi;
use mahjuro_bake_stamp::room_shadow::RoomShadow;

use crate::bake_cli::{BakeRoomCli, RoomBakeKind};
use crate::room_bake::scene_for_room;
use crate::room_bake::{bake_player_progress, bake_render_settings};
use crate::room_bake_app::RoomBakeApp;
use crate::slug::{resolve_lightmap_bake_rooms, resolve_shadow_bake_rooms};

const EXPECTED_GI_STAMP_HASH_ENV: &str = "MAHJURO_EXPECT_ROOM_GI_STAMP_HASH";
const EXPECTED_SHADOW_STAMP_HASH_ENV: &str = "MAHJURO_EXPECT_ROOM_SHADOW_STAMP_HASH";

pub fn run(cli: BakeRoomCli) -> anyhow::Result<()> {
    mahjuro::asset_path::init();
    mahjuro::asset_path::log_all_assets();

    let bake_lightmap = cli.kinds.contains(&RoomBakeKind::Lightmap);
    let bake_shadow = cli.kinds.contains(&RoomBakeKind::Shadow);
    anyhow::ensure!(
        bake_lightmap || bake_shadow,
        "no bake kinds selected (use --kinds lightmap,shadow)"
    );
    let lightmap_rooms = if bake_lightmap {
        resolve_lightmap_bake_rooms(&cli.rooms)?
    } else {
        Vec::new()
    };
    let shadow_rooms = if bake_shadow {
        resolve_shadow_bake_rooms(&cli.rooms)?
    } else {
        Vec::new()
    };
    anyhow::ensure!(
        !bake_shadow || !shadow_rooms.is_empty() || bake_lightmap,
        "no room shadow bake targets selected"
    );

    let progress = bake_player_progress();
    let width = cli.width.max(1);
    let height = cli.height.max(1);
    let scene_look = bake_lightmap.then(mahjuro::game::scene_look_tuning::SceneLookTuningSet::load);

    for room in &lightmap_rooms {
        let look = scene_look
            .as_ref()
            .expect("lightmap bake initializes scene look")
            .resolve(Some(room.scene_key()));
        let bake = mahjuro::render::room_gi_bake::bake_room_gi_lightmap_gpu(
            mahjuro::render::room_gi_bake::RoomGiGpuBakeParams {
                room: *room,
                bake_width: width,
                bake_height: height,
                lighting: look.room,
                height_scale: look.room_gltf_height_scale,
            },
            cli.lightmap_size,
        )
        .map_err(|e| {
            anyhow::anyhow!(
                "room GI lightmap GPU bake failed for {:?}; rebake with \
                 `cargo run -p mahjuro-headless --bin mahjuro-bake --features bake -- --kinds lightmap`: {e:#}",
                room
            )
        })?;
        write_room_gi_lightmap_bake(*room, &cli.lightmap_dir, bake)?;
    }

    let mut shadow_app: Option<RoomBakeApp> = None;
    for room in &shadow_rooms {
        let (scene, run, game_in_progress) = scene_for_room(*room, &progress);
        let app = match shadow_app.as_mut() {
            Some(app) => {
                app.reset_scene(scene, run, game_in_progress);
                app
            }
            None => shadow_app.insert(RoomBakeApp::new(
                scene,
                run,
                width,
                height,
                game_in_progress,
                0,
                progress.clone(),
                bake_render_settings(),
            )?),
        };
        let bake = app.bake_room_shadow(*room, cli.warmup_frames)?;
        write_room_shadow_bake(*room, &cli.shadow_dir, bake)?;
    }

    refresh_stamps_if_canonical(&cli, bake_lightmap, bake_shadow)?;
    Ok(())
}

/// Stamp refresh runs only when the user took the default `--lightmap-dir` / `--shadow-dir`
/// AND let the bake cover every room. Anything else is treated as an ad-hoc run that
/// must not poison the committed stamp; we'd rather have `build.rs` panic next time
/// than silently mark a partial bake as authoritative.
fn refresh_stamps_if_canonical(
    cli: &BakeRoomCli,
    bake_lightmap: bool,
    bake_shadow: bool,
) -> anyhow::Result<()> {
    let baked_all_rooms = cli.rooms.is_empty();
    if !baked_all_rooms {
        log::info!("partial room set baked; leaving .inputs_stamp files alone");
        return Ok(());
    }

    let repo = repo_root()?;

    if bake_lightmap {
        if cli.lightmap_dir == Path::new(RoomGi::OUT_DIR) {
            if let Some(expected_hash) = expected_stamp_hash_from_env(EXPECTED_GI_STAMP_HASH_ENV) {
                let stamp_path = repo.join(RoomGi::STAMP_PATH);
                mahjuro_bake_stamp::write_stamp_line(&stamp_path, &expected_hash)?;
                log::info!("refreshed {} ({})", stamp_path.display(), expected_hash);
            } else {
                let stamped = RoomGi::write_stamp(&repo)?;
                log::info!(
                    "refreshed {} ({})",
                    stamped.stamp_path.display(),
                    stamped.hash
                );
            }
        } else {
            log::info!(
                "--lightmap-dir is non-canonical ({}); leaving {} alone",
                cli.lightmap_dir.display(),
                RoomGi::STAMP_PATH
            );
        }
    }

    if bake_shadow {
        if cli.shadow_dir == Path::new(RoomShadow::OUT_DIR) {
            if let Some(expected_hash) =
                expected_stamp_hash_from_env(EXPECTED_SHADOW_STAMP_HASH_ENV)
            {
                let stamp_path = repo.join(RoomShadow::STAMP_PATH);
                mahjuro_bake_stamp::write_stamp_line(&stamp_path, &expected_hash)?;
                log::info!("refreshed {} ({})", stamp_path.display(), expected_hash);
            } else {
                let stamped = RoomShadow::write_stamp(&repo)?;
                log::info!(
                    "refreshed {} ({})",
                    stamped.stamp_path.display(),
                    stamped.hash
                );
            }
        } else {
            log::info!(
                "--shadow-dir is non-canonical ({}); leaving {} alone",
                cli.shadow_dir.display(),
                RoomShadow::STAMP_PATH
            );
        }
    }

    Ok(())
}

fn expected_stamp_hash_from_env(var: &str) -> Option<String> {
    let raw = std::env::var(var).ok()?;
    let hash = raw.trim();
    if hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(hash.to_ascii_lowercase())
    } else {
        log::warn!("ignoring invalid {} value: {:?}", var, raw);
        None
    }
}

/// Repo root with no `..` components. The build script uses `mahjuro`'s
/// `CARGO_MANIFEST_DIR` (already canonical); we walk parents rather than
/// `join("../..")` because `Fnv64::write_path_key` hashes the literal path
/// string, so any `..` would silently desync from `build.rs`'s digest.
fn repo_root() -> anyhow::Result<PathBuf> {
    if let Some(assets) = std::env::var_os("MAHJURO_ASSETS") {
        let assets = PathBuf::from(assets);
        return assets
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow::anyhow!("MAHJURO_ASSETS has no parent (need repo root)"));
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("CARGO_MANIFEST_DIR has no grandparent"))
}

fn write_room_shadow_bake(
    room: mahjuro::render::room_gi_bake::RoomGiRoom,
    output_dir: &PathBuf,
    bake: mahjuro::render::room_shadow_bake::RoomShadowBake,
) -> anyhow::Result<()> {
    let raw = bake.encode();
    let out_name = room.offline_bake_filename("msh.zst");
    let out_path = output_dir.join(out_name);
    ensure_parent_dir(&out_path)?;
    let compressed = zstd::encode_all(raw.as_slice(), 3)?;
    let nbytes = compressed.len();
    std::fs::write(&out_path, &compressed)?;
    mahjuro::asset_path::refresh_cached(&format!("{}.zst", room.shadow_asset_path()), compressed);
    log::info!(
        "room shadow bake {:?} → {} ({}×{}, {} bytes zstd)",
        room,
        out_path.display(),
        bake.width,
        bake.height,
        nbytes
    );
    Ok(())
}

fn write_room_gi_lightmap_bake(
    room: mahjuro::render::room_gi_bake::RoomGiRoom,
    output_dir: &PathBuf,
    bake: mahjuro::render::room_gi_bake::RoomGiLightmapBake,
) -> anyhow::Result<()> {
    let raw = bake.encode_rgba32f_texture()?;
    let hdr_name = room.offline_bake_filename("lightmap.rlm.zst");
    let hdr_path = output_dir.join(hdr_name);
    ensure_parent_dir(&hdr_path)?;
    let compressed = zstd::encode_all(raw.as_slice(), 3)?;
    let compressed_len = compressed.len();
    std::fs::write(&hdr_path, &compressed)?;
    mahjuro::asset_path::refresh_cached(&format!("{}.zst", room.lightmap_asset_path()), compressed);

    let preview_name = room.offline_bake_filename("lightmap.png");
    let preview_path = output_dir.join(preview_name);
    ensure_parent_dir(&preview_path)?;
    let mut rgba = vec![0u8; (bake.width as usize) * (bake.height as usize) * 4];
    for (i, px) in bake.pixels_rgba_f32.chunks_exact(4).enumerate() {
        let a = px[3].clamp(0.0, 1.0);
        let mapped = if a > 0.0 {
            [
                linear_hdr_to_srgb8(px[0]),
                linear_hdr_to_srgb8(px[1]),
                linear_hdr_to_srgb8(px[2]),
                255,
            ]
        } else {
            [0, 0, 0, 0]
        };
        rgba[i * 4..i * 4 + 4].copy_from_slice(&mapped);
    }
    let img = image::RgbaImage::from_raw(bake.width, bake.height, rgba)
        .ok_or_else(|| anyhow::anyhow!("room GI lightmap PNG dimensions overflow"))?;
    img.save(&preview_path)?;
    log::info!(
        "room GI lightmap bake {:?} → {} + {} ({}×{}, {} bytes zstd)",
        room,
        hdr_path.display(),
        preview_path.display(),
        bake.width,
        bake.height,
        compressed_len
    );
    Ok(())
}

fn linear_hdr_to_srgb8(v: f32) -> u8 {
    let mapped = if v.is_finite() && v > 0.0 {
        v / (1.0 + v)
    } else {
        0.0
    };
    let srgb = if mapped <= 0.0031308 {
        mapped * 12.92
    } else {
        1.055 * mapped.powf(1.0 / 2.4) - 0.055
    };
    (srgb.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn ensure_parent_dir(path: &PathBuf) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}
