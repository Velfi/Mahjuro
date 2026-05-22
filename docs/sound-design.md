---
name: Mahjuro sound design document
description: Blueprint for SFX, ambience, and adaptive audio in Mahjuro — locks the audio already shipped, scopes everything still placeholder
type: design-doc
---

# Mahjuro — Sound Design Document

This is the source of truth for what Mahjuro sounds like, who is responsible for what, and how new audio enters the build. It is structured per the public GSDD outline at <https://gamedesignskills.com/game-design/sound/> (Vision → Assets → Branding → Tech → Interactive systems → Implementation → Pipeline), fitted to the systems we already have in code.

The runtime audio engine and its full event catalogue are documented inline in source; cite from there rather than duplicating in this doc:
- Engine + mixer: [src/audio.rs](../src/audio.rs) — `AudioManager`, `SfxId`, `MusicId`, `MAX_CONCURRENT_SFX`.
- Event dispatch: [src/main/frame_tick.rs](../src/main/frame_tick.rs) — every `GameEvent` → `play_sfx` mapping.
- Scene → music policy: [`sync_music_for_scene` in src/main/scene_transition.rs](../src/main/scene_transition.rs).
- Audio settings UI: [src/scenes/options.rs](../src/scenes/options.rs) — Master / Music / SFX sliders + SFX on/off.

---

## 1. Vision

Mahjuro is a single-player mahjong roguelite played on a quiet wooden table in a warm temple/parlor space. The audio identity is **"contemplative table, generous cascade"**:

- **Diegetic tile world.** Hand actions sound like the physical objects on the table: porcelain-on-felt taps, the swept rush of a discard pile, the dry click of a tile rotated in inspect. The player should believe they could close their eyes and still know which tile they grabbed.
- **Musical reward.** The scoring cascade is the only place audio leaves the table and becomes openly musical — a short, layered, in-key climb that pays off the player's decisions. The chromatic semitone climb already wired into `ScoreTick` (`SCORE_TICK_PITCHES`, [src/audio.rs](../src/audio.rs)) is the seed of that identity.
- **Restraint over coverage.** Mahjuro is a long-session game; nothing in the UI loop should be fatiguing after the 200th repetition. Prefer one short, soft sample with a clean envelope over a longer/louder cue, even where the article would suggest a fuller stinger.
- **Silence is a tool.** The Shop and Pick-Blind moments are paced. Music continues, but UI cues there are quieter than gameplay UI cues — the player is reading and deciding, not reacting.

The negative space: no whoosh-laden modern-mobile UX kit, no orchestral hits on routine wins, no voice barks, no "epic" boss screams, no Skinner-box success jingles after small actions.

---

## 2. Asset classification

Mahjuro has one canonical SFX enum and one music enum. Both are exhaustively listed and routed in `src/audio.rs`; this section organizes them by their **design role** rather than by file location.

### 2.1 Locked (do not replace without explicit approval)

These cues are considered final. They define the identity that everything else must sit alongside.

| Role | Sample | Used for |
|---|---|---|
| BGM — Main Menu / Collection | `audio/music/main_menu.ogg` | `MusicId::MainMenu` |
| BGM — Gameplay (regular) | `audio/music/gameplay_intro.ogg` → `gameplay.ogg` | `MusicId::GameplayIntro` → `Gameplay` |
| BGM — Gameplay (boss) | `audio/music/gameplay_intense_intro.ogg` → `gameplay_intense.ogg` | `MusicId::GameplayIntenseIntro` → `GameplayIntense` |
| BGM — Shop / Pick Blind / Hallway | `audio/music/shop.ogg` | `MusicId::Shop` |
| Blind / boss win stinger | `audio/music/blind_win.ogg`, `boss_win.ogg` | `MusicId::BlindWin`, `BossWin` |
| Blind / boss loss stinger | `audio/music/blind_loss.ogg`, `boss_loss.ogg` | `MusicId::BlindLoss`, `BossLoss` |
| Pack open | `audio/pack_open.ogg` | `SfxId::PackOpen` |
| Zodiac acquire | `audio/zodiac_reveal.ogg` | `SfxId::ZodiacReveal` |
| Talisman trigger | `audio/talisman_used.ogg` | `SfxId::TalismanUsed` |
| Cash in | `audio/cash_in.ogg` | `SfxId::CashIn` |
| Tile discard sweep | `audio/freesound_community-tile-shuffle-99834.ogg` | `SfxId::TileDiscard` |
| Tile click (mouse pick) | `audio/kenney_interface-sounds/Audio/drop_003.ogg` | `SfxId::TileClick` |
| Per-relic activation stingers | `audio/relics/<slug>.ogg` (82 today) | `play_relic_trigger(rid)` |
| Per-yaku scoring stingers | `audio/yaku_<kind>.ogg` (13 today) | `SfxId::Yaku*` via `SfxId::for_yaku` |

