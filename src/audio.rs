//! Audio system: sound effects and background music.
//!
//! Uses rodio for playback. Gracefully degrades if audio device is unavailable.
//! SFX (OGG/Vorbis) and music (MP3) are decoded to PCM once at load time so
//! the device thread never runs decoders during playback.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rodio::source::SeekError;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

/// Decoded asset: interleaved `i16` samples (same layout as [`Decoder`]).
struct PcmClip {
    channels: u16,
    sample_rate: u32,
    samples: Vec<i16>,
}

/// Cheap playback handle: [`Arc`] to shared PCM plus a cursor (no buffer copy per play).
struct SharedPcmSource {
    clip: Arc<PcmClip>,
    pos: usize,
}

impl SharedPcmSource {
    fn new(clip: Arc<PcmClip>) -> Self {
        Self { clip, pos: 0 }
    }

    fn duration(clip: &PcmClip) -> Duration {
        let sample_rate = clip.sample_rate as u64;
        let channels = clip.channels.max(1) as u64;
        let ns = 1_000_000_000u64
            .checked_mul(clip.samples.len() as u64)
            .unwrap_or(0)
            / sample_rate
            / channels;
        Duration::new(ns / 1_000_000_000, (ns % 1_000_000_000) as u32)
    }
}

impl Iterator for SharedPcmSource {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        let s = *self.clip.samples.get(self.pos)?;
        self.pos += 1;
        Some(s)
    }
}

impl Source for SharedPcmSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.clip.channels
    }

    fn sample_rate(&self) -> u32 {
        self.clip.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(Self::duration(&self.clip))
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let ch = self.channels() as usize;
        let curr_channel = self.pos % ch;
        let new_pos = (pos.as_secs_f32() * self.sample_rate() as f32 * ch as f32) as usize;
        let new_pos = new_pos.min(self.clip.samples.len());
        let new_pos = new_pos.next_multiple_of(ch);
        let new_pos = new_pos.saturating_sub(curr_channel);
        self.pos = new_pos;
        Ok(())
    }
}

fn decode_rodio(label: &str, bytes: &[u8]) -> Option<Arc<PcmClip>> {
    let cursor = Cursor::new(bytes.to_vec());
    let decoder = Decoder::new(cursor).ok()?;
    let channels = decoder.channels();
    let sample_rate = decoder.sample_rate();
    if channels == 0 || sample_rate == 0 {
        log::warn!("decode_rodio({label}): invalid ch={channels} rate={sample_rate}");
        return None;
    }
    let samples: Vec<i16> = decoder.collect();
    if samples.is_empty() {
        log::warn!("decode_rodio({label}): empty");
        return None;
    }
    Some(Arc::new(PcmClip {
        channels,
        sample_rate,
        samples,
    }))
}

/// Background music slot (embedded under `assets/audio/music/`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MusicId {
    MainMenu,
    Gameplay,
    Shop,
}

impl MusicId {
    fn asset_path(self) -> &'static str {
        match self {
            MusicId::MainMenu => "audio/music/main_menu.mp3",
            MusicId::Gameplay => "audio/music/gameplay.mp3",
            MusicId::Shop => "audio/music/shop.mp3",
        }
    }
}

/// Loops a decoded clip without `repeat_infinite`'s extra buffering (important
/// for long stereo tracks).
struct LoopingPcmSource {
    clip: Arc<PcmClip>,
    pos: usize,
}

impl LoopingPcmSource {
    fn new(clip: Arc<PcmClip>) -> Self {
        Self { clip, pos: 0 }
    }

    fn duration(clip: &PcmClip) -> Duration {
        SharedPcmSource::duration(clip)
    }
}

impl Iterator for LoopingPcmSource {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        if self.clip.samples.is_empty() {
            return None;
        }
        if self.pos >= self.clip.samples.len() {
            self.pos = 0;
        }
        let s = self.clip.samples[self.pos];
        self.pos += 1;
        Some(s)
    }
}

