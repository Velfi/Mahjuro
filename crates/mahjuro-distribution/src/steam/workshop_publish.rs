//! In-game Steam Workshop upload for local tileset mods.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use image::imageops::FilterType;
use mahjuro_assets::tileset_mod::{
    list_mod_tilesets, mod_tilesets_root, read_mod_workshop_id, validate_mod_tileset,
    write_mod_workshop_id,
};
use steamworks::{
    AppId, Client, FileType, PublishedFileId, PublishedFileVisibility, SteamError, UpdateStatus,
    UpdateWatchHandle,
};

use super::workshop::steam_client;
use super::MAHJURO_APP_ID;

/// Steam rejects workshop preview files ≥ 1 MiB (`k_EResultLimitExceeded`).
const WORKSHOP_PREVIEW_MAX_BYTES: u64 = 1024 * 1024;
const WORKSHOP_PREVIEW_CACHE: &str = ".workshop_preview.png";

fn format_upload_error(err: SteamError) -> String {
    match err {
        SteamError::LimitExceeded => {
            "Workshop upload failed: preview must be under 1 MiB (fixed by the game) \
             or your Steam Cloud quota for Mahjuro is full — free space in Steam Cloud settings \
             and ensure the app has a non-zero cloud quota on the partner site"
                .into()
        }
        other => format!("Workshop upload failed: {other:?}"),
    }
}

/// Workshop listing thumbnail. Uses `atlas.png` when small enough; otherwise writes
/// `.workshop_preview.png` beside the mod (content upload still uses full `atlas.png`).
fn resolve_workshop_preview(mod_dir: &Path, atlas_png: &Path) -> Result<PathBuf, String> {
    let atlas_len = fs::metadata(atlas_png)
        .map_err(|e| format!("stat atlas.png: {e}"))?
        .len();
    if atlas_len <= WORKSHOP_PREVIEW_MAX_BYTES {
        return Ok(atlas_png.to_path_buf());
    }

    let preview_path = mod_dir.join(WORKSHOP_PREVIEW_CACHE);
    let mut img = image::open(atlas_png).map_err(|e| format!("decode atlas.png: {e}"))?;
    let mut target_w = 512u32.min(img.width()).max(1);
    let mut target_h = ((img.height() as f64) * (target_w as f64) / (img.width() as f64))
        .round()
        .max(1.0) as u32;

    for _ in 0..6 {
        let scaled = if target_w == img.width() && target_h == img.height() {
            img.clone()
        } else {
            image::DynamicImage::ImageRgba8(image::imageops::resize(
                &img,
                target_w,
                target_h,
                FilterType::Lanczos3,
            ))
        };
        scaled
            .save(&preview_path)
            .map_err(|e| format!("write {WORKSHOP_PREVIEW_CACHE}: {e}"))?;
        let preview_len = fs::metadata(&preview_path)
            .map_err(|e| format!("stat {WORKSHOP_PREVIEW_CACHE}: {e}"))?
            .len();
        if preview_len <= WORKSHOP_PREVIEW_MAX_BYTES {
            log::info!(
                "Workshop preview: atlas.png is {atlas_len} bytes; using {WORKSHOP_PREVIEW_CACHE} \
                 ({preview_len} bytes, {target_w}x{target_h})"
            );
            return Ok(preview_path);
        }
        target_w = (target_w * 3 / 4).max(128);
        target_h = (target_h * 3 / 4).max(96);
        img = scaled;
    }

    Err(format!(
        "could not shrink atlas.png ({atlas_len} bytes) to a workshop preview under 1 MiB"
    ))
}

#[derive(Clone, Debug)]
pub struct WorkshopPublishResult {
    pub file_id: u64,
    pub updated: bool,
    pub needs_legal_agreement: bool,
}

struct PublisherState {
    phase: Phase,
    watch: Option<UpdateWatchHandle>,
    folder_name: String,
    progress_label: Option<String>,
}

