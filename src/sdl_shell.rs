//! Single SDL3 context: window, event pump, gamepads (wgpu stays separate).

use std::collections::{HashMap, HashSet};

use sdl3::gamepad::Gamepad;
use sdl3::joystick::JoystickId;
use sdl3::video::{FullscreenType, Window, WindowBuilder};
use sdl3::{EventPump, GamepadSubsystem, JoystickSubsystem, Sdl, VideoSubsystem};

pub struct SdlShell {
    pub(crate) _sdl: Sdl,
    pub(crate) _video: VideoSubsystem,
    pub window: Window,
    pub(crate) pump: EventPump,
    pub(crate) gamepad: GamepadSubsystem,
    /// Rumble is applied via the gamepad API but SDL documents that
    /// [`SDL_UpdateJoysticks`](https://wiki.libsdl.org/SDL3/SDL_RumbleGamepad) must run for effects to reach hardware.
    pub(crate) joystick: JoystickSubsystem,
    pub(crate) pads: HashMap<JoystickId, Gamepad>,
    pub(crate) lt_prev: HashMap<JoystickId, f32>,
    pub(crate) rt_prev: HashMap<JoystickId, f32>,
    /// Joystick IDs we already logged as non-gamepad (avoid spam each frame).
    non_gamepad_logged: HashSet<JoystickId>,
}

impl SdlShell {
    pub fn new(title: &str, width: u32, height: u32) -> anyhow::Result<Self> {
        let _sdl = sdl3::init().map_err(anyhow::Error::from)?;
        let _ = sdl3::hint::set("SDL_VIDEO_MACOSX_METAL_LAYER", "1");

        let _video = _sdl.video().map_err(anyhow::Error::from)?;
        let gamepad = _sdl.gamepad().map_err(anyhow::Error::from)?;
        let joystick = _sdl.joystick().map_err(anyhow::Error::from)?;

        let mut wb = WindowBuilder::new(&_video, title, width, height);
        wb.resizable().high_pixel_density().position_centered();

        #[cfg(target_os = "macos")]
        {
            wb.set_flags(
                WindowFlags::METAL | WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY,
            );
            wb.metal_view();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            wb.vulkan();
        }
        #[cfg(target_os = "windows")]
        {
            wb.set_flags(WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY);
        }

        let window = wb
            .build()
            .map_err(|e| anyhow::anyhow!("SDL_CreateWindow: {e}"))?;

        let pump = _sdl.event_pump().map_err(anyhow::Error::from)?;

        let mut shell = Self {
            _sdl,
            _video,
            window,
            pump,
            gamepad,
            joystick,
            pads: HashMap::new(),
            lt_prev: HashMap::new(),
            rt_prev: HashMap::new(),
            non_gamepad_logged: HashSet::new(),
        };
        shell.refresh_gamepads();
        Ok(shell)
    }

    pub fn show_cursor(&self, show: bool) {
        self._sdl.mouse().show_cursor(show);
    }

    pub fn prepare_gamepad_frame(&mut self) {
        self.gamepad.update();
        self.joystick.update();
        self.refresh_gamepads();
    }

    /// Call after any `Gamepad::set_rumble` so SDL pushes the effect to the driver this frame.
    #[inline]
    pub fn sync_gamepad_rumble_output(&self) {
        self.gamepad.update();
        self.joystick.update();
    }

    pub fn refresh_gamepads(&mut self) {
        let Ok(ids) = self.gamepad.gamepads() else {
            return;
        };
        self.pads.retain(|id, _| ids.contains(id));
        self.lt_prev.retain(|id, _| ids.contains(id));
        self.rt_prev.retain(|id, _| ids.contains(id));
        self.non_gamepad_logged.retain(|id| ids.contains(id));
        for &id in &ids {
            if self.pads.contains_key(&id) {
                continue;
            }
            if !self.gamepad.is_gamepad(id) {
                if !self.non_gamepad_logged.contains(&id) {
                    if let Ok(name) = self.gamepad.name_for_id(id) {
                        let lower = name.to_lowercase();
                        if lower.contains("nintendo")
                            || lower.contains("switch")
                            || lower.contains("pro controller")
                        {
                            log::warn!(
                                "SDL lists '{name}' (id={}) but is_gamepad=false — controller input is disabled. \
                                 Re-pair the pad, update the game/SDL, or set SDL_GAMECONTROLLERCONFIG / SDL hints per SDL docs.",
                                id.0
                            );
                        }
                    }
                    self.non_gamepad_logged.insert(id);
                }
                continue;
            }
            match self.gamepad.open(id) {
                Ok(gp) => {
                    let name = gp.name().unwrap_or_else(|| "(unknown)".into());
                    let rumble = unsafe { gp.has_rumble() };
                    log::debug!(
                        "SDL gamepad opened: id={} name={name:?} SDL_PROP_GAMEPAD_CAP_RUMBLE={rumble}",
                        id.0
                    );
                    if !rumble {
                        log::warn!(
                            "This gamepad reports no rumble to SDL — force feedback will not run (driver/SDL limits; see SDL joystick hints)."
                        );
                    }
                    self.lt_prev.insert(id, 0.0);
                    self.rt_prev.insert(id, 0.0);
                    self.pads.insert(id, gp);
                }
                Err(e) => {
                    log::warn!("SDL_OpenGamepad failed for id {}: {e}", id.0);
                }
            }
        }
    }

    pub fn drawable_size(&self) -> (u32, u32) {
        self.window.size_in_pixels()
    }

    /// Map SDL window-coordinate mouse position to drawable pixels (HiDPI).
    pub fn event_xy_to_pixels(&self, x: f32, y: f32) -> (f32, f32) {
        let s = self.window.display_scale();
        (x * s, y * s)
    }

    pub fn desktop_fullscreen_on(&self) -> bool {
        matches!(
            self.window.fullscreen_state(),
            FullscreenType::Desktop | FullscreenType::True
        )
    }

    pub fn set_desktop_fullscreen(&mut self, on: bool) -> anyhow::Result<()> {
        self.window
            .set_fullscreen(on)
            .map_err(anyhow::Error::from)?;
        Ok(())
    }
}
