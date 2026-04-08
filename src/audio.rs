//! Audio system: sound effects and background music.
//!
//! Uses rodio for playback. Gracefully degrades if audio device is unavailable.

use std::collections::HashMap;
use std::io::Cursor;

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

/// Sound effect identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SfxId {
    TileClick,
    TilePlace,
    TileDiscard,
    ScoreReveal,
    ScoreStep,
    ScoreFinal,
    RelicPickup,
    InvalidAction,
    RoundWin,
    GameOver,
}

/// All SFX variants in display order. Single source of truth shared by the
/// startup loader and the debug "Sound Effects Test" overlay so they can't drift.
pub fn all_sfx_ids() -> &'static [SfxId] {
    &[
        SfxId::TileClick,
        SfxId::TilePlace,
        SfxId::TileDiscard,
        SfxId::ScoreReveal,
        SfxId::ScoreStep,
        SfxId::ScoreFinal,
        SfxId::RelicPickup,
        SfxId::InvalidAction,
        SfxId::RoundWin,
        SfxId::GameOver,
    ]
}

impl SfxId {
    pub(crate) fn filename(self) -> &'static str {
        match self {
            SfxId::TileClick => "kenney_interface-sounds/Audio/drop_003.ogg",
            SfxId::TilePlace => "nomagician-ui-button-sound-cancel-back-exit-continue-467877.mp3",
            SfxId::TileDiscard => "freesound_community-tile-shuffle-99834.mp3",
            SfxId::ScoreReveal => "kenney_ui-audio/Audio/switch38.ogg",
            SfxId::ScoreStep => "kenney_ui-audio/Audio/rollover1.ogg",
            SfxId::ScoreFinal => "kenney_interface-sounds/Audio/confirmation_002.ogg",
            SfxId::RelicPickup => "relic_pickup.ogg",
            SfxId::InvalidAction => "kenney_interface-sounds/Audio/drop_003.ogg",
            SfxId::RoundWin => "kenney_music-jingles/Audio/Sax jingles/jingles_SAX16.ogg",
            SfxId::GameOver => "alphix-game-over-417465.mp3",
        }
    }
}

pub struct AudioManager {
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    sfx_data: HashMap<SfxId, Vec<u8>>,
    master_volume: f32,
    sfx_volume: f32,
    music_volume: f32,
    enabled: bool,
}

impl AudioManager {
    pub fn new() -> Self {
        let (stream, handle, enabled) = match OutputStream::try_default() {
            Ok((s, h)) => (Some(s), Some(h), true),
            Err(e) => {
                log::warn!("Audio device unavailable: {e}. Running without sound.");
                (None, None, false)
            }
        };

        let mut sfx_data = HashMap::new();
        for &sfx_id in all_sfx_ids() {
            let asset_path = format!("audio/{}", sfx_id.filename());
            if let Some(file) = crate::asset_path::get(&asset_path) {
                sfx_data.insert(sfx_id, file.data.to_vec());
            }
        }

        if sfx_data.is_empty() {
            log::info!("No audio files found in assets/audio/. Sound effects disabled.");
        } else {
            log::info!("Loaded {} sound effect(s).", sfx_data.len());
        }

        Self {
            _stream: stream,
            handle,
            sfx_data,
            master_volume: 0.7,
            sfx_volume: 0.7,
            music_volume: 0.7,
            enabled,
        }
    }

    /// Play a sound effect. No-op if audio is unavailable or the SFX file wasn't loaded.
    pub fn play_sfx(&self, id: SfxId) {
        if !self.enabled {
            log::debug!("play_sfx({id:?}): disabled");
            return;
        }
        let Some(handle) = &self.handle else {
            log::debug!("play_sfx({id:?}): no handle");
            return;
        };
        let Some(data) = self.sfx_data.get(&id) else {
            log::debug!("play_sfx({id:?}): no data");
            return;
        };
        let cursor = Cursor::new(data.clone());
        let Ok(source) = Decoder::new(cursor) else {
            log::warn!("play_sfx({id:?}): decoder failed");
            return;
        };
        let Ok(sink) = Sink::try_new(handle) else {
            log::warn!("play_sfx({id:?}): sink creation failed");
            return;
        };
        let effective_vol = self.master_volume * self.sfx_volume;
        log::debug!("play_sfx({id:?}): vol={effective_vol:.2}");
        let amplified = source.amplify(effective_vol);
        sink.append(amplified);
        sink.detach();
    }

    /// Set the master volume (0.0 to 1.0).
    pub fn set_master_volume(&mut self, vol: f32) {
        self.master_volume = vol.clamp(0.0, 1.0);
    }

    /// Set the sound effects volume (0.0 to 1.0).
    pub fn set_sfx_volume(&mut self, vol: f32) {
        self.sfx_volume = vol.clamp(0.0, 1.0);
    }

    /// Set the music volume (0.0 to 1.0).
    pub fn set_music_volume(&mut self, vol: f32) {
        self.music_volume = vol.clamp(0.0, 1.0);
    }

    /// Enable or disable sound effects.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}
