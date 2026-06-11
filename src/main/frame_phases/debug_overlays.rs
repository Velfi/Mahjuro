use super::locals::FrameLocals;
use crate::App;
use crate::debug_overlays::{DebugVisResult, TuningResult};

pub fn run(app: &mut App, locals: &mut FrameLocals) {
    if let Some(ref mut overlay) = app.debug.tuning_overlay {
        let (ww, wh) = (
            app.last_drawable_px.width as f32,
            app.last_drawable_px.height as f32,
        );
        let mouse = app.input.as_ref().map(|i| {
            (
                i.last_cursor.0,
                i.last_cursor.1,
                app.mouse_clicked,
                app.mouse_left_down,
            )
        });
        match overlay.update(&locals.actions, mouse, ww, wh) {
            TuningResult::Stay => {
                // Apply live tuning changes.
                app.cascade_tuning = overlay.tuning.clone();
            }
            TuningResult::Close => {
                // Apply final tuning and close.
                app.cascade_tuning = overlay.tuning.clone();
                app.debug.tuning_overlay = None;
                log::debug!("Closed cascade tuning overlay");
            }
            TuningResult::Export => {
                let json = serde_json::to_string_pretty(&overlay.tuning)
                    .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"));
                let path = "cascade_tuning.json";
                match std::fs::write(path, &json) {
                    Ok(()) => log::debug!("Exported tuning to {path}"),
                    Err(e) => log::error!("Failed to export tuning: {e}"),
                }
            }
        }
        app.mouse_clicked = false;
        locals.clear_input();
    }

    if let Some(mut overlay) = app.debug.sfx_test_overlay.take() {
        let mouse = app.input.as_ref().map(|i| {
            let (mx, my) = i.last_cursor;
            (mx, my, app.mouse_clicked, app.mouse_left_down)
        });
        let close = overlay.update(&locals.actions, &mut app.audio, mouse);
        app.mouse_clicked = false;
        if !close {
            app.debug.sfx_test_overlay = Some(overlay);
        } else {
            log::debug!("Closed SFX test overlay");
        }
        locals.clear_input();
    }

    if let Some(mut overlay) = app.debug.camera_debug_overlay.take() {
        let (ww, wh) = (
            app.last_drawable_px.width as f32,
            app.last_drawable_px.height as f32,
        );
        let mouse = app.input.as_ref().map(|i| {
            (
                i.last_cursor.0,
                i.last_cursor.1,
                app.mouse_clicked,
                app.mouse_left_down,
            )
        });
        let close = overlay.update(&locals.actions, mouse, ww, wh);
        if !close {
            app.debug.camera_debug_overlay = Some(overlay);
        } else {
            log::debug!("Closed camera debug overlay");
        }
        locals.clear_input();
    }

    // Pick-blind hallway hall FX overlay (sliders; drawn above shop env panel).
    if let Some(mut overlay) = app.debug.hallway_distortion_debug_overlay.take() {
        let (ww, wh) = (
            app.last_drawable_px.width as f32,
            app.last_drawable_px.height as f32,
        );
        let mouse = app.input.as_ref().map(|i| {
            (
                i.last_cursor.0,
                i.last_cursor.1,
                app.mouse_clicked,
                app.mouse_left_down,
            )
        });
        let close = overlay.update(&locals.actions, mouse, ww, wh);
        app.mouse_clicked = false;
        if !close {
            app.debug.hallway_distortion_debug_overlay = Some(overlay);
        } else {
            log::debug!("Closed hallway vertex warp debug overlay");
        }
        locals.clear_input();
    }

    // Scene look overlay (tonemap + room GLB sliders, per-scene save).
    if let Some(mut overlay) = app.debug.scene_look_debug_overlay.take() {
        use crate::debug_overlays::SceneLookDebugResult;
        use crate::game::scene_look_tuning::{SceneLookTuning, clear_scene_look, save_scene_look};
        let (ww, wh) = (
            app.last_drawable_px.width as f32,
            app.last_drawable_px.height as f32,
        );
        let mouse = app.input.as_ref().map(|i| {
            (
                i.last_cursor.0,
                i.last_cursor.1,
                app.mouse_clicked,
                app.mouse_left_down,
            )
        });
        let scene_key_lookup = overlay.scene_key().map(str::to_string);
        let persist_key = overlay.scene_key_persist().to_string();
        let mut close = false;
        match overlay.update(&locals.actions, mouse, ww, wh, &app.scene_look) {
            SceneLookDebugResult::Stay => {
                app.scene_look
                    .set(scene_key_lookup.as_deref(), overlay.look);
            }
            SceneLookDebugResult::Reset => {
                overlay.look = SceneLookTuning::default();
                app.scene_look.clear(scene_key_lookup.as_deref());
                match clear_scene_look(&persist_key) {
                    Ok(()) => {
                        log::debug!("Cleared SceneLookTuning override for scene '{persist_key}'")
                    }
                    Err(e) => log::warn!(
                        "Failed to clear SceneLookTuning override for '{persist_key}': {e}"
                    ),
                }
            }
            SceneLookDebugResult::Save => {
                app.scene_look
                    .set(scene_key_lookup.as_deref(), overlay.look);
                match save_scene_look(&persist_key, &overlay.look) {
                    Ok(()) => {
                        log::debug!("Saved SceneLookTuning override for scene '{persist_key}'")
                    }
                    Err(e) => log::warn!(
                        "Failed to save SceneLookTuning override for '{persist_key}': {e}"
                    ),
                }
            }
            SceneLookDebugResult::Close => {
                app.scene_look
                    .set(scene_key_lookup.as_deref(), overlay.look);
                close = true;
            }
        }
        app.mouse_clicked = false;
        if !close {
            app.debug.scene_look_debug_overlay = Some(overlay);
        } else {
            log::debug!("Closed scene look debug overlay");
        }
        locals.clear_input();
    }

    if let Some(mut overlay) = app.debug.rain_debug_overlay.take() {
        use crate::render::main_menu_effects_debug_overlay::MainMenuEffectsDebugResult;
        use crate::render::main_menu_effects_tuning::MainMenuEffectsTuning;
        let (ww, wh) = (
            app.last_drawable_px.width as f32,
            app.last_drawable_px.height as f32,
        );
        let mouse = app.input.as_ref().map(|i| {
            (
                i.last_cursor.0,
                i.last_cursor.1,
                app.mouse_clicked,
                app.mouse_left_down,
            )
        });
        let mut close = false;
        match overlay.update(&locals.actions, mouse, ww, wh) {
            MainMenuEffectsDebugResult::Stay => {}
            MainMenuEffectsDebugResult::Reset => {
                overlay.tuning = MainMenuEffectsTuning::shipping_default();
                if let Err(e) = MainMenuEffectsTuning::clear_saved() {
                    log::warn!("Failed to clear MainMenuEffectsTuning override: {e}");
                } else {
                    log::debug!("Cleared MainMenuEffectsTuning override");
                }
            }
            MainMenuEffectsDebugResult::Save => {
                if let Err(e) = overlay.tuning.save() {
                    log::warn!("Failed to save MainMenuEffectsTuning override: {e}");
                } else {
                    log::debug!("Saved MainMenuEffectsTuning override");
                }
            }
            MainMenuEffectsDebugResult::Close => close = true,
        }
        if let Some(renderer) = app.renderer.as_mut() {
            renderer.main_menu_effects = overlay.tuning;
            renderer.main_menu_pride_rainbow_debug = overlay.pride_rainbow_debug;
            renderer.main_menu_moon_phase_debug = overlay.moon_phase_debug;
        }
        app.debug.main_menu_pride_rainbow_debug = overlay.pride_rainbow_debug;
        app.debug.main_menu_moon_phase_debug = overlay.moon_phase_debug;
        app.mouse_clicked = false;
        if !close {
            app.debug.rain_debug_overlay = Some(overlay);
        } else {
            log::debug!("Closed main menu effects debug overlay");
        }
        locals.clear_input();
    }

    if let Some(mut overlay) = app.debug.flame_debug_overlay.take() {
        use crate::render::flame_debug_overlay::FlameDebugResult;
        use crate::render::flame_tuning::FlameTuning;
        let (ww, wh) = (
            app.last_drawable_px.width as f32,
            app.last_drawable_px.height as f32,
        );
        let mouse = app.input.as_ref().map(|i| {
            (
                i.last_cursor.0,
                i.last_cursor.1,
                app.mouse_clicked,
                app.mouse_left_down,
            )
        });
        let mut close = false;
        match overlay.update(&locals.actions, mouse, ww, wh) {
            FlameDebugResult::Stay => {}
            FlameDebugResult::Reset => {
                overlay.tuning = FlameTuning::shipping_default();
                if let Some(renderer) = app.renderer.as_mut() {
                    renderer.flame_gust_runtime = Default::default();
                }
                if let Err(e) = FlameTuning::clear_saved() {
                    log::warn!("Failed to clear FlameTuning override: {e}");
                } else {
                    log::debug!("Cleared FlameTuning override");
                }
            }
            FlameDebugResult::Save => {
                if let Err(e) = overlay.tuning.save() {
                    log::warn!("Failed to save FlameTuning override: {e}");
                } else {
                    log::debug!("Saved FlameTuning override");
                }
            }
            FlameDebugResult::Close => close = true,
            FlameDebugResult::TriggerGust { room } => {
                if let Some(renderer) = app.renderer.as_mut() {
                    let dir =
                        glam::Vec2::new(overlay.tuning.wind_bias_x, overlay.tuning.wind_bias_y);
                    renderer.flame_gust_runtime.trigger(dir, room, 0.0);
                    log::debug!(
                        "Triggered {} gust (bias=({:.3}, {:.3}))",
                        if room { "room" } else { "per-candle" },
                        overlay.tuning.wind_bias_x,
                        overlay.tuning.wind_bias_y,
                    );
                }
            }
        }
        if let Some(renderer) = app.renderer.as_mut() {
            renderer.flame_tuning = overlay.tuning;
        }
        app.mouse_clicked = false;
        if !close {
            app.debug.flame_debug_overlay = Some(overlay);
        } else {
            log::debug!("Closed flame debug overlay");
        }
        locals.clear_input();
    }

    if let Some(mut overlay) = app.debug.victory_moon_debug_overlay.take() {
        use crate::render::victory_moon_debug_overlay::VictoryMoonDebugResult;
        use crate::render::victory_moon_tuning::VictoryMoonDebug;
        let (ww, wh) = (
            app.last_drawable_px.width as f32,
            app.last_drawable_px.height as f32,
        );
        let mouse = app.input.as_ref().map(|i| {
            (
                i.last_cursor.0,
                i.last_cursor.1,
                app.mouse_clicked,
                app.mouse_left_down,
            )
        });
        let mut close = false;
        match overlay.update(&locals.actions, mouse, ww, wh) {
            VictoryMoonDebugResult::Stay => {}
            VictoryMoonDebugResult::Reset => {
                overlay.debug = VictoryMoonDebug::shipping_default();
                log::debug!("Reset victory moon debug to defaults");
            }
            VictoryMoonDebugResult::Close => close = true,
        }
        app.debug.victory_moon_debug = overlay.debug;
        app.debug.main_menu_moon_phase_debug = overlay.debug.moon_phase;
        if let Some(renderer) = app.renderer.as_mut() {
            renderer.main_menu_moon_phase_debug = overlay.debug.moon_phase;
        }
        app.mouse_clicked = false;
        if !close {
            app.debug.victory_moon_debug_overlay = Some(overlay);
        } else {
            log::debug!("Closed victory moon debug overlay");
        }
        locals.clear_input();
    }

    // input. Mirror the toggle state back to App fields each
    // frame so the gameplay scene + retain filter pick up live
    // changes immediately.
    if let Some(mut overlay) = app.debug.visibility_overlay.take() {
        let (ww, wh) = (
            app.last_drawable_px.width as f32,
            app.last_drawable_px.height as f32,
        );
        let mouse = app.input.as_ref().map(|i| {
            (
                i.last_cursor.0,
                i.last_cursor.1,
                app.mouse_clicked,
                app.mouse_left_down,
            )
        });
        let result = overlay.update(&locals.actions, mouse, ww, wh);
        app.mouse_clicked = false;
        app.debug.visibility = overlay.vis;
        if result == DebugVisResult::Stay {
            app.debug.visibility_overlay = Some(overlay);
        } else {
            log::debug!("Closed debug visibility overlay");
        }
        locals.clear_input();
    }
}
