//! Single SDL3 context: window, event pump, gamepads (wgpu stays separate).

use std::collections::{HashMap, HashSet};

use sdl3::gamepad::Gamepad;
use sdl3::joystick::JoystickId;
use sdl3::video::{FullscreenType, Window, WindowBuilder, WindowFlags};
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
    /// `borderless_fullscreen`: after creating a windowed surface, enter borderless fullscreen
    /// (ignored when `SteamTenfoot` is set — that path always starts fullscreen).
    pub fn new(
        title: &str,
        width: u32,
        height: u32,
        borderless_fullscreen: bool,
    ) -> anyhow::Result<Self> {
        let _sdl = sdl3::init().map_err(anyhow::Error::from)?;
        #[cfg(target_os = "macos")]
        sdl3::hint::set("SDL_VIDEO_MACOSX_METAL_LAYER", "1");

        // Steam Input arbitration:
        //
        // When the player has a controller bound through Steam Input, Steam
        // signals SDL to suppress the raw HID device so the game doesn't see
        // both Steam's virtual gamepad and the underlying physical pad
        // (double-input bug from the Steam Controller dev docs:
        // <https://partner.steamgames.com/doc/features/steam_controller/getting_started_for_devs>).
        // SDL3 enables this arbitration automatically when started inside the
        // Steam runtime, but we set the hints explicitly to lock the
        // behaviour against future SDL default changes.
        sdl3::hint::set("SDL_JOYSTICK_HIDAPI", "1");
        sdl3::hint::set("SDL_JOYSTICK_HIDAPI_STEAM", "1");
        sdl3::hint::set("SDL_JOYSTICK_HIDAPI_STEAMDECK", "1");

        let _video = _sdl.video().map_err(anyhow::Error::from)?;
        let gamepad = _sdl.gamepad().map_err(anyhow::Error::from)?;
        let joystick = _sdl.joystick().map_err(anyhow::Error::from)?;

        let tenfoot = std::env::var_os("SteamTenfoot").is_some();
        let (win_w, win_h) = if tenfoot {
            (width, height)
        } else {
            Self::clamp_launch_window_size(&_video, width, height)
        };
        let mut wb = WindowBuilder::new(&_video, title, win_w, win_h);
        wb.resizable().high_pixel_density();
        if tenfoot {
            wb.fullscreen();
        } else {
            wb.position_centered();
        }

        #[cfg(target_os = "macos")]
        {
            use sdl3::video::WindowFlags;
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
            use sdl3::video::WindowFlags;
            wb.set_flags(WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY);
            // Do NOT set `SDL_WINDOW_VULKAN` here: wgpu creates its Vulkan surface from the raw
            // HWND (`Instance::create_surface_unsafe` → `vkCreateWin32SurfaceKHR`), so the flag is
            // not required for the Vulkan WSI path. Setting it changes the Win32 window class SDL
            // registers, which drops mouse events through the default `WindowProc`.
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

        if !tenfoot && borderless_fullscreen {
            shell.set_desktop_fullscreen(true)?;
        }

        Ok(shell)
    }

    fn clamp_launch_window_size(video: &VideoSubsystem, w: u32, h: u32) -> (u32, u32) {
        let Ok(display) = video.get_primary_display() else {
            return (w, h);
        };
        let Ok(bounds) = display.get_usable_bounds() else {
            return (w, h);
        };
        let max_w = bounds.width().max(1);
        let max_h = bounds.height().max(1);
        (w.min(max_w), h.min(max_h))
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

    /// True when this window should simulate and render (focused, visible, not minimized).
    ///
    /// When false, the SDL main loop skips per-frame simulation and rendering so the game does
    /// not keep running while backgrounded.
    pub fn window_is_foreground(&self) -> bool {
        let flags = WindowFlags::from(self.window.window_flags());
        self.window.has_input_focus()
            && !flags.intersects(WindowFlags::MINIMIZED | WindowFlags::HIDDEN)
    }

    /// Map SDL window-coordinate mouse position to drawable pixels (HiDPI).
    ///
    /// Use `SDL_GetWindowPixelDensity` (= `pixel_size / window_size`), not
    /// `SDL_GetWindowDisplayScale`. The latter folds in the user's display content scale (the
    /// Windows "Scale & layout" 125/150 % setting), so at 150 % a click at the right edge of the
    /// window lands ~1.5× past the right edge of the backbuffer and misses every UI rect.
    pub fn event_xy_to_pixels(&self, x: f32, y: f32) -> (f32, f32) {
        let s = self.window.pixel_density();
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
