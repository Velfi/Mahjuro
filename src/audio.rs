//! Audio system: sound effects and background music.
//!
//! Uses rodio for playback. Gracefully degrades if audio device is unavailable.
//! SFX and music (OGG/Vorbis) are decoded to PCM once at load time so the
//! device thread never runs decoders during playback.

use rustc_hash::FxHashMap;
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

fn clip_duration_at_speed(clip: &PcmClip, speed: f32) -> Duration {
    let base = SharedPcmSource::duration(clip);
    if speed > 0.0 && speed.is_finite() {
        Duration::from_secs_f32(base.as_secs_f32() / speed)
    } else {
        base
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

pub use crate::sfx_id::{AmbientId, MusicId, SfxId, all_sfx_ids};

impl crate::sfx_id::MusicId {
    fn asset_path(self) -> &'static str {
        match self {
            MusicId::MainMenu => "audio/music/main_menu.ogg",
            MusicId::Gameplay => "audio/music/gameplay.ogg",
            MusicId::GameplayIntense => "audio/music/gameplay_intense.ogg",
            MusicId::GameplayIntro => "audio/music/gameplay_intro.ogg",
            MusicId::GameplayIntenseIntro => "audio/music/gameplay_intense_intro.ogg",
            MusicId::Shop => "audio/music/shop.ogg",
            MusicId::Credits => "audio/music/credits.ogg",
            MusicId::ChamberWin => "audio/music/chamber_win.ogg",
            MusicId::ChamberLoss => "audio/music/chamber_loss.ogg",
            MusicId::OrdealWin => "audio/music/ordeal_win.ogg",
            MusicId::OrdealLoss => "audio/music/ordeal_loss.ogg",
        }
    }

    /// Loop that follows a one-shot intro stinger.
    fn loop_after_intro(self) -> Option<MusicId> {
        match self {
            MusicId::GameplayIntro => Some(MusicId::Gameplay),
            MusicId::GameplayIntenseIntro => Some(MusicId::GameplayIntense),
            _ => None,
        }
    }

    /// True when the track is a one-shot (no looping); used by
    /// [`AudioManager::play_music_jingle`] vs [`AudioManager::set_music_track`].
    fn is_one_shot(self) -> bool {
        self.is_jingle() || self.loop_after_intro().is_some()
    }

    /// True when the track is a win/loss stinger (no intro hand-off).
    fn is_jingle(self) -> bool {
        matches!(
            self,
            MusicId::ChamberWin | MusicId::ChamberLoss | MusicId::OrdealWin | MusicId::OrdealLoss
        )
    }

    /// True when the track loops via [`AudioManager::set_music_track`].
    fn is_loop(self) -> bool {
        !self.is_one_shot()
    }
}

impl crate::sfx_id::AmbientId {
    fn asset_path(self) -> &'static str {
        match self {
            AmbientId::MainMenuRain => "audio/ambient/main_menu_rain.ogg",
            AmbientId::HallwayBulbBuzz => {
                "audio/sfx/freesound_community-room-with-buzz-incandescent-light-bulb-23892.ogg"
            }
        }
    }

    fn gain(self) -> f32 {
        match self {
            AmbientId::MainMenuRain => AMBIENT_RAIN_GAIN,
            AmbientId::HallwayBulbBuzz => AMBIENT_HALLWAY_BULB_GAIN,
        }
    }
}

/// Scales [`AmbientId::MainMenuRain`] under main-menu BGM.
const AMBIENT_RAIN_GAIN: f32 = 0.32;
const AMBIENT_HALLWAY_BULB_GAIN: f32 = 0.28;

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

/// Random thuds shared by all focus-navigation cues.
const FOCUS_THUDS: &[&str] = &[
    "sfx/sfx13.ogg",
    "sfx/sfx14.ogg",
    "sfx/sfx15.ogg",
    "sfx/sfx16.ogg",
    "sfx/sfx17.ogg",
    "sfx/sfx18.ogg",
    "sfx/sfx19.ogg",
    "sfx/sfx20.ogg",
    "sfx/sfx21.ogg",
    "sfx/sfx22.ogg",
    "sfx/sfx23.ogg",
    "sfx/sfx24.ogg",
    "sfx/sfx25.ogg",
    "sfx/sfx26.ogg",
    "sfx/sfx27.ogg",
    "sfx/sfx28.ogg",
    "sfx/sfx29.ogg",
];