`TileClick` overlapping the Kenney UI pack is intentional — the existing sample sits well next to the discard and the per-tile snap, so it stays even though its source kit is otherwise placeholder.

### 2.2 Hand & table (gameplay diegetic)

The "table world." These sounds should feel like one physical object hitting another. Materials are porcelain glaze on a felt surface with bamboo edging.

- **`TilePlace` / `CascadeLand`** — tile settles into hand or score reel. Today: `Snap.ogg`. Keep, but consider 3 randomized variants (`Snap_a/b/c.ogg`) at ±1 dB and ±2% speed to defeat the 200th-repeat fatigue. Selection logic: `play_clip` picks any of `Snap_*` if multiple exist (loader change in `src/audio.rs`).
- **`TileSelect` / `TileDeselect`** — focus moves through the hand row. Today: `kenney_interface-sounds/tick_002.ogg` / `tick_004.ogg`. Replace with two **felt thumps** with a very short attack (≤5 ms) and ~30 ms decay. The select cue is one semitone above deselect to give directionality. Loudness: ≤ −18 LUFS short-term, well under `TilePlace`.
- **`StructureCommit`** — meld locks into the mirror tray. Currently `confirmation_002.ogg` (placeholder). Design as a **soft chord-of-2 taps** (the meld is a set of tiles meeting): two `TilePlace`-adjacent samples played 18 ms apart in mono, mixed −3 dB below a single `TilePlace`. The closeness of the two taps reads as "they belong together."
- **`InvalidAction`** — bad meld, full structure, no charges. Today: `invalid.ogg`. Acceptable, but the file is too long and slightly cartoony. Replace with a **muted, dampened tap** — like a tile half-rejected by the tray. ≤120 ms total. No descending pitch; the cue is "stopped," not "wrong."
- **`TilesDestroyed`** *(no asset yet — silently no-ops)*. Needed for Taotie / curse interactions where tiles are permanently removed. Design as a **dry shatter against cloth** — a single splintery crack followed by an immediate damped tail, ~250 ms. No glass tinkle; tiles in Mahjuro are porcelain *on felt*, not on stone.
- **`CashIn`** — player commits the meld and the cascade begins. Today: `cash_in.ogg`. Keep, but plan to **duck music −3 dB for 250 ms** when it fires so the cascade reveal that follows has air.

### 2.3 Scoring cascade (musical)

This is the only sustained musical sequence in the moment-to-moment loop. It is treated as a small composition:

1. **`ScoreReveal`** — `intake.ogg`. The inhale before the climb.
2. **`ScoreTick`** layered with **`ScoreStep`** — per reveal beat. The eight-semitone chromatic climb on `ScoreTick` (`SCORE_TICK_PITCHES` in `src/audio.rs`) is the spine and should not be lost.
3. **`DoraScored`** — per dora tile, 180 ms staggered. Today: `glass_002.ogg`. Replace with a **single struck-bell tine** at a fixed pitch (suggestion: a perfect fifth above the cascade root). It is a recurring decoration; pitch must be in-key with the music or the climb falls apart.
4. **`YakuTanyao..ChickenHand`** — per yaku, 200 ms staggered. The 13 existing stingers are locked. **`YakuKokushiMusou` is missing** — produce a final-tier stinger that is recognizably bigger than `YakuChinitsu` (the current top end) but still fits the 200 ms slot without bleeding into the next yaku.
5. **`CascadeMerge` / `CascadeLaunch` / `CascadeLand`** — the chips × mult merge → fly → settle into the reel. Today: `vwomp1`, `intake`, `Snap`. Keep `CascadeLand = Snap` (the totals are a tile-sized object landing). `CascadeMerge` and `CascadeLaunch` are placeholder; design them as **one continuous gesture in two parts** — a pitched-up swirl that resolves into a release whoosh. Same instrument, two segments.
6. **`ScoreCrescendo` + `ScoreFinal`** — the totals lock in. Both placeholders; design as a **single two-beat resolution** in the cascade's key, with `ScoreFinal` falling slightly off-beat to act as a comma rather than a period.
7. **`CandleFlareWhoosh` + `CandleFlareImpact`** — single-hand blind clear. Today: `candle_flareup.ogg`, `candle_impact.ogg`. Keep.

**Hard constraint:** every recurring cascade sample (`ScoreTick`, `DoraScored`, `ScoreStep`) must be tonally neutral enough to layer with *any* of the three BGM tracks in their respective keys. The cascade does not pause music; it plays *over* music. This is why those samples are tuned percussion (clave, finger cymbal, kalimba-type tine), not pitched melodic instruments.

### 2.4 Economy

