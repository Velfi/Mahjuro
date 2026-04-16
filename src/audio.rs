//! Audio system: sound effects and background music.
//!
//! Uses rodio for playback. Gracefully degrades if audio device is unavailable.

use std::collections::HashMap;
use std::io::Cursor;

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

/// Sound effect identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SfxId {
    UiConfirm,
    UiCancel,
    TileClick,
    TileSelect,
    TileDeselect,
    TilePlace,
    TileDiscard,
    ScoreReveal,
    ScoreStep,
    /// Base sample for the cascade tick. Cycled through eight semitone-spaced
    /// pitches per step via [`AudioManager::play_score_tick`] so the reveal
    /// sequence audibly climbs a scale.
    ScoreTick,
    /// Brassy hit jingle layered on top of `ScoreFinal` to add weight to
    /// the closing crescendo.
    ScoreCrescendo,
    ScoreFinal,
    RelicPickup,
    InvalidAction,
    CoinDrop,
    RoundWin,
    GameOver,
    /// Tile pack purchased in the shop.
    PackBuy,
    /// Tile pack foil wrapper tearing open.
    PackOpen,
    /// Individual tile revealed during pack opening celebration.
    PackTileReveal,
    /// Zodiac ribbon close-up reveal (mystical shimmer).
    ZodiacReveal,
    /// Zodiac consumed — yaku levelled up (positive stinger).
    ZodiacLevelUp,
    /// Rising whoosh when candles flare up on a blind-breaking hand.
    CandleFlareWhoosh,
    /// Heavy impact sting layered on top of the whoosh.
    CandleFlareImpact,
    /// Melds locked into the structure bank (distinct from [`Self::TilePlace`]).
    StructureCommit,
    /// Focus moved to a hand tile.
    FocusHandTile,
    /// Focus moved to an action-bar button (Play, Discard, Sort, etc.).
    FocusButton,
    /// Focus moved to a consumable slot.
    FocusConsumable,
    /// Focus moved to a relic on the shelf.
    FocusRelic,
    /// Focus moved to a UI peg (hand-size, discards, etc.).
    FocusPeg,
    /// Focus moved to the gold readout.
    FocusGold,
    /// Focus moved to a yaku-progress tablet.
    FocusYakuTablet,
    /// Focus moved to the dora indicator stand.
    FocusDora,
    /// A gameplay round begins.
    RoundStart,
    /// The shooting-star cascade transition fires (dramatic scene change).
    StarShimmer,
    /// Relic / zodiac / talisman purchased in the shop.
    Purchase,
    /// Relic / consumable sold back for gold.
    Sell,
    /// Pause menu opened.
    Pause,
    /// Pause menu dismissed (game resumed).
    Unpause,
    /// One chime per dora tile in a scored hand. Queued sequentially so
    /// multiple dora play as a rolling ding-ding-ding rather than a single
    /// stacked hit.
    DoraScored,
    /// Structure cashed in (manual Trigger or auto-full). Bright bell-stack
    /// chime that precedes the scoring cascade.
    CashIn,
}

/// All SFX variants in display order. Single source of truth shared by the
/// startup loader and the debug "Sound Effects Test" overlay so they can't drift.
pub fn all_sfx_ids() -> &'static [SfxId] {
    &[
        SfxId::UiConfirm,
        SfxId::UiCancel,
        SfxId::TileClick,
        SfxId::TileSelect,
        SfxId::TileDeselect,
        SfxId::TilePlace,
        SfxId::TileDiscard,
        SfxId::ScoreReveal,
        SfxId::ScoreStep,
        SfxId::ScoreTick,
        SfxId::ScoreCrescendo,
        SfxId::ScoreFinal,
        SfxId::RelicPickup,
        SfxId::InvalidAction,
        SfxId::CoinDrop,
        SfxId::RoundWin,
        SfxId::GameOver,
        SfxId::PackBuy,
        SfxId::PackOpen,
        SfxId::PackTileReveal,
        SfxId::ZodiacReveal,
        SfxId::ZodiacLevelUp,
        SfxId::CandleFlareWhoosh,
        SfxId::CandleFlareImpact,
        SfxId::StructureCommit,
        SfxId::FocusHandTile,
        SfxId::FocusButton,
        SfxId::FocusConsumable,
        SfxId::FocusRelic,
        SfxId::FocusPeg,
        SfxId::FocusGold,
        SfxId::FocusYakuTablet,
        SfxId::FocusDora,
        SfxId::RoundStart,
        SfxId::StarShimmer,
        SfxId::Purchase,
        SfxId::Sell,
        SfxId::Pause,
        SfxId::Unpause,
        SfxId::DoraScored,
        SfxId::CashIn,
    ]
}