impl crate::sfx_id::SfxId {
    pub(crate) fn filename(self) -> &'static str {
        match self {
            SfxId::UiConfirm => "cash_in.ogg",
            SfxId::UiCancel => "sfx/can_ping.ogg",
            SfxId::TileClick => "sfx/tile_click-01.ogg",
            SfxId::TileSelect => "sfx/sfx1.ogg",
            SfxId::TileDeselect => "sfx/sfx2.ogg",
            SfxId::TilePlace => "Snap.ogg",
            SfxId::TileDiscard => "freesound_community-tile-shuffle-99834.ogg",
            SfxId::ScoreReveal => "intake.ogg",
            SfxId::ScoreStep => "vwomp2.ogg",
            SfxId::ScoreTick => "plink3.ogg",
            SfxId::ScoreCrescendo => "scorecrescendo.ogg",
            SfxId::ScoreFinal => "MixingBell.ogg",
            SfxId::RelicPickup => "relic_pickup.ogg",
            SfxId::InvalidAction => "sfx/metal_bonk1.ogg",
            SfxId::CoinDrop => "coindrop.ogg",
            SfxId::RoundWin => "roundwin.ogg",
            SfxId::GameOver => "gameover.ogg",
            SfxId::PackBuy => "sfx/sfx3.ogg",
            SfxId::PackOpen => "pack_open.ogg",
            SfxId::PackTileReveal => "sfx/sfx4.ogg",
            SfxId::ZodiacReveal => "zodiac_reveal.ogg",
            SfxId::ZodiacLevelUp => "zodiac_jingle.ogg",
            SfxId::CandleFlareWhoosh => "candle_flareup.ogg",
            SfxId::CandleFlareImpact => "candle_impact.ogg",
            SfxId::StructureCommit => "sfx/sfx5.ogg",
            SfxId::FocusHandTile => "sfx/sfx13.ogg",
            SfxId::FocusButton => "sfx/sfx13.ogg",
            SfxId::FocusConsumable => "sfx/sfx13.ogg",
            SfxId::FocusRelic => "sfx/sfx13.ogg",
            SfxId::FocusPeg => "sfx/sfx13.ogg",
            SfxId::FocusGold => "sfx/sfx13.ogg",
            SfxId::FocusYakuTablet => "sfx/sfx13.ogg",
            SfxId::FocusDora => "sfx/sfx13.ogg",
            SfxId::RoundStart => "sfx/sfx7.ogg",
            SfxId::ChamberSkipped => "sfx/sfx8.ogg",
            SfxId::BrownoutFlicker => "freesound_community-flickeringlight-90411.mp3",
            SfxId::RoomCreak => "sfx/creak1.ogg",
            SfxId::StarShimmer => "sfx/sfx9.ogg",
            SfxId::Purchase => "sfx/sfx31.ogg",
            SfxId::Sell => "sfx/sfx32.ogg",
            SfxId::Pause => "sfx/sfx11.ogg",
            SfxId::Unpause => "sfx/sfx12.ogg",
            SfxId::DoraScored => "sfx/sfx10.ogg",
            SfxId::CashIn => "sfx/sfx30.ogg",
            SfxId::CascadeMerge => "vwomp1.ogg",
            SfxId::CascadeLaunch => "intake.ogg",
            SfxId::CascadeLand => "Snap.ogg",
            SfxId::YakuTanyao => "yaku/tanyao.ogg",
            SfxId::YakuToitoi => "yaku/toitoi.ogg",
            SfxId::YakuFullHand => "yaku/full_hand.ogg",
            SfxId::YakuYakuhai => "yaku/yakuhai.ogg",
            SfxId::YakuIipeikou => "yaku/iipeikou.ogg",
            SfxId::YakuSanshokuDoujun => "yaku/sanshoku_doujun.ogg",
            SfxId::YakuIttsu => "yaku/ittsu.ogg",
            SfxId::YakuHonitsu => "yaku/honitsu.ogg",
            SfxId::YakuChinitsu => "yaku/chinitsu.ogg",
            SfxId::YakuJunchan => "yaku/junchan.ogg",
            SfxId::YakuHonroutou => "yaku/honroutou.ogg",
            SfxId::YakuChiitoitsu => "yaku/chiitoitsu.ogg",
            SfxId::YakuKokushiMusou => "yaku/kokushi_musou.ogg",
            SfxId::YakuChickenHand => "yaku/chicken_hand.ogg",
            SfxId::YakuChanta => "yaku/chanta.ogg",
            SfxId::YakuRyanpeikou => "yaku/ryanpeikou.ogg",
            SfxId::YakuSanshokuDoukou => "yaku/sanshoku_doukou.ogg",
            SfxId::YakuPinfu => "yaku/pinfu.ogg",
            SfxId::Victory => "victory.ogg",
            SfxId::Victory2 => "victory2.ogg",
            SfxId::Defeat => "defeat.ogg",
            SfxId::NewGameStinger => "sfx/gong_swell1.ogg",
            SfxId::ProductionLogo => "loading/zelda_built_this.ogg",
            SfxId::MainMenuEnter => "mahjuro.ogg",
            SfxId::StairwayEnter => "imgmidi-bell-transition-1-305458.mp3",
            SfxId::ShopEnter => "sfx/door_chimes1.ogg",
            SfxId::LevelUp => "levelup.ogg",
            SfxId::TilesDestroyed => "tiles_destroyed.ogg",
            SfxId::OrdealEncountered => "ordeal_encountered.ogg",
            SfxId::OrdealDefeated => "ordeal_defeated.ogg",
            SfxId::TalismanPurchased => "talisman_purchased.ogg",
            SfxId::TalismanUsed => "talisman_used.ogg",
            SfxId::RollersSpin => "sfx/bike_spin.ogg",
            SfxId::HoldWindup => "sfx/reel_up.ogg",
        }
    }

    /// When set, [`AudioManager::play_sfx`] picks uniformly from this pool
    /// (no immediate repeat of the same variant).
    pub(crate) fn variant_filenames(self) -> Option<&'static [&'static str]> {
        match self {
            SfxId::TileClick => Some(&[
                "sfx/tile_click-01.ogg",
                "sfx/tile_click-02.ogg",
                "sfx/tile_click-03.ogg",
                "sfx/tile_click-04.ogg",
                "sfx/tile_click-05.ogg",
                "sfx/tile_click-06.ogg",
                "sfx/tile_click-07.ogg",
                "sfx/tile_click-08.ogg",
            ]),
            SfxId::InvalidAction => Some(&[
                "sfx/metal_bonk1.ogg",
                "sfx/metal_bonk2.ogg",
                "sfx/metal_bonk3.ogg",
                "sfx/metal_bonk4.ogg",
                "sfx/metal_bonk5.ogg",
            ]),
            SfxId::RoomCreak => Some(&[
                "sfx/creak1.ogg",
                "sfx/creak2.ogg",
                "sfx/creak3.ogg",
                "sfx/creak4.ogg",
                "sfx/creak5.ogg",
                "sfx/creak6.ogg",
                "sfx/creak7.ogg",
                "sfx/creak8.ogg",
                "sfx/creak9.ogg",
            ]),
            SfxId::NewGameStinger => Some(&[
                "sfx/gong_swell1.ogg",
                "sfx/gong_swell2.ogg",
                "sfx/gong_swell3.ogg",
                "sfx/gong_swell4.ogg",
                "sfx/gong_swell5.ogg",
            ]),
            SfxId::ShopEnter => Some(&[
                "sfx/door_chimes1.ogg",
                "sfx/door_chimes2.ogg",
                "sfx/door_chimes3.ogg",
                "sfx/door_chimes4.ogg",
            ]),
            SfxId::CashIn => Some(&[
                "sfx/sfx30.ogg",
                "sfx/sfx33.ogg",
                "sfx/sfx34.ogg",
                "sfx/sfx35.ogg",
                "sfx/sfx36.ogg",
            ]),
            SfxId::FocusHandTile
            | SfxId::FocusButton
            | SfxId::FocusConsumable
            | SfxId::FocusRelic
            | SfxId::FocusPeg
            | SfxId::FocusGold
            | SfxId::FocusYakuTablet
            | SfxId::FocusDora => Some(FOCUS_THUDS),
            _ => None,
        }
    }
}