- **`CoinDrop`** (also `Purchase`) — generic gold movement. Today: `coindrop.ogg`. Keep.
- **`CashIn`** — see §2.2.
- **`Sell`** — today: `confirmation_002.ogg`. Replace with **a single coin drop *into* a wooden box** (i.e. coin sample + soft wooden enclosure tail) so it is audibly distinguishable from `CoinDrop`. Sell is the player *receiving* gold; the timbre should reward.
- **`PackBuy`** — wallet → counter. Today: `confirmation_003.ogg`. Replace with a **paper-wrapped slap on the counter** — one short hand-flesh-on-wood thump with a paper-rustle layer.
- **`PackOpen`** — paper tear. Shipped: `pack_open.ogg`.
- **`PackTileReveal`** — per tile in the pack. Today: `pluck_001.ogg`. Replace with a **dry, soft tile flip** — a smaller cousin of `TilePlace`, half its body, more click than thump. This is recurring (up to 5 in a row) and must not fatigue.
- **`ZodiacReveal`** — close-up reveal of a zodiac ribbon. Shipped: `zodiac_reveal.ogg`.
- **`ZodiacLevelUp`** — yaku permanently leveled. Today: `zodiac_jingle.ogg`. Keep — it is one of the few places the game gets to play a true jingle, and the current cue earns it.
- **`StarShimmer`** — celebration scene transition. Today: `glass_005.ogg`. Acceptable; revisit only if it clashes with the cascade hand-off.
- **`TalismanPurchased`** *(no asset yet)* — brush-stroke on paper + soft landing thump.
- **`TalismanUsed`** — shipped: `talisman_used.ogg`.

### 2.5 Round / blind / run lifecycle

- **`RoundStart`** — today: `confirmation_004.ogg`. Replace with a **gong-adjacent struck idiophone**, single soft hit, long resonant decay (~1.2 s). It is the curtain rising; it can afford length.
- **`RoundWin` / `Victory` / `Victory2`** — today: `roundwin.ogg`, `victory.ogg`, `victory2.ogg`. Keep; the two-victory randomization is the only randomization Mahjuro currently does and it is working.
- **`GameOver` / `Defeat`** — today: `gameover.ogg`, `defeat.ogg`. Keep.
- **`LevelUp`** — today: `levelup.ogg`. Keep.
- **`MainMenuEnter`** — today: `mahjuro.ogg` logo sting. Keep, locked.
- **`BossEncountered`** *(no asset yet)*. Design as a **single low wooden temple-block thud** (a `mokugyo`-style hit) with a soft, brief shakuhachi-style breath layer. ~700 ms. One sample for all 23 bosses. Per-boss differentiation lives in BGM filter & visual, not in a unique cue per boss — that path is content-prohibitive.
- **`BossDefeated`** *(no asset yet)*. Same temple-block instrument as `BossEncountered`, struck *twice* (call-and-resolve), with the second hit dampened. Reuses the instrument so player can hear the encounter→defeat arc as the same object book-ending the fight.

### 2.6 UI / chrome

This is the largest placeholder surface. Every `kenney_interface-sounds/*` cue is temporary.

- **`UiConfirm`** — every "OK / select / advance." Replace with a **single felt-against-bamboo tick**. Brighter than `TileSelect`. ≤80 ms. Loudness ≤ −20 LUFS short-term.
- **`UiCancel`** — every back / dismiss. The inverse: same instrument, one semitone down, slightly shorter, no brightness boost.
- **`FocusButton` / `FocusHandTile` / `FocusConsumable` / `FocusRelic` / `FocusPeg` / `FocusGold` / `FocusYakuTablet` / `FocusDora`** — currently eight different Kenney samples, one per focusable category. Mahjuro routes these through `Tree<A>::on_focus_changed`, so the system already exists; the **design intent** is "different categories of UI surface have different fingertips on them." Replacement plan:
  - One **base "focus tick" sample** (a single soft pad press).
  - **One short tail layer per category**, mixed at −12 dB under the base:
    - `FocusButton` — clean tick, no tail.
    - `FocusHandTile` — same tick + faint porcelain rim sympathetic-resonance tail (re-uses `TilePlace` body, 200 ms decayed to silence).
    - `FocusConsumable` — same tick + paper rustle.
    - `FocusRelic` — same tick + soft finger-on-glaze sustain.
    - `FocusPeg` — same tick + tiny wooden click.
    - `FocusGold` — same tick + coin rim ring (very faint).
    - `FocusYakuTablet` — same tick + a single stick-on-wood block tap.
    - `FocusDora` — same tick + the `DoraScored` chime at −15 dB.

  This way every focus row has the same primary attack so navigation feels uniform, with a *spice* layer that confirms "what kind of thing I just moved onto." Replaces eight unrelated Kenney samples with one base + seven 100 ms tail layers.
- **`Pause` / `Unpause`** — today: `minimize_003.ogg` / `maximize_003.ogg`. Replace with the **same instrument, two states**: a single damped tile-tap on pause (clipping the room), and a soft inhale-style swell on unpause. Both ≤200 ms.
- **Slider drag tick** *(no SFX today)*. Add `SfxId::SliderTick`. Fire every ~5% of slider movement. Sample is a **tiny bamboo-on-bamboo tap**, mono, ≤30 ms, −24 dB. Must not be present on slider hover, only on value change.
- **Settings saved** *(no SFX today)*. Add `SfxId::SettingsSaved`. Single soft `UiConfirm` variant pitched up 2 semitones. Plays once when leaving the options screen if any value changed.