impl SfxId {
    pub(crate) fn filename(self) -> &'static str {
        match self {
            SfxId::UiConfirm => "kenney_interface-sounds/Audio/confirmation_001.ogg",
            SfxId::UiCancel => "kenney_interface-sounds/Audio/back_001.ogg",
            SfxId::TileClick => "kenney_interface-sounds/Audio/drop_003.ogg",
            SfxId::TileSelect => "kenney_interface-sounds/Audio/tick_002.ogg",
            SfxId::TileDeselect => "kenney_interface-sounds/Audio/tick_004.ogg",
            SfxId::TilePlace => "Snap.ogg",
            SfxId::TileDiscard => "freesound_community-tile-shuffle-99834.ogg",
            SfxId::ScoreReveal => "intake.ogg",
            SfxId::ScoreStep => "vwomp2.ogg",
            SfxId::ScoreTick => "plink3.ogg",
            SfxId::ScoreCrescendo => "scorecrescendo.ogg",
            SfxId::ScoreFinal => "MixingBell.ogg",
            SfxId::RelicPickup => "relic_pickup.ogg",
            SfxId::InvalidAction => "invalid.ogg",
            SfxId::CoinDrop => "coindrop.ogg",
            SfxId::RoundWin => "roundwin.ogg",
            SfxId::GameOver => "gameover.ogg",
            SfxId::PackBuy => "kenney_interface-sounds/Audio/confirmation_003.ogg",
            SfxId::PackOpen => "kenney_interface-sounds/Audio/open_003.ogg",
            SfxId::PackTileReveal => "kenney_interface-sounds/Audio/pluck_001.ogg",
            SfxId::ZodiacReveal => "kenney_interface-sounds/Audio/glass_006.ogg",
            SfxId::ZodiacLevelUp => "zodiac_jingle.ogg",
            SfxId::CandleFlareWhoosh => "candle_flareup.ogg",
            SfxId::CandleFlareImpact => "candle_impact.ogg",
            SfxId::StructureCommit => "kenney_interface-sounds/Audio/confirmation_002.ogg",
            SfxId::FocusHandTile => "kenney_interface-sounds/Audio/select_001.ogg",
            SfxId::FocusButton => "kenney_interface-sounds/Audio/select_004.ogg",
            SfxId::FocusConsumable => "kenney_interface-sounds/Audio/pluck_002.ogg",
            SfxId::FocusRelic => "kenney_interface-sounds/Audio/glass_003.ogg",
            SfxId::FocusPeg => "kenney_interface-sounds/Audio/switch_002.ogg",
            SfxId::FocusGold => "kenney_interface-sounds/Audio/bong_001.ogg",
            SfxId::FocusYakuTablet => "kenney_interface-sounds/Audio/scroll_003.ogg",
            SfxId::FocusDora => "kenney_interface-sounds/Audio/glass_001.ogg",
            SfxId::RoundStart => "kenney_interface-sounds/Audio/confirmation_004.ogg",
            SfxId::StarShimmer => "kenney_interface-sounds/Audio/glass_005.ogg",
            SfxId::Purchase => "coindrop.ogg",
            SfxId::Sell => "kenney_interface-sounds/Audio/confirmation_002.ogg",
            SfxId::Pause => "kenney_interface-sounds/Audio/minimize_003.ogg",
            SfxId::Unpause => "kenney_interface-sounds/Audio/maximize_003.ogg",
            SfxId::DoraScored => "kenney_interface-sounds/Audio/glass_002.ogg",
            SfxId::CashIn => "cash_in.ogg",
        }
    }
}

/// Maximum number of SFX sinks allowed to ring simultaneously. Rodio mixes
/// detached sinks by summing samples with no limiter, so uncapped pileups
/// (e.g. a candle-flare whoosh + impact overlapping a burst of cascade
/// ticks, each of which itself layers two sounds) clip and smear into mud.
/// When the cap is hit, the oldest live sink is dropped to make room.
const MAX_CONCURRENT_SFX: usize = 8;

/// Playback speeds used by [`AudioManager::play_score_tick`], cycling per
/// cascade step. Each ratio is a semitone above the previous (2^(n/12)) so
/// the reveal climbs a chromatic scale across eight steps before wrapping.
const SCORE_TICK_PITCHES: [f32; 8] = [
    1.000_000, // root
    1.059_463, // +1 semitone
    1.122_462, // +2
    1.189_207, // +3
    1.259_921, // +4
    1.334_840, // +5
    1.414_214, // +6
    1.498_307, // +7
];

pub struct AudioManager {
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    sfx_data: HashMap<SfxId, Vec<u8>>,
    /// Live sinks, FIFO-ordered (oldest first). Finished sinks are swept
    /// each `play_sfx` call; when the cap is hit, the oldest is dropped.
    active_sinks: Vec<Sink>,
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
            active_sinks: Vec::with_capacity(MAX_CONCURRENT_SFX),
            master_volume: 0.7,
            sfx_volume: 0.7,
            music_volume: 0.7,
            enabled,
        }
    }

    /// Play the cascade tick at one of [`SCORE_TICK_PITCHES`] stepped speeds,
    /// indexed by `step`. Each speed shifts the base sample by one semitone,
    /// so the cascade climbs a major-scale-ish run as scoring unfolds.
    pub fn play_score_tick(&mut self, step: usize) {
        let speed = SCORE_TICK_PITCHES[step % SCORE_TICK_PITCHES.len()];
        self.play_sfx_with_speed(SfxId::ScoreTick, speed);
    }

    /// Play a sound effect. No-op if audio is unavailable or the SFX file wasn't loaded.
    pub fn play_sfx(&mut self, id: SfxId) {
        self.play_sfx_with_speed(id, 1.0);
    }

    fn play_sfx_with_speed(&mut self, id: SfxId, speed: f32) {
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

        self.active_sinks.retain(|s| !s.empty());
        if self.active_sinks.len() >= MAX_CONCURRENT_SFX {
            let dropped = self.active_sinks.remove(0);
            log::debug!(
                "play_sfx({id:?}): concurrent cap hit, dropping oldest sink (live={})",
                self.active_sinks.len() + 1,
            );
            drop(dropped);
        }

        let effective_vol = self.master_volume * self.sfx_volume;
        log::debug!(
            "play_sfx({id:?}): vol={effective_vol:.2} live={}",
            self.active_sinks.len() + 1,
        );
        let amplified = source.speed(speed).amplify(effective_vol);
        sink.append(amplified);
        self.active_sinks.push(sink);
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
