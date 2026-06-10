use super::locals::FrameLocals;
use crate::audio;
use crate::scenes::SceneBehavior;
use crate::App;
use crate::sdl_shell::SdlShell;
pub fn run(app: &mut App, shell: &mut SdlShell, locals: &mut FrameLocals) {
    let now = std::time::Instant::now();
    locals.now = now;
    app.last_frame_dt = now
        .saturating_duration_since(app.last_frame)
        .as_secs_f32()
        .max(0.0001);
    app.last_frame = now;
    // Pause the watchdog during scene fades (`transition_alpha < 1.0` /
    // `pending_scene` set): those frames legitimately stall on shader /
    // texture loads and would otherwise false-fire on first launch.
    let transitioning = app.scene_replace_in_flight() || app.transition_alpha < 1.0;
    app.perf_watchdog
        .tick(app.last_frame_dt * 1000.0, transitioning, now);
    app.anim.update(now);
    app.audio.tick(now);
    if app
        .debug
        .trailer_mode
        .as_ref()
        .is_some_and(|tm| tm.finished_at(now))
    {
        app.debug.trailer_mode = None;
        log::debug!("Trailer mode finished");
    }
    app.try_play_production_logo_stinger();

    let drawn = app.overlay_stack.last().unwrap_or(&app.scene);
    let brownout_room = crate::main_room_gltf_brownout::RoomGltfBrownout::scene_eligible(drawn);
    let brownout_freeze = app.debug.scene_look_debug_overlay.is_some()
        || app.debug.rain_debug_overlay.is_some()
        || app.scene.has_blocking_overlay()
        || app
            .overlay_stack
            .iter()
            .any(|s| s.has_blocking_overlay());
    let room_ambience = app
        .room_gltf_brownout
        .tick(app.last_frame_dt, brownout_room, brownout_freeze);
    if room_ambience.brownout_started {
        app.audio.play_sfx(audio::SfxId::BrownoutFlicker);
    }
    if room_ambience.play_creak {
        app.audio.play_sfx(audio::SfxId::RoomCreak);
    }

    // Refresh opened gamepads before any rumble this frame. `tick_scoring_rumble_keepalive`
    // and bus handlers run before `gamepad_frame_tick`; without this, `shell.pads` can
    // still be empty on the first frames after connect or if ordering ever regresses.
    shell.prepare_gamepad_frame();
}