### 2.7 Ambience

Mahjuro has no ambience system today. Adding one is in scope.

- **Gameplay table room.** Very soft: distant temple bell every 60–120 s (randomized), faint wind through paper screens, occasional wood-creak of the room settling. Total ambience bed ≤ −30 dB. Implemented as a fourth music-like loop that plays *under* `MusicId::Gameplay` at low volume. Loop length ≥ 90 s with non-zero silence to avoid pattern detection.
- **Shop room.** Different bed: paper rustle of inventory being touched, the *distant* sound of a kettle, an occasional wooden floor creak. Same loudness budget. Cross-faded with the gameplay bed on scene transition.
- **Main menu exterior.** The exterior scene (`MainMenuExterior`) is outside the parlor. The existing music carries this; **no separate ambience bed**. Adding one risks clashing with the logo sting.
- **Archive / Collection.** No ambience. The room is a personal index; silence (under the menu music) is appropriate.

Tech: ambience beds are a separate `Sink` with their own gain, controlled by `master * music * 0.5` for now. Adding a third gain knob (`ambience_volume`) is preferable if a player ever complains; defer until that happens.

---

## 3. Sonic stylization / branding

### 3.1 Material palette

Mahjuro's audio palette is small and physical:

- **Porcelain** — high-glaze ceramic. Bright, short, slightly hollow attack. Source: porcelain teacup struck with wooden chopstick; record at ≥24-bit/48k.
- **Bamboo** — dry, mid-pitch, woody. Source: solid bamboo on bamboo.
- **Felt / mahjong table cloth** — soft attack, no high frequencies. Source: felt-tipped mallet on a flat wooden surface.
- **Paper** — for talismans, packs, journal pages. Two grades: thin folded (talisman), thick wrapped (pack).
- **Coin (brass/copper)** — for gold. Edge-on-edge clink with a wood-cup capture for `Sell`.
- **Struck idiophone (kane / mokugyo)** — for `RoundStart`, `BossEncountered`, `BossDefeated`. Single instrument family across all three.
- **Tuned percussion (clave, finger cymbal, kalimba tine)** — for the cascade. *No pitched melodic instruments* in the cascade except the chromatic `ScoreTick` climb.

### 3.2 Tonality

The cascade tick climb is the canonical key reference. All cascade samples (`DoraScored`, `ScoreCrescendo`, `ScoreFinal`, every `Yaku*` stinger) must be tonally compatible with the eight-semitone `SCORE_TICK_PITCHES` sequence and with the BGM tracks in their respective scenes. In practice this means **A minor pentatonic** for the cascade root, since:

- `MusicId::Gameplay` sits comfortably in that key (verify on the master before final mix).
- The chromatic climb visits the b3 and #4 and resolves cleanly to the 5.
- The yaku stingers already produced for `Tanyao..Chinitsu` cluster around that center.

The Shop and Pick-Blind tracks may modulate; cascade stingers do not play in those scenes so the conflict does not arise.

### 3.3 Processing chain (recommended for new samples)

1. **HPF at 60 Hz** for everything except `RoundStart`, `BossEncountered`, `BossDefeated`. The temple-block / gong family is the only audio entitled to sub-100 Hz energy.
2. **De-clicker** on every recorded attack.
3. **Soft tape saturation** (or analog warmer) at 5–8% on percussive cues to round the attack — bare WAV transients sound digital next to the music tracks.
4. **Convolution reverb** with a small room IR (≤0.4 s tail, ≤−18 dB wet) on all hand/table cues. They live *in the parlor*; bone-dry samples feel pasted in.
5. **Normalize to −1 dBTP peak.** Loudness budget by class (short-term LUFS):
   - Hand/table cues: −18 to −22.
   - UI chrome: −20 to −24.
   - Cascade beats: −15 to −18 (these *are* the music in their moment).
   - Stingers (`RoundStart`, `BossEncountered`, `BossDefeated`, `Victory`): −12 to −15.
6. **Export**: mono OGG Vorbis, 48 kHz, q3 (~96 kbps). The bake step re-encodes anyway (`-q:a 5`); shipping the source at q3 is fine because the perceptual quality cap is set by the re-encode.

### 3.4 Out-of-palette (do not use)

- Modern UI synth woosh kits (the kind that came with the Kenney pack we are replacing).
- Riser sweeps / DAW pre-built impacts.
- Cinematic boomwhacker hits.
- Voice (we have no VO budget and the game does not call for it; even non-lexical vocalizations conflict with the contemplative-table identity).
- Distorted guitar, EDM stabs, retro 8-bit beeps.

