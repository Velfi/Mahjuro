use super::locals::FrameLocals;
use crate::audio;
use crate::game::event_bus::GameEvent;
use crate::steam::DistributionBackend;
use crate::ui::input::InputState;
use crate::ui::modal::{Modal, ModalTheme};
use crate::App;
use crate::sdl_shell::SdlShell;

pub fn run(app: &mut App, shell: &mut SdlShell, locals: &mut FrameLocals) {
        if let Some(input) = app.input.as_mut() {
            input.tick_scoring_rumble_keepalive(shell, locals.now);
        }
        // Drain into a Vec so the bus borrow ends before the loop body, which
        // calls back into `&mut app` (rumble helpers, scene transitions, …).
        let drained: Vec<GameEvent> = app.bus.drain().collect();
        for ev in drained {
            match ev {
                GameEvent::TileDrawn => {
                    app.audio.play_sfx(audio::SfxId::TilePlace);
                }
                GameEvent::TileDiscarded => {
                    app.audio.play_sfx(audio::SfxId::TileDiscard);
                }
                GameEvent::ScoreUpdated => {
                    app.audio.play_sfx(audio::SfxId::ScoreReveal);
                }
                GameEvent::ScoreStepRevealed { index } => {
                    // Climb eight semitones across the reveal so the
                    // cascade audibly rises, then wrap. Layer the
                    // existing ScoreStep "rollover" sound on top to
                    // keep the soft confirmation that's already
                    // wired into the game.
                    app.audio.play_score_tick(index);
                    app.audio.play_sfx(audio::SfxId::ScoreStep);
                    if app.controller_rumble_active() {
                        let (weak, strong, duration_ms, gain) =
                            InputState::cascade_step_rumble_params();
                        app.fire_rumble_pulse(shell, locals.now, weak, strong, duration_ms, gain);
                    }
                }
                GameEvent::ScoreCascadeFinal { earned } => {
                    // Crescendo: brassy hit jingle layered over the
                    // existing confirmation sting so the closing
                    // beat lands with weight.
                    app.audio.play_sfx(audio::SfxId::ScoreFinal);
                    app.audio.play_sfx(audio::SfxId::ScoreCrescendo);
                    app.dist
                        .unlock_achievement(crate::steam::Achievement::FirstStructure);
                    if app.controller_rumble_active() {
                        let (weak, strong, duration_ms, gain) =
                            InputState::cascade_final_rumble_params(earned);
                        app.fire_rumble_pulse(shell, locals.now, weak, strong, duration_ms, gain);
                    }
                }
                GameEvent::GoldChanged { .. } => {
                    app.audio.play_sfx(audio::SfxId::CoinDrop);
                }
                ev @ GameEvent::RoundComplete { .. } => {
                    // Hold the win sting + scene transition until the
                    // scoring cascade has finished playing out — the
                    // player should get to watch the winning hand pop.
                    app.deferred_round_end = Some(ev);
                }
                ev @ GameEvent::GameOver { .. } => {
                    // Same as RoundComplete: hold until the final
                    // cascade has finished animating.
                    app.deferred_round_end = Some(ev);
                }
                GameEvent::PackBought => {
                    app.audio.play_sfx(audio::SfxId::PackBuy);
                }
                GameEvent::PackOpened => {
                    app.audio.play_sfx(audio::SfxId::PackOpen);
                }
                GameEvent::PackTileRevealed => {
                    app.audio.play_sfx(audio::SfxId::PackTileReveal);
                }
                GameEvent::ZodiacReveal => {
                    app.audio.play_sfx(audio::SfxId::ZodiacReveal);
                }
                GameEvent::ZodiacLevelUp => {
                    app.audio.play_sfx(audio::SfxId::ZodiacLevelUp);
                }
                GameEvent::CandleFlare => {
                    app.audio.play_sfx(audio::SfxId::CandleFlareWhoosh);
                    app.audio.play_sfx(audio::SfxId::CandleFlareImpact);
                }
                GameEvent::StructureCommitted => {
                    app.audio.play_sfx(audio::SfxId::StructureCommit);
                }
                GameEvent::TilesDestroyed => {
                    app.audio.play_sfx(audio::SfxId::TilesDestroyed);
                }
                GameEvent::InvalidAction => {
                    app.audio.play_sfx(audio::SfxId::InvalidAction);
                }
                GameEvent::UiSound(id) => {
                    app.audio.play_sfx(id);
                }
                GameEvent::HoldWindupStart => {
                    app.audio.play_hold_windup();
                }
                GameEvent::HoldWindupStop => {
                    app.audio.stop_hold_windup();
                }
                GameEvent::PlayRelicStinger(rid) => {
                    app.audio.play_relic_trigger(rid);
                }
                GameEvent::RelicActivated(rid) => {
                    // Visual feedback (glow + wiggle) is handled by the
                    // active scene; audio is the per-relic stinger,
                    // falling back to a soft chime when the relic has
                    // no bespoke audio file.
                    app.audio.play_relic_trigger(rid);
                    *app.progress.relic_times_activated.entry(rid).or_insert(0) += 1;
                    app.mark_profile_dirty();
                }
                GameEvent::OrdealEncountered(bk) => {
                    app.audio.play_sfx(audio::SfxId::OrdealEncountered);
                    *app
                        .progress
                        .ordeal_times_encountered
                        .entry(bk)
                        .or_insert(0) += 1;
                    app.mark_profile_dirty();
                    // Full Roster — non-final ordeals only. Final-tier
                    // Dragon/House are excluded; beating either is covered
                    // by `FirstRunCompleted` / `HouseDefeated`.
                    let pool = crate::core::ordeal::regular_pool();
                    if pool
                        .iter()
                        .all(|kind| app.progress.ordeal_times_encountered.contains_key(kind))
                    {
                        app.dist
                            .unlock_achievement(crate::steam::Achievement::AllBossesSeen);
                    }
                }
                GameEvent::OrdealDefeated(bk) => {
                    if !app.run.onboarding_active() {
                        app.audio.play_sfx(audio::SfxId::OrdealDefeated);
                        *app.progress.ordeal_times_defeated.entry(bk).or_insert(0) += 1;
                        app.mark_profile_dirty();
                        app.dist
                            .unlock_achievement(crate::steam::Achievement::FirstOrdealDefeated);
                        if bk == crate::core::ordeal::OrdealKind::House {
                            app.dist
                                .unlock_achievement(crate::steam::Achievement::HouseDefeated);
                        }
                    }
                }
                GameEvent::TalismanPurchased(tk) => {
                    app.audio.play_sfx(audio::SfxId::TalismanPurchased);
                    *app
                        .progress
                        .talisman_times_purchased
                        .entry(tk)
                        .or_insert(0) += 1;
                    app.mark_profile_dirty();
                }
                GameEvent::TalismanUsed(tk) => {
                    app.audio.play_sfx(audio::SfxId::TalismanUsed);
                    *app.progress.talisman_times_used.entry(tk).or_insert(0) += 1;
                    app.mark_profile_dirty();
                }
                GameEvent::MemorialTalismanUsed(_) => {
                    app.audio.play_sfx(audio::SfxId::TalismanUsed);
                }
                GameEvent::YakuScored(yk) => {
                    *app.progress.yaku_times_scored.entry(yk).or_insert(0) += 1;
                    app.mark_profile_dirty();
                }
                GameEvent::AchievementUnlocked(ach) => {
                    app.dist.unlock_achievement(ach);
                }
                GameEvent::TransformationSuccessorDiscovered(rid) => {
                    let _ = app.progress.note_transformation_successor_discovered(rid);
                    app.mark_profile_dirty();
                }
                GameEvent::ArchiveItemSeen(mark) => {
                    if app.progress.mark_archive_seen(mark) {
                        app.mark_profile_dirty();
                    }
                }
                GameEvent::ArchiveSeedSeenIfNeeded => {
                    crate::core::archive_seen::archive_seen_migration_seed(&mut app.progress);
                    app.mark_profile_dirty();
                }
                GameEvent::InfoModal { title, body } => {
                    app.modals.push(Modal::new(title, body, ModalTheme::Info));
                }
                GameEvent::OpenControllerMappingHelp => {
                    app.modals.push(Modal::new(
                        "Controller mapping".to_string(),
                        "Mahjuro reads your gamepad through SDL3 using the standard PC \
                         layout (south = confirm by default). Use Options to swap \
                         South/East or West/North if your printed labels differ. For \
                         OS-wide or per-game remaps, use Windows / macOS / Linux \
                         settings, Steam's controller configuration, or your device's \
                         companion app."
                            .to_string(),
                        ModalTheme::Info,
                    ));
                }
                GameEvent::RoomGltfBrownout => {
                    app.room_gltf_brownout.trigger();
                    app.audio.play_sfx(audio::SfxId::BrownoutFlicker);
                }
            }
        }
}