/// Maximum number of SFX sinks allowed to ring simultaneously. Rodio mixes
/// detached sinks by summing samples with no limiter, so uncapped pileups
/// (e.g. a candle-flare whoosh + impact overlapping a burst of cascade
/// ticks, each of which itself layers two sounds) clip and smear into mud.
/// When the cap is hit, the oldest live sink is dropped to make room.
const MAX_CONCURRENT_SFX: usize = 16;

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
    sfx_data: FxHashMap<SfxId, Arc<PcmClip>>,
    /// Randomized variant pools (see [`SfxId::variant_filenames`]).
    sfx_variant_data: FxHashMap<SfxId, Vec<Arc<PcmClip>>>,
    /// Last variant index per pool — avoids back-to-back repeats.
    sfx_last_variant: FxHashMap<SfxId, usize>,
    music_data: FxHashMap<MusicId, Arc<PcmClip>>,
    /// Per-relic trigger samples decoded on first [`Self::play_relic_trigger`].
    /// Missing entries fall back to [`SfxId::ScoreStep`].
    relic_trigger_data: FxHashMap<crate::core::relic::RelicId, Arc<PcmClip>>,
    /// Live sinks, FIFO-ordered (oldest first). Finished sinks are swept
    /// each `play_sfx` call; when the cap is hit, the oldest is dropped.
    active_sinks: Vec<Sink>,
    /// Sfx queued for future playback with their due time, kept in
    /// ascending time order. Drained each frame by [`AudioManager::tick`]
    /// as entries come due. Used to stagger stacked stingers (e.g. yaku
    /// on a multi-yaku commit) so they roll out one after another.
    pending_sfx: Vec<(Instant, SfxId)>,
    /// Looping BGM queued to start at a future instant (e.g. after shop door chimes).
    pending_music: Vec<(Instant, MusicId)>,
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
    /// True while a one-shot sting owns `music_sink`. `set_music_track` /
    /// `stop_background_music` defer their effect until it empties (handled in
    /// [`AudioManager::tick`]) so the sting isn't cut off by scene transitions.
    jingle_active: bool,
    /// Background track to start once the active jingle finishes. `None`
    /// means "stop music when the jingle ends".
    pending_post_jingle_music: Option<MusicId>,
    ambient_data: FxHashMap<AmbientId, Arc<PcmClip>>,
    /// Looping environmental beds (separate from BGM + SFX), one sink each.
    ambient_sinks: FxHashMap<AmbientId, Sink>,
    ambient_active: Vec<AmbientId>,
    /// Dedicated sink for looping one-shot SFX beds (e.g. score rollers).
    sfx_loop_sink: Option<Sink>,
    sfx_loop_active: Option<SfxId>,
    sfx_loop_speed: f32,
    /// Hold-to-act windup (`reel_up`); stopped on early release.
    hold_windup_sink: Option<Sink>,
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

        let mut sfx_data = FxHashMap::default();
        let mut sfx_variant_data = FxHashMap::default();
        for &sfx_id in all_sfx_ids() {
            if let Some(paths) = sfx_id.variant_filenames() {
                let mut clips = Vec::with_capacity(paths.len());
                for path in paths {
                    let asset_path = format!("audio/{path}");
                    if let Some(file) = crate::asset_path::get(&asset_path) {
                        let label = format!("{asset_path} ({sfx_id:?} variant)");
                        if let Some(clip) = decode_rodio(&label, file.data.as_ref()) {
                            clips.push(clip);
                        }
                    }
                }
                if !clips.is_empty() {
                    sfx_variant_data.insert(sfx_id, clips);
                }
            }
            let asset_path = format!("audio/{}", sfx_id.filename());
            if sfx_id.variant_filenames().is_some() {
                continue;
            }
            if let Some(file) = crate::asset_path::get(&asset_path) {
                let label = format!("{asset_path} ({sfx_id:?})");
                if let Some(clip) = decode_rodio(&label, file.data.as_ref()) {
                    sfx_data.insert(sfx_id, clip);
                }
            }
        }

        if sfx_data.is_empty() && sfx_variant_data.is_empty() {
            log::warn!("No audio files found in assets/audio/. Sound effects disabled.");
        } else {
            log::debug!(
                "Loaded {} sound effect(s) ({} with variants).",
                sfx_data.len(),
                sfx_variant_data.len(),
            );
        }

        let hold_secs = sfx_data
            .get(&SfxId::HoldWindup)
            .map(|clip| clip_duration_at_speed(clip, 1.0).as_secs_f32())
            .unwrap_or(crate::ui::prompt_hold_ring::HOLD_ACT_FALLBACK_SECONDS);
        crate::ui::prompt_hold_ring::set_hold_act_seconds(hold_secs);

        let music_data = FxHashMap::default();

        Self {
            _stream: stream,
            handle,
            sfx_data,
            sfx_variant_data,
            sfx_last_variant: FxHashMap::default(),
            music_data,
            relic_trigger_data: FxHashMap::default(),
            active_sinks: Vec::with_capacity(MAX_CONCURRENT_SFX),
            pending_sfx: Vec::new(),
            pending_music: Vec::new(),
            master_volume: crate::persistence::VOLUME_UNITY,
            sfx_volume: crate::persistence::VOLUME_UNITY,
            music_volume: crate::persistence::VOLUME_UNITY,
            sfx_enabled: true,
            music_sink: None,
            last_music: None,
            music_active_id: None,
            jingle_active: false,
            pending_post_jingle_music: None,
            ambient_data: FxHashMap::default(),
            ambient_sinks: FxHashMap::default(),
            ambient_active: Vec::new(),
            sfx_loop_sink: None,
            sfx_loop_active: None,
            sfx_loop_speed: 1.0,
            hold_windup_sink: None,
        }
    }

    /// Play the hold-to-act windup on a dedicated sink (restarts if already playing).
    pub fn play_hold_windup(&mut self) {
        if !self.sfx_enabled {
            self.stop_hold_windup();
            return;
        }
        let Some(clip) = self.sfx_data.get(&SfxId::HoldWindup).cloned() else {
            log::debug!("play_hold_windup: no data");
            return;
        };
        let Some(handle) = &self.handle else {
            return;
        };
        if let Some(sink) = self.hold_windup_sink.take() {
            sink.stop();
        }
        let Ok(sink) = Sink::try_new(handle) else {
            log::warn!("play_hold_windup: sink creation failed");
            return;
        };
        let effective_vol = self.master_volume * self.sfx_volume;
        let source = SharedPcmSource::new(clip);
        sink.set_volume(effective_vol);
        sink.append(source);
        self.hold_windup_sink = Some(sink);
    }

    /// Halt the hold windup when the player releases before the action completes.
    pub fn stop_hold_windup(&mut self) {
        if let Some(sink) = self.hold_windup_sink.take() {
            sink.stop();
        }
    }

    /// Play the cascade tick at one of [`SCORE_TICK_PITCHES`] stepped speeds,
    /// indexed by `step`. Each speed shifts the base sample by one semitone,
    /// so the cascade climbs a major-scale-ish run as scoring unfolds.
    pub fn play_score_tick(&mut self, step: usize) {
        let speed = SCORE_TICK_PITCHES[step % SCORE_TICK_PITCHES.len()];
        self.play_sfx_with_speed(SfxId::ScoreTick, speed);
    }

    /// Nominal playback length for a loaded SFX clip at 1.0 speed (no sink started).
    pub fn sfx_duration(&self, id: SfxId) -> Option<Duration> {
        self.sfx_duration_with_speed(id, 1.0)
    }

    fn sfx_duration_with_speed(&self, id: SfxId, speed: f32) -> Option<Duration> {
        if let Some(variants) = self.sfx_variant_data.get(&id) {
            return variants
                .first()
                .map(|clip| clip_duration_at_speed(clip, speed));
        }
        self.sfx_data
            .get(&id)
            .map(|clip| clip_duration_at_speed(clip, speed))
    }

    /// Play a sound effect. No-op if audio is unavailable or the SFX file wasn't loaded.
    /// Returns the clip duration (adjusted for `speed`) when playback started.
    pub fn play_sfx(&mut self, id: SfxId) -> Option<Duration> {
        self.play_sfx_with_speed(id, 1.0)
    }

    /// Queue `id` for playback at `when`. Pair with [`AudioManager::tick`]
    /// to actually fire it; used to stagger stacked stingers so they land
    /// as a sequence instead of overlapping.
    pub fn schedule_sfx(&mut self, id: SfxId, when: Instant) {
        self.pending_sfx.push((when, id));
    }

    /// Queue looping BGM to start at `when`. Clears any earlier scheduled track.
    /// Stops whatever loop is on the music sink now (e.g. main-menu BGM during
    /// shop door chimes) without cancelling this schedule. While a win/loss jingle
    /// owns the sink, suppresses the jingle's post-hand-off so this schedule is
    /// the sole source of the next loop.
    pub fn schedule_music_track(&mut self, when: Instant, id: MusicId) {
        debug_assert!(id.is_loop(), "use play_music_jingle for one-shot tracks");
        self.pending_music.clear();
        self.last_music = Some(id);
        if self.jingle_active {
            self.pending_post_jingle_music = None;
        } else {
            self.stop_active_looping_music();
        }
        self.pending_music.push((when, id));
    }

    /// Drain any scheduled sfx whose time has arrived. Call once per frame.
    /// Also detects when an active one-shot jingle has finished and resumes
    /// the deferred background loop (or stops music) accordingly.
    pub fn tick(&mut self, now: Instant) {
        while self.pending_sfx.first().is_some_and(|(t, _)| *t <= now) {
            let (_, id) = self.pending_sfx.remove(0);
            self.play_sfx(id);
        }

        while self.pending_music.first().is_some_and(|(t, _)| *t <= now) {
            let (_, id) = self.pending_music.remove(0);
            self.set_music_track(id);
        }

        if self.jingle_active {
            let finished = self.music_sink.as_ref().map(|s| s.empty()).unwrap_or(true);
            if finished {
                self.jingle_active = false;
                if let Some(sink) = self.music_sink.take() {
                    sink.stop();
                }
                self.music_active_id = None;
                if let Some(next) = self.pending_post_jingle_music.take() {
                    log::debug!("jingle finished — resuming {next:?}");
                    self.start_music_track(next);
                }
            }
        }
    }

    fn play_sfx_with_speed(&mut self, id: SfxId, speed: f32) -> Option<Duration> {
        if let Some(len) = self.sfx_variant_data.get(&id).map(|v| v.len()) {
            if len == 0 {
                log::debug!("play_sfx({id:?}): no variant data");
                return None;
            }
            let idx = self.pick_variant_index(id, len);
            let Some(clip) = self
                .sfx_variant_data
                .get(&id)
                .and_then(|v| v.get(idx))
                .cloned()
            else {
                log::debug!("play_sfx({id:?}): variant index out of range");
                return None;
            };
            return self.play_clip(&format!("{id:?}[{idx}]"), clip, speed);
        }
        let Some(clip) = self.sfx_data.get(&id).cloned() else {
            log::debug!("play_sfx({id:?}): no data");
            return None;
        };
        self.play_clip(&format!("{id:?}"), clip, speed)
    }

    fn pick_variant_index(&mut self, id: SfxId, len: usize) -> usize {
        use rand::RngExt;
        if len <= 1 {
            return 0;
        }
        let mut rng = rand::rng();
        let mut idx = rng.random_range(0..len);
        if self.sfx_last_variant.get(&id).copied() == Some(idx) {
            idx = (idx + 1) % len;
        }
        self.sfx_last_variant.insert(id, idx);
        idx
    }

    /// Decode and cache a relic trigger clip on first use.
    fn ensure_relic_trigger(&mut self, rid: crate::core::relic::RelicId) -> Option<Arc<PcmClip>> {
        if let Some(clip) = self.relic_trigger_data.get(&rid) {
            return Some(clip.clone());
        }
        let slug = rid.asset_filename().trim_end_matches(".png");
        let asset_path = format!("audio/relics/{slug}.ogg");
        let file = crate::asset_path::get(&asset_path)?;
        let clip = decode_rodio(&asset_path, file.data.as_ref())?;
        self.relic_trigger_data.insert(rid, clip.clone());
        Some(clip)
    }

    /// Play the per-relic trigger stinger for `rid`. Falls back to
    /// [`SfxId::ScoreStep`] when no `audio/relics/<slug>.ogg` exists.
    pub fn play_relic_trigger(&mut self, rid: crate::core::relic::RelicId) {
        if let Some(clip) = self.ensure_relic_trigger(rid) {
            self.play_clip(&format!("Relic({rid:?})"), clip, 1.0);
        } else {
            self.play_sfx(SfxId::ScoreStep);
        }
    }

    fn play_clip(&mut self, tag: &str, clip: Arc<PcmClip>, speed: f32) -> Option<Duration> {
        let duration = clip_duration_at_speed(&clip, speed);
        if !self.sfx_enabled {
            log::debug!("play_clip({tag}): sfx disabled");
            return None;
        }
        let Some(handle) = &self.handle else {
            log::debug!("play_clip({tag}): no handle");
            return None;
        };
        let Ok(sink) = Sink::try_new(handle) else {
            log::warn!("play_clip({tag}): sink creation failed");
            return None;
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
        Some(duration)
    }

    /// Set the master volume (0.0 to 1.0).
    pub fn set_master_volume(&mut self, vol: f32) {
        self.master_volume = crate::persistence::clamp_volume(vol);
        self.refresh_music_sink_volume();
        self.refresh_all_ambient_sink_volumes();
        self.refresh_sfx_loop_sink_volume();
        self.refresh_hold_windup_sink_volume();
    }

    /// Set the sound effects volume. Also updates one-shots on `music_sink`
    /// (win/loss stingers, round-start intros) and ambient beds.
    pub fn set_sfx_volume(&mut self, vol: f32) {
        self.sfx_volume = crate::persistence::clamp_volume(vol);
        self.refresh_all_ambient_sink_volumes();
        self.refresh_sfx_loop_sink_volume();
        self.refresh_hold_windup_sink_volume();
        self.refresh_music_sink_volume();
    }

    /// Set the music volume (0.0 to 1.0).
    pub fn set_music_volume(&mut self, vol: f32) {
        self.music_volume = crate::persistence::clamp_volume(vol);
        self.refresh_music_sink_volume();
    }

    /// Enable or disable sound effects. Background music is unaffected.
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.sfx_enabled == enabled {
            return;
        }
        self.sfx_enabled = enabled;
        if enabled {
            let active = self.ambient_active.clone();
            self.set_ambient_tracks(&active);
            if let Some(id) = self.sfx_loop_active {
                self.start_sfx_loop(id, self.sfx_loop_speed);
            }
        } else {
            self.stop_all_ambient_sinks();
            self.stop_sfx_loop_sink();
            self.stop_hold_windup();
        }
        self.refresh_music_sink_volume();
    }

    /// Volume for whatever is on `music_sink`. Looping BGM uses the music
    /// slider; one-shots (win/loss stingers, round-start intros) use SFX.
    fn music_sink_effective_volume(&self) -> f32 {
        if self.jingle_active {
            if !self.sfx_enabled {
                return 0.0;
            }
            self.master_volume * self.sfx_volume
        } else {
            self.master_volume * self.music_volume
        }
    }

    fn refresh_music_sink_volume(&mut self) {
        let Some(sink) = self.music_sink.as_ref() else {
            return;
        };
        sink.set_volume(self.music_sink_effective_volume());
    }

    /// Halt the current looping track on `music_sink` but keep [`Self::last_music`]
    /// and [`Self::pending_music`] (used when shop BGM is deferred after door chimes).
    fn stop_active_looping_music(&mut self) {
        self.music_active_id = None;
        if let Some(sink) = self.music_sink.take() {
            sink.stop();
        }
    }

    /// Stop background music and clear the remembered track (e.g. splash / loading).
    /// While a one-shot jingle is playing, this defers the stop until the
    /// jingle finishes (so blind-loss → game-over transitions don't truncate
    /// the loss jingle).
    pub fn stop_background_music(&mut self) {
        self.pending_music.clear();
        if self.jingle_active {
            self.last_music = None;
            self.pending_post_jingle_music = None;
            return;
        }
        self.last_music = None;
        self.music_active_id = None;
        if let Some(sink) = self.music_sink.take() {
            sink.stop();
        }
    }

    /// Switch background music to `id` (loops). No-op if the asset is missing.
    /// While a one-shot jingle is playing, the change is queued and applied
    /// when the jingle finishes — keeps the celebration / defeat stinger
    /// audible across the post-blind scene transition.
    pub fn set_music_track(&mut self, id: MusicId) {
        debug_assert!(id.is_loop(), "use play_music_jingle for one-shot tracks");
        self.pending_music.clear();
        if self.jingle_active {
            self.pending_post_jingle_music = Some(id);
            self.last_music = Some(id);
            return;
        }
        self.last_music = Some(id);
        self.start_music_track(id);
    }

    /// Start blind BGM: intro once, then the regular or boss loop.
    pub fn set_gameplay_music(&mut self, ordeal_chamber: bool) {
        if ordeal_chamber {
            self.play_music_intro_then_loop(
                MusicId::GameplayIntenseIntro,
                MusicId::GameplayIntense,
            );
        } else {
            self.play_music_intro_then_loop(MusicId::GameplayIntro, MusicId::Gameplay);
        }
    }

    /// Play `intro` once, then loop `loop_id` (round-start sting).
    pub fn play_music_intro_then_loop(&mut self, intro: MusicId, loop_id: MusicId) {
        debug_assert_eq!(intro.loop_after_intro(), Some(loop_id));
        self.last_music = Some(loop_id);
        if !self.sfx_enabled {
            self.start_music_track(loop_id);
            return;
        }
        if self.jingle_active {
            self.pending_post_jingle_music = Some(loop_id);
            return;
        }
        self.ensure_music_loaded(intro);
        let Some(clip) = self.music_data.get(&intro).cloned() else {
            log::debug!("play_music_intro_then_loop({intro:?}): no data — starting {loop_id:?}");
            self.start_music_track(loop_id);
            return;
        };
        let Some(handle) = &self.handle else {
            return;
        };
        if let Some(sink) = self.music_sink.take() {
            sink.stop();
        }
        let Ok(sink) = Sink::try_new(handle) else {
            log::warn!("play_music_intro_then_loop({intro:?}): sink creation failed");
            return;
        };
        sink.set_volume(self.music_sink_effective_volume());
        let source = SharedPcmSource::new(clip);
        sink.append(source);
        self.music_sink = Some(sink);
        self.music_active_id = None;
        self.jingle_active = true;
        self.pending_post_jingle_music = Some(loop_id);
        log::debug!("play_music_intro_then_loop({intro:?}): loop_after={loop_id:?}");
    }

    /// Play `id` once on the music sink (no looping). Replaces whatever
    /// loop or sting is currently on the sink. Once the clip finishes, the
    /// deferred loop ([`Self::set_music_track`] / intro hand-off) resumes from
    /// the start — win/loss jingles use the last loop; round-start intros use
    /// their paired loop. All one-shots obey the SFX volume slider.
    pub fn play_music_jingle(&mut self, id: MusicId) {
        debug_assert!(id.is_one_shot(), "use set_music_track for looping tracks");
        if !self.sfx_enabled {
            return;
        }
        self.ensure_music_loaded(id);
        let Some(clip) = self.music_data.get(&id).cloned() else {
            log::debug!("play_music_jingle({id:?}): no data");
            return;
        };
        let Some(handle) = &self.handle else {
            return;
        };
        if let Some(sink) = self.music_sink.take() {
            sink.stop();
        }
        let Ok(sink) = Sink::try_new(handle) else {
            log::warn!("play_music_jingle({id:?}): sink creation failed");
            return;
        };
        sink.set_volume(self.music_sink_effective_volume());
        let resume_to = id
            .loop_after_intro()
            .or(self.pending_post_jingle_music)
            .or(self.last_music);
        let source = SharedPcmSource::new(clip);
        sink.append(source);
        self.music_sink = Some(sink);
        self.music_active_id = None;
        self.jingle_active = true;
        self.pending_post_jingle_music = resume_to;
        log::debug!(
            "play_music_jingle({id:?}): resume_to={:?}",
            self.pending_post_jingle_music
        );
    }

    fn ensure_music_loaded(&mut self, id: MusicId) {
        if self.music_data.contains_key(&id) {
            return;
        }
        let path = id.asset_path();
        if let Some(file) = crate::asset_path::get(path)
            && let Some(clip) = decode_rodio(path, file.data.as_ref())
        {
            log::debug!("Loaded BGM {path} ({id:?})");
            self.music_data.insert(id, clip);
        }
    }

    /// Decode round BGM clips ahead of the hallway → gameplay transition.
    pub fn prefetch_gameplay_music(&mut self) {
        for id in [
            MusicId::GameplayIntro,
            MusicId::Gameplay,
            MusicId::GameplayIntenseIntro,
            MusicId::GameplayIntense,
        ] {
            self.ensure_music_loaded(id);
        }
    }

    fn start_music_track(&mut self, id: MusicId) {
        self.ensure_music_loaded(id);
        let Some(clip) = self.music_data.get(&id).cloned() else {
            log::debug!("start_music_track({id:?}): no data");
            return;
        };
        let Some(handle) = &self.handle else {
            return;
        };
        if let Some(sink) = self.music_sink.as_ref()
            && !sink.empty()
            && self.music_active_id == Some(id)
        {
            self.refresh_music_sink_volume();
            return;
        }
        if let Some(sink) = self.music_sink.take() {
            sink.stop();
        }
        let Ok(sink) = Sink::try_new(handle) else {
            log::warn!("start_music_track({id:?}): sink creation failed");
            return;
        };
        sink.set_volume(self.music_sink_effective_volume());
        let source = LoopingPcmSource::new(clip);
        sink.append(source);
        self.music_sink = Some(sink);
        self.music_active_id = Some(id);
    }

    /// Replace the active ambient bed set (e.g. rain + bulb buzz on main menu).
    pub fn set_ambient_tracks(&mut self, ids: &[AmbientId]) {
        let desired: rustc_hash::FxHashSet<AmbientId> = ids.iter().copied().collect();
        self.ambient_sinks.retain(|id, sink| {
            if desired.contains(id) {
                true
            } else {
                sink.stop();
                false
            }
        });
        self.ambient_active = ids.to_vec();
        if !self.sfx_enabled {
            self.stop_all_ambient_sinks();
            return;
        }
        for &id in ids {
            if self.ambient_sinks.contains_key(&id) {
                self.refresh_ambient_sink_volume(id);
            } else {
                self.start_ambient_sink(id);
            }
        }
    }

    fn ensure_ambient_loaded(&mut self, id: AmbientId) {
        if self.ambient_data.contains_key(&id) {
            return;
        }
        let path = id.asset_path();
        if let Some(file) = crate::asset_path::get(path)
            && let Some(clip) = decode_rodio(path, file.data.as_ref())
        {
            log::debug!("Loaded ambient {path} ({id:?})");
            self.ambient_data.insert(id, clip);
        }
    }

    fn ambient_effective_volume(&self, id: AmbientId) -> f32 {
        if !self.sfx_enabled {
            return 0.0;
        }
        self.master_volume * self.sfx_volume * id.gain()
    }

    fn refresh_ambient_sink_volume(&mut self, id: AmbientId) {
        let Some(sink) = self.ambient_sinks.get(&id) else {
            return;
        };
        sink.set_volume(self.ambient_effective_volume(id));
    }

    fn refresh_all_ambient_sink_volumes(&mut self) {
        let ids: Vec<AmbientId> = self.ambient_sinks.keys().copied().collect();
        for id in ids {
            self.refresh_ambient_sink_volume(id);
        }
    }

    fn stop_all_ambient_sinks(&mut self) {
        for (_, sink) in self.ambient_sinks.drain() {
            sink.stop();
        }
    }

    fn start_ambient_sink(&mut self, id: AmbientId) {
        self.ensure_ambient_loaded(id);
        let Some(clip) = self.ambient_data.get(&id).cloned() else {
            log::debug!("start_ambient_sink({id:?}): no data");
            return;
        };
        let Some(handle) = &self.handle else {
            return;
        };
        if let Some(sink) = self.ambient_sinks.get(&id)
            && !sink.empty()
        {
            self.refresh_ambient_sink_volume(id);
            return;
        }
        if let Some(sink) = self.ambient_sinks.remove(&id) {
            sink.stop();
        }
        let Ok(sink) = Sink::try_new(handle) else {
            log::warn!("start_ambient_sink({id:?}): sink creation failed");
            return;
        };
        sink.set_volume(self.ambient_effective_volume(id));
        sink.append(LoopingPcmSource::new(clip));
        self.ambient_sinks.insert(id, sink);
    }

    /// Start or stop a looping SFX bed (uses the SFX volume slider).
    pub fn set_sfx_loop(&mut self, id: Option<SfxId>, speed: f32) {
        match id {
            Some(id) => self.start_sfx_loop(id, speed.max(0.01)),
            None => self.stop_sfx_loop(),
        }
    }

    fn sfx_loop_effective_volume(&self) -> f32 {
        if !self.sfx_enabled {
            return 0.0;
        }
        self.master_volume * self.sfx_volume
    }

    fn refresh_sfx_loop_sink_volume(&mut self) {
        let Some(sink) = self.sfx_loop_sink.as_ref() else {
            return;
        };
        sink.set_volume(self.sfx_loop_effective_volume());
    }

    fn refresh_hold_windup_sink_volume(&mut self) {
        let Some(sink) = self.hold_windup_sink.as_ref() else {
            return;
        };
        let vol = if self.sfx_enabled {
            self.master_volume * self.sfx_volume
        } else {
            0.0
        };
        sink.set_volume(vol);
    }

    fn stop_sfx_loop_sink(&mut self) {
        if let Some(sink) = self.sfx_loop_sink.take() {
            sink.stop();
        }
    }

    fn stop_sfx_loop(&mut self) {
        self.sfx_loop_active = None;
        self.sfx_loop_speed = 1.0;
        self.stop_sfx_loop_sink();
    }

    fn start_sfx_loop(&mut self, id: SfxId, speed: f32) {
        self.sfx_loop_active = Some(id);
        self.sfx_loop_speed = speed;
        if !self.sfx_enabled {
            self.stop_sfx_loop_sink();
            return;
        }
        let Some(clip) = self.sfx_data.get(&id).cloned() else {
            log::debug!("start_sfx_loop({id:?}): no data");
            return;
        };
        let Some(handle) = &self.handle else {
            return;
        };
        if let Some(sink) = self.sfx_loop_sink.as_ref()
            && !sink.empty()
            && self.sfx_loop_active == Some(id)
        {
            sink.set_speed(speed);
            self.refresh_sfx_loop_sink_volume();
            return;
        }
        if let Some(sink) = self.sfx_loop_sink.take() {
            sink.stop();
        }
        let Ok(sink) = Sink::try_new(handle) else {
            log::warn!("start_sfx_loop({id:?}): sink creation failed");
            return;
        };
        sink.set_volume(self.sfx_loop_effective_volume());
        sink.set_speed(speed);
        sink.append(LoopingPcmSource::new(clip));
        self.sfx_loop_sink = Some(sink);
    }
}