---

## 4. Technical info

### 4.1 File formats

| Class | Container | Codec | Sample rate | Channels | Bit depth / quality |
|---|---|---|---|---|---|
| SFX | `.ogg` | Vorbis | 48 kHz | mono | source q3+, re-encoded `-q:a 5` |
| Music | `.mp3` | LAME | 44.1 or 48 kHz | stereo | source CBR 192+ or VBR V2, re-encoded `-q:a 5` |
| Music stingers (`*_win`/`*_loss`) | `.mp3` | LAME | match the track | stereo | match the track |

Loader is `decode_rodio` in [src/audio.rs](../src/audio.rs); anything outside these formats fails to decode and the cue silently no-ops. **Stereo SFX** are accepted by the decoder but discouraged — they bypass the only mix knob (`Source::amplify`) for pan control and waste cache vs. mono.

### 4.2 Naming conventions

- `assets/audio/<role>.ogg` for flat one-off cues (`Snap.ogg`, `cash_in.ogg`, `levelup.ogg`).
- `assets/audio/yaku_<kind>.ogg` for yaku stingers; `<kind>` is the `YakuKind` variant in `snake_case` (`yaku_kokushi_musou.ogg` is the missing one).
- `assets/audio/relics/<slug>.ogg` for per-relic stingers; `<slug>` matches `RelicId::asset_filename()` minus `.png`.
- `assets/audio/music/<track>.ogg` for music loops, intros, and stingers.
- `assets/audio/<role>_<variant>.ogg` for future randomized variants (e.g. `Snap_a.ogg`, `Snap_b.ogg`). Variant selection is **not yet implemented**; adding it is the loader change described in §2.2.

`PascalCase` vs `snake_case` is inconsistent in the current tree (`Snap.ogg`, `MixingBell.ogg` are imports). New files use `snake_case`. The loader literally references whatever string `SfxId::filename()` returns, so renaming an existing file also requires editing `src/audio.rs`.

### 4.3 Memory and performance constraints

- **Pre-decoded to PCM at boot.** Every SFX listed in `all_sfx_ids()` decodes once to interleaved i16 and is reference-counted via `Arc<PcmClip>` (`src/audio.rs`). A 200 ms mono 48 kHz cue is ~19 kB on the heap. The total SFX budget today is on the order of 100 files × ~200 ms × ~19 kB ≈ **2 MB resident**. Doubling that is fine. Single SFX files longer than 1.5 s should be reviewed before adding — they bloat the boot decode and the resident set.
- **Music is lazy.** First play decodes; subsequent loops re-use the in-memory `Arc<PcmClip>`. A 90-second stereo track is ~30 MB resident. Adding ambience beds at the same length adds ~30 MB *per bed*; we have 2 beds proposed (gameplay, shop) → ~60 MB. Acceptable.
- **Voice cap.** `MAX_CONCURRENT_SFX = 8` with FIFO eviction (`src/audio.rs`). The cascade reveal is the only place this gets exercised — yaku stingers + dora chimes + per-relic stingers can stack. Eviction is currently silent (oldest sink `stop()`-ed). New designs must assume **any one cue can be cut off after 8 simultaneous voices** — keep tails short.
- **No limiter.** rodio sums sinks linearly. Sustained simultaneous loud cues will clip into the surface. The loudness budget in §3.3 is what keeps us out of trouble; a real master-bus limiter is in §7 open questions.
- **No DSP.** No filters, no reverb send, no EQ at runtime. All processing is **baked into the sample**. This is the single biggest cost-of-change in the audio system; see §7.

### 4.4 Asset pipeline

`build.rs` runs `tools/bake_assets/bake_assets.py`. SFX live in `mahjuro-pack-shared.zip` (eager mount at boot), music lives in `mahjuro-pack-music.zip` (lazy). Both `.ogg` and `.mp3` are re-encoded with ffmpeg `-q:a 5` when the lossy profile is on (release builds). Skip with `MAHJURO_SKIP_ASSET_BAKE=1`; override the runtime mount with `MAHJURO_ASSETS=<dir>` for loose-file iteration.

Re-encoding a sample to itself is **idempotent only up to the encoder** — committing a pristine high-quality source and letting bake produce the ship build is preferred over hand-tuning a low-bitrate OGG and committing that.

### 4.5 Audio settings (player-facing)

- **Master volume** — slider, 0.00–1.00, applies to SFX *and* music *and* (planned) ambience.
- **Music volume** — slider, 0.00–1.00.
- **SFX volume** — slider, 0.00–1.00.
- **Sound Effects ON/OFF** — toggle; mutes SFX only.

Source: `Section::Audio` in [src/scenes/options.rs](../src/scenes/options.rs). Settings persist via `crate::persistence::{load_settings, save_settings}`. No mute-music toggle, no per-category mixer beyond Master / Music / SFX.

---

## 5. Interactive systems

