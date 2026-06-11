//! Steam Workshop sync for player tilesets.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mahjuro_assets::tileset_workshop::{self, WorkshopTilesetInstall};
use steamworks::{
    AppId, CallbackHandle, Client, DownloadItemResult, ItemState, PublishedFileId, UGC,
};

use super::MAHJURO_APP_ID;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

static STEAM_CLIENT: std::sync::OnceLock<Arc<Client>> = std::sync::OnceLock::new();

pub(crate) fn steam_client() -> Option<Arc<Client>> {
    STEAM_CLIENT.get().cloned()
}

pub fn register_steam_client(client: Arc<Client>) {
    let _ = STEAM_CLIENT.set(client);
}

/// Open Mahjuro's Workshop hub in the Steam overlay (no-op when Steam is disabled).
pub fn open_tileset_workshop_overlay() {
    let Some(client) = STEAM_CLIENT.get() else {
        return;
    };
    let url = format!("https://steamcommunity.com/app/{MAHJURO_APP_ID}/workshop/");
    client.friends().activate_game_overlay_to_web_page(&url);
}

pub struct TilesetWorkshop {
    client: Arc<Client>,
    dirty: AtomicBool,
    last_refresh: Mutex<Instant>,
    _download_listener: CallbackHandle,
}

impl TilesetWorkshop {
    pub fn new(client: Arc<Client>) -> Self {
        register_steam_client(client.clone());
        let dirty = Arc::new(AtomicBool::new(true));
        let dirty_cb = dirty.clone();
        let _download_listener = client.register_callback(move |result: DownloadItemResult| {
            if result.app_id.0 != MAHJURO_APP_ID {
                return;
            }
            if result.error.is_none() {
                dirty_cb.store(true, Ordering::Relaxed);
            }
        });
        Self {
            client,
            dirty: AtomicBool::new(true),
            last_refresh: Mutex::new(Instant::now() - REFRESH_INTERVAL),
            _download_listener,
        }
    }

    pub fn tick(&self) {
        super::workshop_publish::tick_publish();
        let now = Instant::now();
        let interval_elapsed = self
            .last_refresh
            .lock()
            .map(|last| now.duration_since(*last) >= REFRESH_INTERVAL)
            .unwrap_or(true);
        if !self.dirty.load(Ordering::Relaxed) && !interval_elapsed {
            return;
        }
        self.refresh();
        self.dirty.store(false, Ordering::Relaxed);
        if let Ok(mut last) = self.last_refresh.lock() {
            *last = now;
        }
    }

    pub fn open_browse_overlay(&self) {
        open_tileset_workshop_overlay();
    }

    fn refresh(&self) {
        let ugc = self.client.ugc();
        let app_id = AppId(MAHJURO_APP_ID);
        let subscribed = ugc.subscribed_items(false);
        let mut installs = Vec::new();
        let mut titleless = Vec::new();

        for file_id in subscribed {
            let state = ugc.item_state(file_id);
            if !state.contains(ItemState::INSTALLED) {
                if state.contains(ItemState::NEEDS_UPDATE)
                    || (!state.contains(ItemState::DOWNLOADING)
                        && !state.contains(ItemState::DOWNLOAD_PENDING))
                {
                    ugc.download_item(file_id, false);
                }
                continue;
            }
            let Some(info) = ugc.item_install_info(file_id) else {
                continue;
            };
            let install_root = std::path::Path::new(&info.folder);
            let Some(content_dir) = tileset_workshop::validate_workshop_install(install_root) else {
                log::warn!(
                    "skipping subscribed Workshop item {}: no valid atlas.toml + atlas.png under {}",
                    file_id.0,
                    info.folder
                );
                continue;
            };
            let prior_title = tileset_workshop::list_workshop_tilesets()
                .into_iter()
                .find(|e| e.published_file_id == file_id.0)
                .and_then(|e| e.title);
            if prior_title.is_none() {
                titleless.push(file_id);
            }
            installs.push(WorkshopTilesetInstall {
                published_file_id: file_id.0,
                title: prior_title,
                content_dir,
            });
        }

        installs.sort_by_key(|e| e.published_file_id);
        tileset_workshop::set_workshop_installs(installs);
        self.fetch_titles(&ugc, app_id, titleless);
    }

    fn fetch_titles(&self, ugc: &UGC, app_id: AppId, ids: Vec<PublishedFileId>) {
        if ids.is_empty() {
            return;
        }
        let Ok(query) = ugc.query_items(ids) else {
            return;
        };
        query.fetch(move |result| match result {
            Ok(results) => {
                for index in 0..results.returned_results() {
                    let Some(item) = results.get(index) else {
                        continue;
                    };
                    if item.consumer_app_id != Some(app_id) {
                        continue;
                    }
                    tileset_workshop::update_workshop_title(item.published_file_id.0, item.title);
                }
            }
            Err(err) => {
                log::debug!("Workshop tileset title query failed: {err:?}");
            }
        });
    }
}