impl Source for LoopingPcmSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.clip.channels
    }

    fn sample_rate(&self) -> u32 {
        self.clip.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(Self::duration(&self.clip))
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let ch = self.channels() as usize;
        let curr_channel = self.pos % ch;
        let new_pos = (pos.as_secs_f32() * self.sample_rate() as f32 * ch as f32) as usize;
        let new_pos = new_pos.min(self.clip.samples.len());
        let new_pos = new_pos.next_multiple_of(ch);
        let new_pos = new_pos.saturating_sub(curr_channel);
        self.pos = new_pos;
        Ok(())
    }
}

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
    /// Chips/×/mult trio snaps together at the start of the hand-off
    /// tween — signals "the accounting is done, here's the total".
    CascadeMerge,
    /// Merged total leaves the pad and begins its flight toward the score
    /// reel — a rising whoosh that cues the upward motion.
    CascadeLaunch,
    /// Merged total lands in the score reel at the end of the flight —
    /// crisp impact paired with the reel finishing its tick-up.
    CascadeLand,
    /// One stinger per yaku detected in a scored hand. Emitted from the
    /// scoring loop so stacked yaku play as a rolling sequence of distinct
    /// cues rather than a single hit.
    YakuTanyao,
    YakuToitoi,
    YakuFullHand,
    YakuYakuhai,
    YakuIipeikou,
    YakuSanshokuDoujun,
    YakuIttsu,
    YakuHonitsu,
    YakuChinitsu,
    YakuJunchan,
    YakuHonroutou,
    YakuChiitoitsu,
    YakuKokushiMusou,
    YakuChickenHand,
    /// Played ~1 second after the victory screen appears.
    Victory,
    /// Alternate victory stinger; `Victory` / `Victory2` are picked at random.
    Victory2,
    /// Played ~1 second after the defeat screen appears.
    Defeat,
    /// Played whenever the main menu (start screen) is entered.
    MainMenuEnter,
    /// Generic level-up stinger.
    LevelUp,
    /// Tiles permanently destroyed (e.g. Taotie).
    TilesDestroyed,
    /// Boss blind encountered — ominous sting as the round begins.
    BossEncountered,
    /// Boss blind defeated — triumphant sting.
    BossDefeated,
    /// Talisman purchased from the shop.
    TalismanPurchased,
    /// Talisman consumed from the dish.
    TalismanUsed,
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
        SfxId::CascadeMerge,
        SfxId::CascadeLaunch,
        SfxId::CascadeLand,
        SfxId::YakuTanyao,
        SfxId::YakuToitoi,
        SfxId::YakuFullHand,
        SfxId::YakuYakuhai,
        SfxId::YakuIipeikou,
        SfxId::YakuSanshokuDoujun,
        SfxId::YakuIttsu,
        SfxId::YakuHonitsu,
        SfxId::YakuChinitsu,
        SfxId::YakuJunchan,
        SfxId::YakuHonroutou,
        SfxId::YakuChiitoitsu,
        SfxId::YakuKokushiMusou,
        SfxId::YakuChickenHand,
        SfxId::Victory,
        SfxId::Victory2,
        SfxId::Defeat,
        SfxId::MainMenuEnter,
        SfxId::LevelUp,
        SfxId::TilesDestroyed,
        SfxId::BossEncountered,
        SfxId::BossDefeated,
        SfxId::TalismanPurchased,
        SfxId::TalismanUsed,
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
            SfxId::CascadeMerge => "vwomp1.ogg",
            SfxId::CascadeLaunch => "intake.ogg",
            SfxId::CascadeLand => "Snap.ogg",
            SfxId::YakuTanyao => "yaku_tanyao.ogg",
            SfxId::YakuToitoi => "yaku_toitoi.ogg",
            SfxId::YakuFullHand => "yaku_full_hand.ogg",
            SfxId::YakuYakuhai => "yaku_yakuhai.ogg",
            SfxId::YakuIipeikou => "yaku_iipeikou.ogg",
            SfxId::YakuSanshokuDoujun => "yaku_sanshoku_doujun.ogg",
            SfxId::YakuIttsu => "yaku_ittsu.ogg",
            SfxId::YakuHonitsu => "yaku_honitsu.ogg",
            SfxId::YakuChinitsu => "yaku_chinitsu.ogg",
            SfxId::YakuJunchan => "yaku_junchan.ogg",
            SfxId::YakuHonroutou => "yaku_honroutou.ogg",
            SfxId::YakuChiitoitsu => "yaku_chiitoitsu.ogg",
            // TODO(audio): Ship `assets/audio/yaku_kokushi_musou.ogg` (or alias another stinger); until then scoring no-ops this cue.
            SfxId::YakuKokushiMusou => "yaku_kokushi_musou.ogg",
            SfxId::YakuChickenHand => "yaku_chicken_hand.ogg",
            SfxId::Victory => "victory.ogg",
            SfxId::Victory2 => "victory2.ogg",
            SfxId::Defeat => "defeat.ogg",
            SfxId::MainMenuEnter => "mahjuro.ogg",
            SfxId::LevelUp => "levelup.ogg",
            SfxId::TilesDestroyed => "tiles_destroyed.ogg",
            SfxId::BossEncountered => "boss_encountered.ogg",
            SfxId::BossDefeated => "boss_defeated.ogg",
            SfxId::TalismanPurchased => "talisman_purchased.ogg",
            SfxId::TalismanUsed => "talisman_used.ogg",
        }
    }

    /// Per-yaku stinger for `kind`, emitted once per detected yaku during
    /// scoring. Missing audio files no-op in [`AudioManager::play_sfx`], so
    /// dropping new `.ogg` assets into `assets/audio/` with the filenames
    /// above is all it takes to enable these.
    pub fn for_yaku(kind: crate::core::yaku::YakuKind) -> SfxId {
        use crate::core::yaku::YakuKind;
        match kind {
            YakuKind::Tanyao => SfxId::YakuTanyao,
            YakuKind::Toitoi => SfxId::YakuToitoi,
            YakuKind::FullHand => SfxId::YakuFullHand,
            YakuKind::Yakuhai => SfxId::YakuYakuhai,
            YakuKind::Iipeikou => SfxId::YakuIipeikou,
            YakuKind::SanshokuDoujun => SfxId::YakuSanshokuDoujun,
            YakuKind::Ittsu => SfxId::YakuIttsu,
            YakuKind::Honitsu => SfxId::YakuHonitsu,
            YakuKind::Chinitsu => SfxId::YakuChinitsu,
            YakuKind::Junchan => SfxId::YakuJunchan,
            YakuKind::Honroutou => SfxId::YakuHonroutou,
            YakuKind::Chiitoitsu => SfxId::YakuChiitoitsu,
            YakuKind::KokushiMusou => SfxId::YakuKokushiMusou,
            YakuKind::ChickenHand => SfxId::YakuChickenHand,
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
    1.000_000,                // root
    1.059_463,                // +1 semitone
    1.122_462,                // +2
    1.189_207,                // +3
    1.259_921,                // +4
    1.334_84,                 // +5
    std::f32::consts::SQRT_2, // +6 (2^(6/12))
    1.498_307,                // +7
];

pub struct AudioManager {
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    sfx_data: HashMap<SfxId, Arc<PcmClip>>,
    music_data: HashMap<MusicId, Arc<PcmClip>>,
    /// Per-relic trigger samples loaded from `assets/audio/relics/<slug>.ogg`
    /// at startup. Lookup is by `RelicId`; missing entries fall back to
    /// [`SfxId::ScoreStep`] in [`AudioManager::play_relic_trigger`].
    relic_trigger_data: HashMap<crate::core::relic::RelicId, Arc<PcmClip>>,
    /// Live sinks, FIFO-ordered (oldest first). Finished sinks are swept
    /// each `play_sfx` call; when the cap is hit, the oldest is dropped.
    active_sinks: Vec<Sink>,
    /// Sfx queued for future playback with their due time, kept in
    /// ascending time order. Drained each frame by [`AudioManager::tick`]
    /// as entries come due. Used to stagger stacked stingers (e.g. yaku
    /// on a multi-yaku commit) so they roll out one after another.
    pending_sfx: Vec<(Instant, SfxId)>,
    master_volume: f32,
    sfx_volume: f32,
    music_volume: f32,
    /// User toggle: sound effects only; music uses [`Self::handle`] independently.
    sfx_enabled: bool,
    /// Dedicated sink for looping background music (separate from SFX sinks).
    music_sink: Option<Sink>,
    /// Last requested background track.
    last_music: Option<MusicId>,
    /// Track currently driving `music_sink` (when playing).
    music_active_id: Option<MusicId>,
}

impl AudioManager {
    pub fn new() -> Self {
        let (stream, handle) = match OutputStream::try_default() {
            Ok((s, h)) => (Some(s), Some(h)),
            Err(e) => {
                log::warn!("Audio device unavailable: {e}. Running without sound.");
                (None, None)
            }
        };

        let mut sfx_data = HashMap::new();
        for &sfx_id in all_sfx_ids() {
            let asset_path = format!("audio/{}", sfx_id.filename());
            if let Some(file) = crate::asset_path::get(&asset_path) {
                let label = format!("{asset_path} ({sfx_id:?})");
                if let Some(clip) = decode_rodio(&label, file.data.as_ref()) {
                    sfx_data.insert(sfx_id, clip);
                }
            }
        }

        if sfx_data.is_empty() {
            log::warn!("No audio files found in assets/audio/. Sound effects disabled.");
        } else {
            log::debug!("Loaded {} sound effect(s).", sfx_data.len());
        }

        let mut relic_trigger_data = HashMap::new();
        for def in crate::core::relic::all_relic_defs() {
            let slug = def.id.asset_filename().trim_end_matches(".png");
            let asset_path = format!("audio/relics/{slug}.ogg");
            if let Some(file) = crate::asset_path::get(&asset_path) {
                let label = asset_path.clone();
                if let Some(clip) = decode_rodio(&label, file.data.as_ref()) {
                    relic_trigger_data.insert(def.id, clip);
                }
            }
        }
        if !relic_trigger_data.is_empty() {
            log::debug!(
                "Loaded {} per-relic trigger sound(s).",
                relic_trigger_data.len()
            );
        }

        let mut music_data = HashMap::new();
        for id in [MusicId::MainMenu, MusicId::Gameplay, MusicId::Shop] {
            let path = id.asset_path();
            if let Some(file) = crate::asset_path::get(path) {
                if let Some(clip) = decode_rodio(path, file.data.as_ref()) {
                    music_data.insert(id, clip);
                }
            }
        }
        if music_data.is_empty() {
            log::debug!("No music files in assets/audio/music/; background music disabled.");
        } else {
            log::debug!("Loaded {} background music track(s).", music_data.len());
        }

        Self {
            _stream: stream,
            handle,
            sfx_data,
            music_data,
            relic_trigger_data,
            active_sinks: Vec::with_capacity(MAX_CONCURRENT_SFX),
            pending_sfx: Vec::new(),
            master_volume: 0.7,
            sfx_volume: 0.7,
            music_volume: 0.7,
            sfx_enabled: true,
            music_sink: None,
            last_music: None,
            music_active_id: None,
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

    /// Queue `id` for playback at `when`. Pair with [`AudioManager::tick`]
    /// to actually fire it; used to stagger stacked stingers so they land
    /// as a sequence instead of overlapping.
    pub fn schedule_sfx(&mut self, id: SfxId, when: Instant) {
        self.pending_sfx.push((when, id));
    }

    /// Drain any scheduled sfx whose time has arrived. Call once per frame.
    pub fn tick(&mut self, now: Instant) {
        while self.pending_sfx.first().is_some_and(|(t, _)| *t <= now) {
            let (_, id) = self.pending_sfx.remove(0);
            self.play_sfx(id);
        }
    }

    fn play_sfx_with_speed(&mut self, id: SfxId, speed: f32) {
        let Some(clip) = self.sfx_data.get(&id).cloned() else {
            log::debug!("play_sfx({id:?}): no data");
            return;
        };
        self.play_clip(&format!("{id:?}"), clip, speed);
    }

    /// Play the per-relic trigger stinger for `rid`. Falls back to
    /// [`SfxId::ScoreStep`] when no `audio/relics/<slug>.ogg` is loaded for
    /// this relic, so new relics get a reasonable default until bespoke
    /// audio is added.
    pub fn play_relic_trigger(&mut self, rid: crate::core::relic::RelicId) {
        if let Some(clip) = self.relic_trigger_data.get(&rid).cloned() {
            self.play_clip(&format!("Relic({rid:?})"), clip, 1.0);
        } else {
            self.play_sfx(SfxId::ScoreStep);
        }
    }

    fn play_clip(&mut self, tag: &str, clip: Arc<PcmClip>, speed: f32) {
        if !self.sfx_enabled {
            log::debug!("play_clip({tag}): sfx disabled");
            return;
        }
        let Some(handle) = &self.handle else {
            log::debug!("play_clip({tag}): no handle");
            return;
        };
        let Ok(sink) = Sink::try_new(handle) else {
            log::warn!("play_clip({tag}): sink creation failed");
            return;
        };

        self.active_sinks.retain(|s| !s.empty());
        if self.active_sinks.len() >= MAX_CONCURRENT_SFX {
            let dropped = self.active_sinks.remove(0);
            log::debug!(
                "play_clip({tag}): concurrent cap hit, dropping oldest sink (live={})",
                self.active_sinks.len() + 1,
            );
            drop(dropped);
        }

        let effective_vol = self.master_volume * self.sfx_volume;
        log::trace!(
            "play_clip({tag}): vol={effective_vol:.2} live={}",
            self.active_sinks.len() + 1,
        );
        let source = SharedPcmSource::new(clip);
        let amplified = source.speed(speed).amplify(effective_vol);
        sink.append(amplified);
        self.active_sinks.push(sink);
    }

    /// Set the master volume (0.0 to 1.0).
    pub fn set_master_volume(&mut self, vol: f32) {
        self.master_volume = vol.clamp(0.0, 1.0);
        self.refresh_music_sink_volume();
    }

    /// Set the sound effects volume (0.0 to 1.0).
    pub fn set_sfx_volume(&mut self, vol: f32) {
        self.sfx_volume = vol.clamp(0.0, 1.0);
    }

    /// Set the music volume (0.0 to 1.0).
    pub fn set_music_volume(&mut self, vol: f32) {
        self.music_volume = vol.clamp(0.0, 1.0);
        self.refresh_music_sink_volume();
    }

    /// Enable or disable sound effects. Background music is unaffected.
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.sfx_enabled == enabled {
            return;
        }
        self.sfx_enabled = enabled;
    }

    fn refresh_music_sink_volume(&mut self) {
        let Some(sink) = self.music_sink.as_ref() else {
            return;
        };
        sink.set_volume(self.master_volume * self.music_volume);
    }

    /// Stop background music and clear the remembered track (e.g. splash / loading).
    pub fn stop_background_music(&mut self) {
        self.last_music = None;
        self.music_active_id = None;
        if let Some(sink) = self.music_sink.take() {
            sink.stop();
        }
    }

    /// Switch background music to `id` (loops). No-op if the asset is missing.
    pub fn set_music_track(&mut self, id: MusicId) {
        self.last_music = Some(id);
        self.start_music_track(id);
    }

    fn start_music_track(&mut self, id: MusicId) {
        let Some(clip) = self.music_data.get(&id).cloned() else {
            log::debug!("start_music_track({id:?}): no data");
            return;
        };
        let Some(handle) = &self.handle else {
            return;
        };
        if let Some(sink) = self.music_sink.as_ref() {
            if !sink.empty() && self.music_active_id == Some(id) {
                self.refresh_music_sink_volume();
                return;
            }
        }
        if let Some(sink) = self.music_sink.take() {
            sink.stop();
        }
        let Ok(sink) = Sink::try_new(handle) else {
            log::warn!("start_music_track({id:?}): sink creation failed");
            return;
        };
        sink.set_volume(self.master_volume * self.music_volume);
        let source = LoopingPcmSource::new(clip);
        sink.append(source);
        self.music_sink = Some(sink);
        self.music_active_id = Some(id);
    }
}