This section describes how audio reacts to gameplay. Some items are implemented; others are designed-but-not-yet-built.

### 5.1 Event-driven SFX (implemented)

Scenes push gameplay-meaningful events onto `EventBus`. The main loop drains the bus in `App::frame_tick` and converts each event into a play call ([src/main/frame_tick.rs](../src/main/frame_tick.rs)). The full list of events and their mappings lives in source; **do not duplicate it here**. When adding a new cue:

1. Add a variant to `SfxId` and its filename in `SfxId::filename()`.
2. Add it to `all_sfx_ids()` so the SFX test overlay in `src/debug_overlays.rs` picks it up automatically.
3. Drop the OGG at the path the loader expects.
4. Either call `play_sfx(id)` directly, or add a new `GameEvent` variant + bus handler in `frame_tick.rs`.

### 5.2 Staggered cascades (implemented)

Two cues use scheduled playback to avoid stacking:

- **Yaku scoring** — `GameEvent::YakuScored` queues each `SfxId::Yaku*` with a 200 ms inter-stinger delay (`frame_tick.rs`).
- **Dora chiming** — per-dora chimes are scheduled at 180 ms intervals (`src/scenes/gameplay/cascade_controller.rs`).

This is the only "music-aware" scheduling in the engine. Keep these intervals when adding new cascade-time cues; if a new cue needs to interleave, schedule it on the same offset grid.

### 5.3 Music transitions (implemented; minimal)

Music switching is hard. `start_music_track` stops the current sink and immediately starts the new one ([src/audio.rs](../src/audio.rs)). Stingers (`MusicId::BlindWin`, `BossWin`, etc.) take ownership of the music sink, play once, and defer the next looping track until the stinger empties (`tick()`). This produces the existing "blind cleared → shop" hand-off.

**Designed but not built — crossfade.** Replace the hard stop with a 250–400 ms equal-power crossfade between two sinks. Mandatory for ambience beds (§5.5) where a hard cut between gameplay and shop ambience is jarring.

### 5.4 Music ducking (designed; not built)

For `CashIn` and the cascade peak (`ScoreFinal`), the music bus should duck 3–4 dB for 250 ms and recover over 400 ms. Implementation: `AudioManager::duck_music(amount_db, hold_ms, recover_ms)`, applied to the music `Sink::set_volume`. Without this, the cascade's musical content fights the BGM.

`InvalidAction` should *not* duck — it is a failure cue and the volume mismatch helps it land.

### 5.5 Ambience beds (designed; not built)

Two looping beds (`gameplay_room`, `shop_room`) tied to scene tag, fading in/out with the same crossfade as music. Implementation: a new `ambience_sink` parallel to `music_sink`, fed by the same `LoopingPcmSource` (`src/audio.rs`). Loudness gated by `master * music * 0.5` until a separate `ambience_volume` slider becomes warranted.

Triggered ambient one-shots inside the bed (the random temple bell every 60–120 s, occasional wood-creak) should fire from a small ambient-trigger system in `AudioManager::tick`, not from gameplay code — the bed is its own object.

### 5.6 Per-relic activation routing (implemented)

`play_relic_trigger(rid)` resolves an `Arc<PcmClip>` from `relic_clips: HashMap<RelicId, …>` (populated at boot from `audio/relics/<slug>.ogg`); falls back to `SfxId::ScoreStep` on miss (`src/audio.rs`). Drop-in workflow: add an OGG at the expected path and it just plays.

When designing a per-relic stinger, treat it as a **single beat of the cascade** — same loudness budget as §3.3 cascade beats, same tonal compatibility constraint as §3.2. The relic activation often plays *inside* the cascade.

### 5.7 Per-yaku scoring (implemented)

`SfxId::for_yaku(yaku_kind)` maps a yaku to its stinger; mapping is exhaustive except for `KokushiMusou` ([src/audio.rs](../src/audio.rs)). New yaku in the future: add the `Yaku*` `SfxId`, add to `for_yaku`, drop the OGG.

### 5.8 SFX randomization (designed; not built)

Today only `Victory` / `Victory2` are randomized, and that randomization happens in `GameOverScene`, not in the audio engine.

Recommended: extend `play_sfx(id)` to look for `assets/audio/<basename>_a.ogg`, `_b.ogg`, … and pick at random with a **no-immediate-repeat** rule (track last variant played per `SfxId`). First files to randomize: `Snap.ogg`, `TileDiscard`, `TileSelect`, `UiConfirm`. These are the highest-repetition cues in a session.

### 5.9 Pitch / speed jitter (designed; not built)

For the highest-repetition cues, in addition to variant randomization: jitter playback `Source::speed` by ±2% (≈ ±34 cents). This is below the threshold of "pitched" but breaks identical-attack fatigue. Implement as a small wrapper around `Source::amplify` (which is the existing wrap point in `play_clip`).

### 5.10 Audio occlusion / 3D (not in scope)

