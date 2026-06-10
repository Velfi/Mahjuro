use super::locals::FrameLocals;
use crate::audio;
use crate::App;
use crate::sdl_shell::SdlShell;

pub fn run(app: &mut App, shell: &mut SdlShell, locals: &mut FrameLocals) {
    if locals.quit_requested {
        app.quit_requested = true;
    }

    app.draw(shell);
    app.flush_dirty_profile();

    let cpu_done = app.cpu_profiler.take_just_completed();
    let gpu_done = app
        .renderer
        .as_mut()
        .is_some_and(|r| r.take_gpu_profile_just_completed());
    if cpu_done || gpu_done {
        app.audio.play_sfx(audio::SfxId::UiConfirm);
    }
}