enum Phase {
    Idle,
    Creating,
    /// Upload queued; [`maybe_start_pending_upload`] runs on the next tick (never from a Steam callback).
    PendingUpload {
        file_id: PublishedFileId,
        updated: bool,
    },
    Uploading,
    Finished(Option<Result<WorkshopPublishResult, String>>),
}

static PUBLISHER: OnceLock<Mutex<PublisherState>> = OnceLock::new();

fn publisher() -> &'static Mutex<PublisherState> {
    PUBLISHER.get_or_init(|| {
        Mutex::new(PublisherState {
            phase: Phase::Idle,
            watch: None,
            folder_name: String::new(),
            progress_label: None,
        })
    })
}

fn mod_title(folder_name: &str) -> String {
    folder_name.replace('_', " ")
}

fn mod_paths(folder_name: &str) -> Result<(PathBuf, PathBuf), String> {
    let dir = mod_tilesets_root().join(folder_name);
    validate_mod_tileset(&dir).map_err(|e| format!("{e}"))?;
    let atlas = dir.join("atlas.png");
    if !atlas.is_file() {
        return Err("mod is missing atlas.png (required as Workshop preview)".into());
    }
    let preview = resolve_workshop_preview(&dir, &atlas)?;
    Ok((dir, preview))
}

fn set_finished(result: Result<WorkshopPublishResult, String>) {
    let Ok(mut state) = publisher().lock() else {
        return;
    };
    state.watch = None;
    state.progress_label = None;
    state.phase = Phase::Finished(Some(result));
}

fn begin_upload(
    client: Arc<Client>,
    file_id: PublishedFileId,
    folder_name: &str,
    updated: bool,
) -> Result<(), String> {
    let (content_dir, preview) = mod_paths(folder_name)?;
    let title = mod_title(folder_name);
    let description = "A custom mahjong tileset for Mahjuro. Requires atlas.toml and atlas.png.";
    let folder_for_cb = folder_name.to_string();
    let watch = client
        .ugc()
        .start_item_update(AppId(MAHJURO_APP_ID), file_id)
        .title(&title)
        .description(description)
        .content_path(&content_dir)
        .preview_path(&preview)
        .visibility(PublishedFileVisibility::Public)
        .tags(vec!["Tileset"], false)
        .submit(None, move |result| match result {
            Ok((id, needs_legal)) => {
                if let Err(e) = write_mod_workshop_id(&folder_for_cb, id.0) {
                    log::warn!("Workshop upload ok but failed to save .workshop_id: {e}");
                }
                set_finished(Ok(WorkshopPublishResult {
                    file_id: id.0,
                    updated,
                    needs_legal_agreement: needs_legal,
                }));
            }
            Err(err) => set_finished(Err(format_upload_error(err))),
        });

    let mut state = publisher()
        .lock()
        .map_err(|_| "Workshop publisher lock poisoned".to_string())?;
    state.watch = Some(watch);
    state.phase = Phase::Uploading;
    state.progress_label = Some("Starting upload…".into());
    Ok(())
}

fn queue_pending_upload(file_id: PublishedFileId, updated: bool) -> Result<(), String> {
    let mut state = publisher()
        .lock()
        .map_err(|_| "Workshop publisher lock poisoned".to_string())?;
    if !matches!(state.phase, Phase::Creating) {
        return Err("Workshop upload state changed unexpectedly".into());
    }
    state.phase = Phase::PendingUpload { file_id, updated };
    state.progress_label = Some("Preparing upload…".into());
    Ok(())
}

/// Must run outside Steam `run_callbacks` — `submit()` deadlocks if called from a call-result handler.
fn maybe_start_pending_upload() {
    let Some(client) = steam_client() else {
        set_finished(Err("Steam disconnected during Workshop upload".into()));
        return;
    };
    let pending = {
        let Ok(state) = publisher().lock() else {
            return;
        };
        let Phase::PendingUpload { file_id, updated } = state.phase else {
            return;
        };
        (file_id, state.folder_name.clone(), updated)
    };
    let (file_id, folder_name, updated) = pending;
    if let Err(e) = begin_upload(client, file_id, &folder_name, updated) {
        set_finished(Err(e));
    }
}