Mahjuro is a fixed-camera 2.5D presentation. There is no 3D positional audio. Stereo panning of UI cues based on screen position is *also* out of scope — the contemplative-table identity is mono-centered.

### 5.11 Boss-blind audio variation (designed; not built — one knob)

A common request would be "each boss has its own theme." This is out of scope (23 bosses × music = 23 tracks). Instead, the boss blind applies a **single global low-pass filter** at ~3 kHz to `MusicId::Gameplay` while the boss is on the table, and removes it on defeat. The filter sells "the room got smaller" without authoring new music. Implementation requires runtime DSP on the music sink — see §7.

---

## 6. Implementation notes

### 6.1 Trigger conditions

Triggers are 1:1 with the `GameEvent` variants in [src/game/event_bus.rs](../src/game/event_bus.rs). When `frame_tick.rs` maps a new event → cue, also:

- Decide whether the cue should be **discardable** at the voice cap. If a cue *must* play (e.g. `ScoreFinal`, `Victory`), it should be played via a future `play_sfx_priority(id)` path that evicts the **oldest non-priority** sink first instead of the absolute oldest. Not built yet; today everything is treated equal.
- Decide whether the cue should **defer if a stinger is holding the music sink**. Cues currently fire regardless; `MainMenuEnter` already plays *over* the music jingle on entry. Most cues are fine with this; victory/defeat stingers should remain mutually exclusive (current code already enforces this implicitly because the screen states do).

### 6.2 Variant selection

When SFX randomization (§5.8) lands, variant selection is a property of the loader, not the call site. `play_sfx(SfxId::TilePlace)` continues to be the only API; the loader picks `Snap_a/b/c.ogg` from disk.

### 6.3 Mixing rules

- **Cascade priority.** During an active score cascade, UI focus ticks (`Focus*`, `TileSelect`, `TileDeselect`) should not play. The simplest implementation: `AudioManager` exposes `is_cascade_active: bool` set by the gameplay scene; `play_sfx` for focus cues no-ops while true.
- **No double-fire on settle.** Hand-tile focus on tile drawn → tile settles → focus changes — currently this can produce a `TilePlace` and a `FocusHandTile` within ~50 ms. The focus cue should suppress for 80 ms after a `TilePlace`. Implement as a per-`SfxId` last-played timestamp check in `play_sfx`.
- **Modal dismiss is silent if the dismiss happened by clicking the modal's primary button.** The button already plays `UiConfirm`; an extra modal-close cue is double-feedback.

### 6.4 Loop-safety

Looping music must have **bit-exact loop points**. ffmpeg re-encode can introduce a few samples of padding; verify in Audacity that the looped file plays seamlessly back-to-back before committing. The `LoopingPcmSource` ([src/audio.rs](../src/audio.rs)) returns to PCM offset 0 with no overlap, so any padding will be audible as a click.

### 6.5 First-frame correctness

The audio device is the slow path of `AudioManager::new`. If a scene plays a cue in its `on_enter` before the device finishes initializing, the cue is dropped. This is already handled — `AudioManager` gracefully no-ops when the device is unavailable — but means the splash → main menu hand-off must not depend on `MainMenuEnter` having played for state correctness.

---

## 7. Pipeline, workflow, and open questions

### 7.1 Roles

- **Sound designer (external or contracted)** — produces final OGG/MP3 sources per the loudness and tonality constraints in §3, names them per §4.2, hands them to the dev team in batches by class (e.g. "all UI focus cues," "all boss cues").
- **Developer (this repo)** — adds `SfxId` variants and `GameEvent` plumbing, drops files into `assets/audio/`, verifies in the SFX test overlay (`src/debug_overlays.rs`), confirms voice-cap behavior at peak cascade.
- **Reviewer (designer + dev)** — verifies tonal compatibility with BGM and the cascade key (§3.2), verifies loudness budget (§3.3), verifies no fatigue at 200× repetition for high-frequency cues.

### 7.2 Delivery milestones

| Milestone | Scope |
|---|---|
| **M1 — Replace UI chrome** | `UiConfirm`, `UiCancel`, the eight `Focus*` cues, `Pause`, `Unpause`. Removes every Kenney placeholder in the menu loop. |
| **M2 — Replace tile chrome** | `TileSelect`, `TileDeselect`, `StructureCommit`, `InvalidAction`. Locks the hand-row identity. |
| **M3 — Cascade polish** | `CascadeMerge`, `CascadeLaunch`, `DoraScored`, `ScoreCrescendo`, `ScoreFinal`. Plus `YakuKokushiMusou`. Tightens the most-heard musical sequence. |
| **M4 — Missing-asset cues** | `BossEncountered`, `BossDefeated`, `TilesDestroyed`, `TalismanPurchased`, `TalismanUsed`. Brings parity with already-wired events. |
| **M5 — Economy polish** | `Sell`, `PackBuy`, `PackOpen`, `PackTileReveal`, `RoundStart`. |
| **M6 — Ambience beds** | `gameplay_room`, `shop_room` loops. Requires the §5.3 crossfade in code first. |
| **M7 — Engine features** | SFX variant randomization (§5.8), ±2% pitch jitter (§5.9), music ducking (§5.4), low-pass on boss blinds (§5.11). |

M1–M5 are pure content. M6 requires one new sink. M7 is the largest dev work and is gated by audible need.

### 7.3 QA

- The **SFX Test debug overlay** ([src/debug_overlays.rs](../src/debug_overlays.rs), iterating `all_sfx_ids()`) is the auditioning surface. Every new `SfxId` must appear here.
- **Voice-cap stress test:** in a debug build, force a cascade with full yaku coverage and four active relics; confirm no critical cue (`ScoreFinal`, `Victory`) is evicted.
- **Loudness regression:** spot-check new cues against `Snap.ogg` and `cash_in.ogg` on the same playback path; the new cue should not exceed the comparable budget by more than ~2 dB.

### 7.4 Open questions

- **Variant randomization API.** Implement at loader level (filename glob) or at `SfxId` level (each variant is its own enum)? Glob is simpler; enum-level is more typed. Lean glob.
- **Runtime DSP — yes or no.** Music ducking (§5.4), low-pass on boss blinds (§5.11), and modulation-style ambience effects all want runtime DSP on the music sink. rodio has limited filter support; a small custom DSP layer (one-pole filters, simple gain ramps) might be cheaper than swapping engines. Decide once at least one feature actually warrants it; do not pre-build.
- **Ambience as music vs ambience as SFX.** Today the system has two classes (music = stereo MP3, SFX = mono OGG). Ambience beds are stereo loops but conceptually closer to SFX. Treating them as a third class with their own pack (`mahjuro-pack-ambience.zip`, lazy) keeps boot fast. Decide before M6.
- **Master limiter.** Without one, simultaneous victory + relic-stinger + level-up at the run-complete screen can clip the surface. A simple soft-knee brick-wall at −0.5 dBFS on the master sum would be cheap insurance. Defer until a real clip is reported.
- **Localization / culturally-specific instruments.** The struck-idiophone family (mokugyo, kane) is Japanese; some mahjong audiences expect Chinese instruments (e.g. paigu, wooden fish). The visual design currently leans Japanese parlor — match the audio to the visual, do not split. Revisit if the visual rebrands.
- **Dynamic music for the cascade.** A version of the §2.3 cascade where the BGM *itself* gains layers as the cascade builds (Devil May Cry / Doom Eternal pattern) is tempting. It is out of scope until the cascade has a finalized identity at single-layer mix; we should not optimize the cascade-music interaction while still placing fundamental cascade cues.

---

## Appendix: Cue inventory (categorized)

This is the design-side view of the cue inventory. The implementation-side source of truth is `enum SfxId` + `SfxId::filename()` in [src/audio.rs](../src/audio.rs); when the two disagree, source wins and this doc should be updated.

**Locked:** `MainMenu` BGM, `Gameplay` BGM, `Shop` BGM, `BlindWin`/`BlindLoss`/`BossWin`/`BossLoss` music stingers, `TileDiscard`, `TileClick`, all `Yaku*` (except `YakuKokushiMusou`), all `audio/relics/<slug>.ogg` that ship today, `MainMenuEnter`, `RoundWin`, `Victory`/`Victory2`, `GameOver`, `Defeat`, `LevelUp`, `ZodiacLevelUp`, `CashIn`, `CoinDrop`, `Purchase`, `CandleFlareWhoosh`, `CandleFlareImpact`.

**Placeholder — high priority replace (M1–M2):** `UiConfirm`, `UiCancel`, `FocusHandTile`, `FocusButton`, `FocusConsumable`, `FocusRelic`, `FocusPeg`, `FocusGold`, `FocusYakuTablet`, `FocusDora`, `TileSelect`, `TileDeselect`, `StructureCommit`, `InvalidAction`, `Pause`, `Unpause`.

**Placeholder — medium priority (M3, M5):** `Sell`, `PackBuy`, `PackOpen`, `PackTileReveal`, `RoundStart`, `ZodiacReveal`, `StarShimmer`, `DoraScored`, `ScoreCrescendo`, `ScoreFinal`, `CascadeMerge`, `CascadeLaunch`, `CascadeLand`, `ScoreReveal`, `ScoreStep`, `ScoreTick` (base sample).

**Missing — wire is live, asset is absent (M3–M4):** `YakuKokushiMusou`, `TilesDestroyed`, `BossEncountered`, `BossDefeated`, `TalismanPurchased`, `TalismanUsed`.

**New — does not exist in `SfxId` yet (M1, M7):** `SliderTick`, `SettingsSaved`.

**Ambience — does not exist as a class yet (M6):** `Ambience::GameplayRoom`, `Ambience::ShopRoom`.