fn start_create(client: Arc<Client>) {
    client
        .ugc()
        .create_item(AppId(MAHJURO_APP_ID), FileType::Community, move |result| {
            match result {
                Ok((id, _)) => {
                    if let Err(e) = queue_pending_upload(id, false) {
                        set_finished(Err(e));
                    }
                }
                Err(err) => set_finished(Err(format!("Workshop create failed: {err:?}"))),
            }
        });
}

/// Begin uploading a local mod folder to Steam Workshop. Idempotent while busy.
pub fn publish_local_mod(folder_name: &str) -> Result<(), String> {
    let client = steam_client().ok_or_else(|| "Steam is not connected".to_string())?;
    mod_paths(folder_name)?;
    let mut state = publisher()
        .lock()
        .map_err(|_| "Workshop publisher lock poisoned".to_string())?;
    if !matches!(state.phase, Phase::Idle) {
        return Err("A Workshop upload is already in progress".into());
    }
    if !list_mod_tilesets()
        .iter()
        .any(|e| e.folder_name == folder_name)
    {
        return Err(format!("unknown mod folder '{folder_name}'"));
    }

    state.folder_name = folder_name.to_string();

    if let Some(existing) = read_mod_workshop_id(folder_name) {
        state.phase = Phase::PendingUpload {
            file_id: PublishedFileId(existing),
            updated: true,
        };
        state.progress_label = Some("Starting upload…".into());
        return Ok(());
    }

    state.progress_label = Some("Creating Workshop item…".into());
    state.phase = Phase::Creating;
    drop(state);
    start_create(client);
    Ok(())
}

/// Poll upload progress; call each frame from the Steam backend tick.
pub fn tick_publish() {
    maybe_start_pending_upload();
    let Ok(mut state) = publisher().lock() else {
        return;
    };
    let Phase::Uploading = state.phase else {
        return;
    };
    let Some(watch) = state.watch.as_ref() else {
        return;
    };
    let (status, current, total) = watch.progress();
    state.progress_label = Some(match status {
        UpdateStatus::Invalid => "Uploading…".into(),
        UpdateStatus::PreparingConfig => "Preparing…".into(),
        UpdateStatus::PreparingContent => "Preparing content…".into(),
        UpdateStatus::UploadingContent if total > 0 => {
            format!("Uploading… {}%", (current * 100 / total).min(100))
        }
        UpdateStatus::UploadingContent => "Uploading content…".into(),
        UpdateStatus::UploadingPreviewFile => "Uploading preview…".into(),
        UpdateStatus::CommittingChanges => "Finishing…".into(),
    });
}

pub fn publish_busy() -> bool {
    publisher()
        .lock()
        .map(|s| !matches!(s.phase, Phase::Idle))
        .unwrap_or(false)
}

pub fn publish_progress_label() -> Option<String> {
    publisher().lock().ok()?.progress_label.clone()
}

/// Take a finished upload result (success or failure). Returns at most once per upload.
pub fn take_publish_result() -> Option<Result<WorkshopPublishResult, String>> {
    let Ok(mut state) = publisher().lock() else {
        return None;
    };
    let Phase::Finished(result) = &mut state.phase else {
        return None;
    };
    let out = result.take();
    if out.is_some() {
        state.phase = Phase::Idle;
        state.folder_name.clear();
    }
    out
}

pub fn open_workshop_item_overlay(file_id: u64) {
    let Some(client) = steam_client() else {
        return;
    };
    let url = format!("steam://url/CommunityFilePage/{file_id}");
    client.friends().activate_game_overlay_to_web_page(&url);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_title_replaces_underscores() {
        assert_eq!(mod_title("my_cool_set"), "my cool set");
    }
}
